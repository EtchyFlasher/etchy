//! Windows implementation: PowerShell/CIM enumeration + \\.\PhysicalDriveN I/O.
//!
//! Writing a raw physical drive on Windows requires:
//! 1. `FSCTL_LOCK_VOLUME` + `FSCTL_DISMOUNT_VOLUME` on every volume of the disk
//! 2. `IOCTL_DISK_DELETE_DRIVE_LAYOUT` is NOT needed if volumes are dismounted
//! 3. Writes to `\\.\PhysicalDriveN` must be sector-aligned (we pad to 4096)

use crate::types::{Device, EtchyError};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::process::Command;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME};

/// Raw device handle plus the locked volume handles that must stay open
/// (and locked) for the duration of the write.
pub struct RawDevice {
    file: File,
    _locked_volumes: Vec<File>,
}

impl Read for RawDevice {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}
impl Write for RawDevice {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
impl Seek for RawDevice {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

/// Enumerate removable USB disks via CIM (PowerShell).
///
/// Filters: BusType == 'USB', not the system/boot disk, size > 0.
pub fn enumerate_devices() -> Result<Vec<Device>, EtchyError> {
    let ps = r#"
        Get-Disk | Where-Object { $_.BusType -eq 'USB' -and -not $_.IsSystem -and -not $_.IsBoot } |
        ForEach-Object {
            $letters = (Get-Partition -DiskNumber $_.Number -ErrorAction SilentlyContinue |
                        Where-Object DriveLetter | ForEach-Object { "$($_.DriveLetter):" }) -join ','
            [PSCustomObject]@{ n=$_.Number; m=$_.FriendlyName; s=$_.Size; l=$letters }
        } | ConvertTo-Json -Compress
    "#;
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps])
        .output()
        .map_err(|e| EtchyError::Other(format!("powershell failed: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    // Normalize single-object output into an array.
    let json = if stdout.starts_with('[') { stdout.to_string() } else { format!("[{stdout}]") };
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json)
        .map_err(|e| EtchyError::Other(format!("bad disk json: {e}")))?;

    let mut devices = Vec::new();
    for d in parsed {
        let n = d["n"].as_u64().unwrap_or(u64::MAX);
        let size = d["s"].as_u64().unwrap_or(0);
        if n == u64::MAX || size == 0 {
            continue;
        }
        let letters: Vec<String> = d["l"]
            .as_str()
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        devices.push(Device {
            path: format!(r"\\.\PhysicalDrive{n}"),
            model: d["m"].as_str().unwrap_or("USB Drive").trim().to_string(),
            size,
            bus: "usb".into(),
            removable: true,
            mountpoints: letters,
        });
    }
    Ok(devices)
}

/// On Windows "unmounting" happens at open time via volume lock+dismount;
/// nothing to do here.
pub fn unmount(_device: &Device, _mountpoint: &str) -> Result<(), EtchyError> {
    Ok(())
}

fn lock_volumes(device: &Device) -> Result<Vec<File>, EtchyError> {
    let mut locked = Vec::new();
    for letter in &device.mountpoints {
        let vol_path = format!(r"\\.\{letter}"); // e.g. \\.\E:
        let vol = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&vol_path)?;
        let h = vol.as_raw_handle() as HANDLE;
        let mut ret: u32 = 0;
        unsafe {
            if DeviceIoControl(h, FSCTL_LOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut ret, std::ptr::null_mut()) == 0 {
                return Err(EtchyError::Unmount(letter.clone(), "failed to lock volume (files in use?)".into()));
            }
            if DeviceIoControl(h, FSCTL_DISMOUNT_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut ret, std::ptr::null_mut()) == 0 {
                return Err(EtchyError::Unmount(letter.clone(), "failed to dismount volume".into()));
            }
        }
        locked.push(vol);
    }
    Ok(locked)
}

pub fn open_device_rw(device: &Device) -> Result<RawDevice, EtchyError> {
    let locked = lock_volumes(device)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_WRITE_THROUGH)
        .open(&device.path)?;
    Ok(RawDevice { file, _locked_volumes: locked })
}

pub fn open_device_ro_nocache(device: &Device) -> Result<RawDevice, EtchyError> {
    // NOTE: FILE_FLAG_NO_BUFFERING requires sector-aligned buffer *addresses*;
    // Vec<u8> from the global allocator is 16-byte aligned at best, so we rely
    // on the fresh handle + dismounted volumes meaning no stale cache instead.
    let _ = FILE_FLAG_NO_BUFFERING; // documented-why-not marker
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&device.path)?;
    Ok(RawDevice { file, _locked_volumes: Vec::new() })
}

pub fn sync_device(dev: &mut RawDevice) -> Result<(), EtchyError> {
    dev.file.sync_all()?;
    Ok(())
}
