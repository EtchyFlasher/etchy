//! The flash pipeline: validate → (badblocks) → write → sync → verify → (persistence).
//!
//! Verification is **mandatory**: after writing, the exact number of written
//! bytes is read back from the raw device and its SHA-256 must equal the
//! source ISO's SHA-256. There is no way to skip it.

use crate::badblocks;
use crate::hash::{hex, sha256_file, SpeedMeter, CHUNK};
use crate::persistence;
use crate::platform;
use crate::types::*;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Run a complete flash job. Blocking; call from a worker thread.
///
/// `on_progress` is invoked frequently with phase + byte-level progress so
/// the UI can render rings, speed graphs and ETAs.
pub fn flash(
    opts: &FlashOptions,
    cancel: &CancelFlag,
    mut on_progress: impl FnMut(Progress),
) -> Result<FlashReport, EtchyError> {
    let started = Instant::now();

    // ---- Preflight -------------------------------------------------------
    // The device must be one we would enumerate: removable, non-system.
    let device = platform::enumerate_devices()?
        .into_iter()
        .find(|d| d.path == opts.device_path)
        .ok_or_else(|| EtchyError::SystemDeviceRefused(opts.device_path.clone()))?;

    let iso_len = std::fs::metadata(&opts.iso_path)?.len();
    if iso_len > device.size {
        return Err(EtchyError::IsoTooLarge { iso: iso_len, device: device.size });
    }

    // ---- Phase 1: hash the source ISO ------------------------------------
    let source_sha256 =
        sha256_file(&opts.iso_path, Phase::HashSource, cancel, &mut |p| on_progress(p))?;

    if let Some(expected) = &opts.expected_sha256 {
        let expected = expected.trim().to_lowercase();
        if expected != source_sha256 {
            return Err(EtchyError::ChecksumMismatch {
                expected,
                actual: source_sha256,
            });
        }
    }

    // ---- Unmount all mounted filesystems on the target -------------------
    for mp in &device.mountpoints {
        platform::unmount(&device, mp)?;
    }

    // ---- Phase 2 (optional): destructive bad-block / fake-capacity check --
    if opts.badblock_check {
        badblocks::check(&device, cancel, &mut |p| on_progress(p))?;
    }

    // ---- Open the raw device (exclusive, locked) --------------------------
    let mut dev = platform::open_device_rw(&device)?;

    // ---- Phase 3: write ----------------------------------------------------
    let mut iso = File::open(&opts.iso_path)?;
    let mut buf = vec![0u8; CHUNK];
    let mut written: u64 = 0;
    let mut meter = SpeedMeter::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(EtchyError::Cancelled);
        }
        let n = iso.read(&mut buf)?;
        if n == 0 {
            break;
        }
        // Raw device writes on some platforms require sector-aligned sizes;
        // pad the final chunk with zeros up to the 4096-byte boundary.
        let n_aligned = align_up(n, 4096);
        if n_aligned > n {
            buf[n..n_aligned].fill(0);
        }
        dev.write_all(&buf[..n_aligned])?;
        written += n as u64;
        let (speed_bps, eta_secs) = meter.sample(written, iso_len);
        on_progress(Progress {
            phase: Phase::Write,
            bytes_done: written,
            bytes_total: iso_len,
            speed_bps,
            eta_secs,
        });
    }

    // ---- Phase 4: sync -----------------------------------------------------
    on_progress(Progress { phase: Phase::Sync, bytes_done: 0, bytes_total: 1, speed_bps: 0, eta_secs: 0 });
    dev.flush()?;
    platform::sync_device(&mut dev)?;
    drop(dev);
    on_progress(Progress { phase: Phase::Sync, bytes_done: 1, bytes_total: 1, speed_bps: 0, eta_secs: 0 });

    // ---- Phase 5: verify (mandatory) ---------------------------------------
    // Re-open read-only with caches dropped so we read the *physical* data.
    let mut dev_r = platform::open_device_ro_nocache(&device)?;
    let mut hasher = Sha256::new();
    let mut verified_bytes: u64 = 0;
    let mut meter = SpeedMeter::new();

    while verified_bytes < iso_len {
        if cancel.load(Ordering::Relaxed) {
            return Err(EtchyError::Cancelled);
        }
        let want = std::cmp::min(CHUNK as u64, align_up((iso_len - verified_bytes) as usize, 4096) as u64) as usize;
        let n = dev_r.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        // Only hash up to iso_len bytes (ignore alignment padding).
        let useful = std::cmp::min(n as u64, iso_len - verified_bytes) as usize;
        hasher.update(&buf[..useful]);
        verified_bytes += useful as u64;
        let (speed_bps, eta_secs) = meter.sample(verified_bytes, iso_len);
        on_progress(Progress {
            phase: Phase::Verify,
            bytes_done: verified_bytes,
            bytes_total: iso_len,
            speed_bps,
            eta_secs,
        });
    }

    let device_sha256 = hex(&hasher.finalize());
    if device_sha256 != source_sha256 || verified_bytes != iso_len {
        return Err(EtchyError::VerifyFailed { source_hash: source_sha256, device_hash: device_sha256 });
    }
    drop(dev_r);

    // ---- Phase 6 (optional): persistence partition -------------------------
    if let Some(p) = &opts.persistence {
        on_progress(Progress { phase: Phase::Persistence, bytes_done: 0, bytes_total: 1, speed_bps: 0, eta_secs: 0 });
        persistence::add_persistence(&device, iso_len, p)?;
        on_progress(Progress { phase: Phase::Persistence, bytes_done: 1, bytes_total: 1, speed_bps: 0, eta_secs: 0 });
    }

    let elapsed = started.elapsed().as_secs_f64();
    on_progress(Progress { phase: Phase::Done, bytes_done: iso_len, bytes_total: iso_len, speed_bps: 0, eta_secs: 0 });

    Ok(FlashReport {
        source_sha256,
        device_sha256,
        verified: true,
        bytes_written: written,
        elapsed_secs: elapsed,
        avg_write_bps: if elapsed > 0.0 { (written as f64 / elapsed) as u64 } else { 0 },
    })
}

#[inline]
pub fn align_up(n: usize, align: usize) -> usize {
    n.div_ceil(align) * align
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_works() {
        assert_eq!(align_up(1, 4096), 4096);
        assert_eq!(align_up(4096, 4096), 4096);
        assert_eq!(align_up(4097, 4096), 8192);
    }
}
