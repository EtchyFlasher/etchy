//! macOS implementation: `diskutil` enumeration + raw /dev/rdiskN I/O.
//!
//! We use the *character* device (`/dev/rdiskN`) for writing — it bypasses
//! the buffer cache and is dramatically faster than `/dev/diskN`. Raw
//! device I/O on macOS must be sector-aligned, which our 4096-padded
//! writes satisfy.

use crate::types::{Device, EtchyError};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::process::Command;

pub struct RawDevice(File);

impl Read for RawDevice {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl Write for RawDevice {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl Seek for RawDevice {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

/// Enumerate external, removable physical disks via `diskutil`.
///
/// Filters (ALL must pass, straight from `diskutil info -plist`):
/// - `Internal == false` and (`RemovableMedia` or `Removable` or `Ejectable`)
/// - whole disk only (no partition slices)
/// - protocol is USB
pub fn enumerate_devices() -> Result<Vec<Device>, EtchyError> {
    let out = Command::new("diskutil")
        .args(["list", "-plist", "physical"])
        .output()
        .map_err(|e| EtchyError::Other(format!("diskutil failed: {e}")))?;
    let list = String::from_utf8_lossy(&out.stdout);

    // Parse WholeDisks entries out of the plist without a plist dependency:
    // <key>WholeDisks</key><array><string>disk0</string>...</array>
    let mut disks = Vec::new();
    if let Some(idx) = list.find("<key>WholeDisks</key>") {
        if let Some(arr_end) = list[idx..].find("</array>") {
            let section = &list[idx..idx + arr_end];
            for part in section.split("<string>").skip(1) {
                if let Some(end) = part.find("</string>") {
                    disks.push(part[..end].to_string());
                }
            }
        }
    }

    let mut devices = Vec::new();
    for disk in disks {
        let info = Command::new("diskutil")
            .args(["info", "-plist", &disk])
            .output()
            .map_err(|e| EtchyError::Other(format!("diskutil info failed: {e}")))?;
        let info = String::from_utf8_lossy(&info.stdout);

        let get_bool = |key: &str| -> bool {
            info.find(&format!("<key>{key}</key>"))
                .map(|i| info[i..].trim_start_matches(&format!("<key>{key}</key>")).trim_start().starts_with("<true/>"))
                .unwrap_or(false)
        };
        let get_str = |key: &str| -> String {
            info.find(&format!("<key>{key}</key>"))
                .and_then(|i| {
                    let rest = &info[i..];
                    let s = rest.find("<string>")? + 8;
                    let e = rest.find("</string>")?;
                    (s < e).then(|| rest[s..e].to_string())
                })
                .unwrap_or_default()
        };
        let get_int = |key: &str| -> u64 {
            info.find(&format!("<key>{key}</key>"))
                .and_then(|i| {
                    let rest = &info[i..];
                    let s = rest.find("<integer>")? + 9;
                    let e = rest.find("</integer>")?;
                    rest[s..e].parse().ok()
                })
                .unwrap_or(0)
        };

        let internal = get_bool("Internal");
        let removable = get_bool("RemovableMedia") || get_bool("Removable") || get_bool("Ejectable");
        let protocol = get_str("BusProtocol");
        if internal || !removable || !protocol.eq_ignore_ascii_case("usb") {
            continue;
        }
        let size = get_int("TotalSize").max(get_int("Size"));
        if size == 0 {
            continue;
        }

        devices.push(Device {
            path: format!("/dev/r{disk}"), // raw character device
            model: get_str("MediaName"),
            size,
            bus: "usb".into(),
            removable: true,
            mountpoints: vec![format!("/dev/{disk}")], // token: unmountDisk target
        });
    }
    Ok(devices)
}

/// On macOS we unmount the *whole disk* once via diskutil.
pub fn unmount(_device: &Device, mountpoint: &str) -> Result<(), EtchyError> {
    let target = mountpoint.trim_start_matches("/dev/");
    let out = Command::new("diskutil")
        .args(["unmountDisk", "force", target])
        .output()
        .map_err(|e| EtchyError::Unmount(target.into(), e.to_string()))?;
    if !out.status.success() {
        return Err(EtchyError::Unmount(
            target.into(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    Ok(())
}

pub fn open_device_rw(device: &Device) -> Result<RawDevice, EtchyError> {
    let f = OpenOptions::new().read(true).write(true).open(&device.path)?;
    Ok(RawDevice(f))
}

pub fn open_device_ro_nocache(device: &Device) -> Result<RawDevice, EtchyError> {
    // /dev/rdiskN is already uncached; also set F_NOCACHE for belt & braces.
    let f = File::open(&device.path)?;
    unsafe {
        libc::fcntl(f.as_raw_fd(), libc::F_NOCACHE, 1);
    }
    Ok(RawDevice(f))
}

pub fn sync_device(dev: &mut RawDevice) -> Result<(), EtchyError> {
    // F_FULLFSYNC forces the drive to flush its own cache too.
    let r = unsafe { libc::fcntl(dev.0.as_raw_fd(), libc::F_FULLFSYNC) };
    if r != 0 {
        dev.0.sync_all()?;
    }
    Ok(())
}
