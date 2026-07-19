//! Etchy Tauri application layer.
//!
//! Responsibilities:
//! - expose `list_devices`, `inspect_iso`, `start_flash`, `cancel_flash` to the UI
//! - device enumeration runs unprivileged (read-only sysfs/diskutil/CIM)
//! - flashing spawns the bundled `etchy-helper` binary **elevated**
//!   (pkexec on Linux, UAC via runas-style launch on Windows, osascript on macOS)
//!   and relays its JSON progress stream to the UI as Tauri events.

mod elevate;

use serde::Serialize;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// Handle to a running helper process so we can cancel it.
#[derive(Default)]
struct FlashJob(Mutex<Option<Child>>);

#[derive(Serialize)]
struct IsoInfo {
    path: String,
    name: String,
    size: u64,
}

#[tauri::command]
fn list_devices() -> Result<Vec<etchy_core::Device>, String> {
    etchy_core::list_devices().map_err(|e| e.to_string())
}

#[tauri::command]
fn inspect_iso(path: String) -> Result<IsoInfo, String> {
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    Ok(IsoInfo { path, name, size: meta.len() })
}

/// Start a flash job. Progress arrives at the UI as `flash-progress`,
/// completion as `flash-done`, failure as `flash-error` events.
#[tauri::command]
fn start_flash(
    app: AppHandle,
    job: State<'_, FlashJob>,
    options: etchy_core::FlashOptions,
) -> Result<(), String> {
    let mut guard = job.0.lock().unwrap();
    if guard.is_some() {
        return Err("a flash job is already running".into());
    }

    // Write options to a temp file: avoids quoting issues across elevation shims.
    let opts_json = serde_json::to_string(&options).map_err(|e| e.to_string())?;
    let opts_path = std::env::temp_dir().join(format!("etchy-opts-{}.json", std::process::id()));
    std::fs::write(&opts_path, &opts_json).map_err(|e| e.to_string())?;

    let helper = elevate::helper_path(&app).map_err(|e| e.to_string())?;
    let mut cmd: Command = elevate::elevated_command(&helper, &[
        "flash".to_string(),
        "--options-file".to_string(),
        opts_path.to_string_lossy().into_owned(),
    ])
    .map_err(|e| e.to_string())?;

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start elevated helper: {e}"))?;

    let stdout = child.stdout.take().ok_or("no stdout from helper")?;
    *guard = Some(child);
    drop(guard);

    // Relay the helper's JSON lines to the frontend as events.
    let app2 = app.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut saw_terminal_event = false;
        for line in reader.lines().map_while(Result::ok) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            match v["event"].as_str() {
                Some("progress") => {
                    let _ = app2.emit("flash-progress", &v);
                }
                Some("done") => {
                    saw_terminal_event = true;
                    let _ = app2.emit("flash-done", &v["report"]);
                }
                Some("error") => {
                    saw_terminal_event = true;
                    let _ = app2.emit("flash-error", v["message"].as_str().unwrap_or("unknown error"));
                }
                _ => {}
            }
        }
        // Helper exited without a terminal event (crash / auth dialog dismissed).
        if !saw_terminal_event {
            let _ = app2.emit("flash-error", "helper exited unexpectedly (was the authorization dialog cancelled?)");
        }
        let job = app2.state::<FlashJob>();
        let mut guard = job.0.lock().unwrap();
        if let Some(mut c) = guard.take() {
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&opts_path);
    });

    Ok(())
}

#[tauri::command]
fn cancel_flash(job: State<'_, FlashJob>) -> Result<(), String> {
    let mut guard = job.0.lock().unwrap();
    if let Some(child) = guard.as_mut() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = writeln!(stdin, "cancel");
            return Ok(());
        }
        // No stdin (shouldn't happen) — kill as a last resort.
        let _ = child.kill();
        return Ok(());
    }
    Err("no flash job running".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(FlashJob::default())
        .invoke_handler(tauri::generate_handler![
            list_devices,
            inspect_iso,
            start_flash,
            cancel_flash
        ])
        .run(tauri::generate_context!())
        .expect("error while running Etchy");
}
