//! Cross-platform privilege elevation for the helper binary.
//!
//! The GUI stays unprivileged; only the tiny `etchy-helper` process is
//! elevated, and only for the duration of one flash job.
//!
//! - **Linux**:   `pkexec etchy-helper ...` (PolicyKit auth dialog)
//! - **macOS**:   `osascript -e 'do shell script ... with administrator privileges'`
//! - **Windows**: PowerShell `Start-Process -Verb RunAs` (UAC prompt) with the
//!                helper's stdio redirected through a named-pipe relay is
//!                complex, so instead we ship the helper with a UAC manifest
//!                (`requireAdministrator`) and just spawn it — Windows shows
//!                the UAC prompt automatically.

use std::path::PathBuf;
use std::process::Command;
use tauri::{AppHandle, Manager};

/// Locate the bundled etchy-helper next to the app binary (Tauri sidecar layout).
pub fn helper_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    let exe_name = if cfg!(windows) { "etchy-helper.exe" } else { "etchy-helper" };

    // 1. Next to our own executable (bundled sidecar).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(exe_name);
            if p.exists() {
                return Ok(p);
            }
        }
    }
    // 2. Tauri resource dir.
    if let Ok(res) = app.path().resource_dir() {
        let p = res.join(exe_name);
        if p.exists() {
            return Ok(p);
        }
    }
    // 3. Development: target/{debug,release} of the workspace.
    for profile in ["debug", "release"] {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target")
            .join(profile)
            .join(exe_name);
        if p.exists() {
            return Ok(p);
        }
    }
    anyhow::bail!("etchy-helper binary not found — is the app bundled correctly?")
}

/// Build an elevated Command for the helper with the given arguments.
#[cfg(target_os = "linux")]
pub fn elevated_command(helper: &std::path::Path, args: &[String]) -> anyhow::Result<Command> {
    // pkexec pops the PolicyKit auth dialog and runs the helper as root.
    let mut cmd = Command::new("pkexec");
    cmd.arg(helper);
    cmd.args(args);
    Ok(cmd)
}

#[cfg(target_os = "macos")]
pub fn elevated_command(helper: &std::path::Path, args: &[String]) -> anyhow::Result<Command> {
    // `do shell script ... with administrator privileges` shows the native
    // macOS auth dialog. Quote everything defensively.
    fn sh_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
    let joined = std::iter::once(helper.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|a| sh_quote(&a))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "do shell script {} with administrator privileges",
        applescript_quote(&joined)
    );
    let mut cmd = Command::new("osascript");
    cmd.args(["-e", &script]);
    Ok(cmd)
}

#[cfg(target_os = "macos")]
fn applescript_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(windows)]
pub fn elevated_command(helper: &std::path::Path, args: &[String]) -> anyhow::Result<Command> {
    // The helper .exe carries a requireAdministrator manifest (see
    // etchy-helper build docs), so spawning it directly triggers UAC.
    let mut cmd = Command::new(helper);
    cmd.args(args);
    Ok(cmd)
}
