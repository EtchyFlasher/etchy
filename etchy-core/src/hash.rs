//! Streaming SHA-256 hashing with progress reporting.

use crate::types::{CancelFlag, EtchyError, Phase, Progress};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::sync::atomic::Ordering;
use std::time::Instant;

pub const CHUNK: usize = 4 * 1024 * 1024; // 4 MiB

/// Exponentially-smoothed throughput meter that also derives ETA.
pub struct SpeedMeter {
    last_t: Instant,
    last_bytes: u64,
    smoothed_bps: f64,
}

impl Default for SpeedMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeedMeter {
    pub fn new() -> Self {
        Self { last_t: Instant::now(), last_bytes: 0, smoothed_bps: 0.0 }
    }

    /// Feed cumulative byte count; returns (speed_bps, eta_secs for `remaining`).
    pub fn sample(&mut self, bytes_done: u64, bytes_total: u64) -> (u64, u64) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_t).as_secs_f64();
        if dt >= 0.25 {
            let db = bytes_done.saturating_sub(self.last_bytes) as f64;
            let inst = db / dt;
            self.smoothed_bps = if self.smoothed_bps == 0.0 {
                inst
            } else {
                0.7 * self.smoothed_bps + 0.3 * inst
            };
            self.last_t = now;
            self.last_bytes = bytes_done;
        }
        let remaining = bytes_total.saturating_sub(bytes_done) as f64;
        let eta = if self.smoothed_bps > 1.0 { remaining / self.smoothed_bps } else { 0.0 };
        (self.smoothed_bps as u64, eta as u64)
    }
}

/// Hash a file with SHA-256, streaming progress to `on_progress`.
pub fn sha256_file(
    path: &str,
    phase: Phase,
    cancel: &CancelFlag,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<String, EtchyError> {
    let mut file = File::open(path)?;
    let total = file.metadata()?.len();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut done: u64 = 0;
    let mut meter = SpeedMeter::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(EtchyError::Cancelled);
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        let (speed_bps, eta_secs) = meter.sample(done, total);
        on_progress(Progress { phase, bytes_done: done, bytes_total: total, speed_bps, eta_secs });
    }

    Ok(hex(&hasher.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::new_cancel_flag;
    use std::io::Write;

    #[test]
    fn hashes_known_content() {
        let f = tempfile_path();
        std::fs::File::create(&f.0).unwrap().write_all(b"etchy").unwrap();
        let cancel = new_cancel_flag();
        let h = sha256_file(&f.0, Phase::HashSource, &cancel, &mut |_| {}).unwrap();
        // sha256("etchy")
        assert_eq!(h, "753cc9a0c64aaa0128ef8cbf7dfd2340785f046ad43f00b46df555f107d8dac6");
        let _ = std::fs::remove_file(&f.0);
        drop(f);
    }

    fn tempfile_path() -> (String,) {
        let p = std::env::temp_dir().join(format!("etchy-test-{}", std::process::id()));
        (p.to_string_lossy().into_owned(),)
    }
}
