//! # etchy-core
//!
//! The heart of Etchy: a safe, verified ISO → USB flashing engine.
//!
//! Design goals:
//! - **Never touch a system disk.** Enumeration only returns removable,
//!   non-system devices; the platform layer is the single gatekeeper.
//! - **Always verify.** Every flash is followed by a full read-back
//!   compare against the source hash. This is not optional.
//! - **Rich progress.** Every long operation reports fine-grained progress
//!   (bytes, speed, ETA, phase) through a callback, so the GUI can render
//!   live feedback.

pub mod badblocks;
pub mod flash;
pub mod hash;
pub mod persistence;
pub mod platform;
pub mod types;

pub use types::*;

/// Enumerate removable USB block devices that are safe to flash.
///
/// System / internal disks are filtered out by the platform layer and are
/// never returned. This is the *only* supported way to discover targets.
pub fn list_devices() -> Result<Vec<Device>, EtchyError> {
    platform::enumerate_devices()
}
