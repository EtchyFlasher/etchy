//! Destructive bad-block & fake-capacity check.
//!
//! Two problems this catches before you waste 20 minutes flashing:
//! 1. **Dying flash cells** — writes a pseudo-random pattern and reads it back.
//! 2. **Counterfeit drives** — "128 GB" sticks that are really 8 GB wrap
//!    writes around; because our pattern is seeded by the *absolute offset*,
//!    wrapped data never matches and the check fails fast.
//!
//! Strategy: full pattern write+readback over the device in stripes
//! (default samples ~64 stripes across the whole device rather than every
//! byte, so a 128 GB stick checks in ~a minute on USB 3 instead of hours),
//! plus always the first and last 64 MiB — the areas most likely to be
//! fake or worn.

use crate::hash::SpeedMeter;
use crate::platform;
use crate::types::*;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;

const STRIPE: u64 = 16 * 1024 * 1024; // 16 MiB per stripe
const STRIPES: u64 = 64; // sample count across the device
const EDGE: u64 = 64 * 1024 * 1024; // always check first/last 64 MiB

/// xorshift64* PRNG seeded by absolute device offset — position-dependent
/// so wrap-around (fake capacity) is detected.
fn pattern_fill(buf: &mut [u8], offset: u64) {
    let mut x = offset ^ 0x9E37_79B9_7F4A_7C15;
    for chunk in buf.chunks_mut(8) {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes();
        let n = chunk.len();
        chunk.copy_from_slice(&v[..n]);
    }
}

fn stripe_offsets(size: u64) -> Vec<u64> {
    let mut offs = Vec::new();
    // Edges.
    let mut o = 0;
    while o < EDGE.min(size) {
        offs.push(o);
        o += STRIPE;
    }
    if size > EDGE {
        let tail_start = size.saturating_sub(EDGE) / STRIPE * STRIPE;
        let mut o = tail_start;
        while o < size {
            offs.push(o);
            o += STRIPE;
        }
    }
    // Even samples across the middle.
    if size > 2 * EDGE {
        let step = (size / STRIPES).max(STRIPE);
        let mut o = EDGE;
        while o + STRIPE <= size - EDGE {
            offs.push(o / STRIPE * STRIPE);
            o += step;
        }
    }
    offs.sort_unstable();
    offs.dedup();
    offs
}

/// Run the destructive check. The device content is garbage afterwards,
/// which is fine — we flash right after.
pub fn check(
    device: &Device,
    cancel: &CancelFlag,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), EtchyError> {
    let offsets = stripe_offsets(device.size);
    let total: u64 = offsets.len() as u64 * STRIPE * 2; // write + read
    let mut done: u64 = 0;
    let mut meter = SpeedMeter::new();

    let mut dev = platform::open_device_rw(device)?;
    let mut wbuf = vec![0u8; STRIPE as usize];
    let mut rbuf = vec![0u8; STRIPE as usize];

    // Pass 1: write patterns.
    for &off in &offsets {
        if cancel.load(Ordering::Relaxed) {
            return Err(EtchyError::Cancelled);
        }
        let len = STRIPE.min(device.size - off) as usize;
        let len = len / 4096 * 4096;
        if len == 0 { continue; }
        pattern_fill(&mut wbuf[..len], off);
        dev.seek(SeekFrom::Start(off))?;
        dev.write_all(&wbuf[..len]).map_err(|e| EtchyError::BadBlocks {
            offset: off,
            reason: format!("write error: {e}"),
        })?;
        done += len as u64;
        let (speed_bps, eta_secs) = meter.sample(done, total);
        on_progress(Progress { phase: Phase::BadBlockCheck, bytes_done: done, bytes_total: total, speed_bps, eta_secs });
    }
    dev.flush()?;
    platform::sync_device(&mut dev)?;
    drop(dev);

    // Pass 2: read back with caches dropped and compare.
    let mut dev = platform::open_device_ro_nocache(device)?;
    for &off in &offsets {
        if cancel.load(Ordering::Relaxed) {
            return Err(EtchyError::Cancelled);
        }
        let len = STRIPE.min(device.size - off) as usize;
        let len = len / 4096 * 4096;
        if len == 0 { continue; }
        pattern_fill(&mut wbuf[..len], off);
        dev.seek(SeekFrom::Start(off))?;
        dev.read_exact(&mut rbuf[..len]).map_err(|e| EtchyError::BadBlocks {
            offset: off,
            reason: format!("read error: {e}"),
        })?;
        if rbuf[..len] != wbuf[..len] {
            let bad = wbuf[..len].iter().zip(&rbuf[..len]).position(|(a, b)| a != b).unwrap_or(0);
            return Err(EtchyError::BadBlocks {
                offset: off + bad as u64,
                reason: "read-back mismatch — drive has bad blocks or fake capacity".into(),
            });
        }
        done += len as u64;
        let (speed_bps, eta_secs) = meter.sample(done, total);
        on_progress(Progress { phase: Phase::BadBlockCheck, bytes_done: done, bytes_total: total, speed_bps, eta_secs });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_position_dependent() {
        let mut a = vec![0u8; 4096];
        let mut b = vec![0u8; 4096];
        pattern_fill(&mut a, 0);
        pattern_fill(&mut b, 4096);
        assert_ne!(a, b, "different offsets must produce different patterns");
        let mut a2 = vec![0u8; 4096];
        pattern_fill(&mut a2, 0);
        assert_eq!(a, a2, "same offset must be deterministic");
    }

    #[test]
    fn stripes_cover_edges() {
        let offs = stripe_offsets(8 * 1024 * 1024 * 1024); // 8 GiB
        assert_eq!(offs[0], 0);
        assert!(*offs.last().unwrap() >= 8 * 1024 * 1024 * 1024 - EDGE);
    }
}
