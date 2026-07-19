//! etchy-helper — the privileged worker process.
//!
//! The GUI never runs as root/admin. Instead it launches this small binary
//! elevated (pkexec / UAC / authopen) and talks to it over pipes:
//!
//! - stdout: one JSON object per line (`{"event": ...}`)
//! - stdin:  a single line `cancel\n` aborts the job
//!
//! Usage:
//!   etchy-helper list
//!   etchy-helper flash --options-json '<FlashOptions JSON>'
//!   etchy-helper flash --options-file /path/to/options.json

use etchy_core::{flash::flash, list_devices, new_cancel_flag, FlashOptions};
use std::io::{BufRead, Write};
use std::sync::atomic::Ordering;

fn emit(value: serde_json::Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");

    match cmd {
        "list" => match list_devices() {
            Ok(devices) => emit(serde_json::json!({ "event": "devices", "devices": devices })),
            Err(e) => {
                emit(serde_json::json!({ "event": "error", "message": e.to_string() }));
                std::process::exit(1);
            }
        },
        "flash" => {
            let opts = parse_flash_options(&args).unwrap_or_else(|msg| {
                emit(serde_json::json!({ "event": "error", "message": msg }));
                std::process::exit(2);
            });
            run_flash(opts);
        }
        "version" => {
            emit(serde_json::json!({ "event": "version", "version": env!("CARGO_PKG_VERSION") }));
        }
        _ => {
            eprintln!("usage: etchy-helper <list|flash|version>");
            std::process::exit(2);
        }
    }
}

fn parse_flash_options(args: &[String]) -> Result<FlashOptions, String> {
    let mut json: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--options-json" => {
                json = args.get(i + 1).cloned();
                i += 2;
            }
            "--options-file" => {
                let path = args.get(i + 1).ok_or("--options-file needs a path")?;
                json = Some(std::fs::read_to_string(path).map_err(|e| e.to_string())?);
                i += 2;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let json = json.ok_or("missing --options-json or --options-file")?;
    serde_json::from_str(&json).map_err(|e| format!("bad FlashOptions JSON: {e}"))
}

fn run_flash(opts: FlashOptions) {
    let cancel = new_cancel_flag();

    // Watch stdin for a cancel request.
    {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                if line.trim() == "cancel" {
                    cancel.store(true, Ordering::Relaxed);
                    break;
                }
            }
        });
    }

    // Throttle progress events to ~20/sec to keep the pipe light.
    let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let result = flash(&opts, &cancel, |p| {
        let now = std::time::Instant::now();
        let force = p.bytes_done == p.bytes_total; // always emit phase completions
        if force || now.duration_since(last_emit).as_millis() >= 50 {
            last_emit = now;
            emit(serde_json::json!({
                "event": "progress",
                "phase": p.phase,
                "bytes_done": p.bytes_done,
                "bytes_total": p.bytes_total,
                "speed_bps": p.speed_bps,
                "eta_secs": p.eta_secs,
                "percent": p.percent(),
            }));
        }
    });

    match result {
        Ok(report) => {
            emit(serde_json::json!({ "event": "done", "report": report }));
        }
        Err(e) => {
            emit(serde_json::json!({ "event": "error", "message": e.to_string() }));
            std::process::exit(1);
        }
    }
}
