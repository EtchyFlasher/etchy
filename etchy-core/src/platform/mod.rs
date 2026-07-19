//! Platform abstraction: device enumeration, unmounting, raw device I/O.
//!
//! This module is the single safety gatekeeper. `enumerate_devices` must
//! only ever return removable, non-system devices, and every other function
//! only accepts `Device` values produced by it.

use crate::types::{Device, EtchyError};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

/// A handle to a raw block device supporting Read/Write/Seek.
pub type RawDevice = imp::RawDevice;

/// List removable, non-system block devices. The ONLY device source.
pub fn enumerate_devices() -> Result<Vec<Device>, EtchyError> {
    imp::enumerate_devices()
}

/// Unmount a mounted filesystem living on the target device.
pub fn unmount(device: &Device, mountpoint: &str) -> Result<(), EtchyError> {
    imp::unmount(device, mountpoint)
}

/// Open the raw device read-write with an exclusive lock.
pub fn open_device_rw(device: &Device) -> Result<RawDevice, EtchyError> {
    imp::open_device_rw(device)
}

/// Open the raw device read-only, bypassing OS caches, for verification.
pub fn open_device_ro_nocache(device: &Device) -> Result<RawDevice, EtchyError> {
    imp::open_device_ro_nocache(device)
}

/// Force all written data onto the physical medium.
pub fn sync_device(dev: &mut RawDevice) -> Result<(), EtchyError> {
    imp::sync_device(dev)
}
