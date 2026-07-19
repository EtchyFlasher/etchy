//! Linux implementation: sysfs enumeration + O_DIRECT/O_EXCL raw I/O.

use crate::types::{Device, EtchyError};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Raw device wrapper. Reads/writes go straight through; `O_EXCL` on a block
/// device makes the kernel refuse while any partition is mounted — a free
/// extra safety net.
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

fn read_sys(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Enumerate removable USB block devices via /sys/block.
///
/// Filters (ALL must pass):
/// - `removable == 1`
/// - bus is USB (resolved via the device symlink) — extra belt & braces
/// - not the device hosting `/` (checked via mountinfo)
/// - size > 0
pub fn enumerate_devices() -> Result<Vec<Device>, EtchyError> {
    let mut out = Vec::new();
    let root_dev = root_backing_device();

    for entry in std::fs::read_dir("/sys/block")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip obvious non-disk nodes.
        if name.starts_with("loop")
            || name.starts_with("ram")
            || name.starts_with("dm-")
            || name.starts_with("zram")
            || name.starts_with("md")
            || name.starts_with("sr")
        {
            continue;
        }
        let sys = entry.path();

        let removable = read_sys(&sys.join("removable")).as_deref() == Some("1");
        if !removable {
            continue;
        }

        // Resolve bus: the sysfs device path contains "/usb" for USB devices.
        let real = std::fs::canonicalize(sys.join("device")).unwrap_or_default();
        let real_s = real.to_string_lossy();
        let bus = if real_s.contains("/usb") { "usb" } else { "other" };
        if bus != "usb" {
            continue; // USB only: memory-card readers etc. still show as usb bridges
        }

        // Never the disk backing /.
        if let Some(rd) = &root_dev {
            if &name == rd {
                continue;
            }
        }

        let sectors: u64 = read_sys(&sys.join("size")).and_then(|s| s.parse().ok()).unwrap_or(0);
        let size = sectors * 512;
        if size == 0 {
            continue;
        }

        let vendor = read_sys(&sys.join("device/vendor")).unwrap_or_default();
        let model = read_sys(&sys.join("device/model")).unwrap_or_default();
        let model = format!("{} {}", vendor, model).trim().to_string();

        out.push(Device {
            path: format!("/dev/{name}"),
            model: if model.is_empty() { name.clone() } else { model },
            size,
            bus: bus.to_string(),
            removable,
            mountpoints: mountpoints_for(&name),
        });
    }
    Ok(out)
}

/// Which /sys/block name backs the root filesystem, if resolvable.
fn root_backing_device() -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    let dev = mounts
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            let src = it.next()?;
            let mp = it.next()?;
            (mp == "/").then(|| src.to_string())
        })?;
    let dev = std::fs::canonicalize(&dev).ok()?;
    let name = dev.file_name()?.to_string_lossy().into_owned();
    // Strip partition suffix: sda3 -> sda, nvme0n1p2 -> nvme0n1, mmcblk0p1 -> mmcblk0
    let base = if let Some(idx) = name.rfind('p').filter(|_| name.contains("nvme") || name.contains("mmcblk")) {
        name[..idx].to_string()
    } else {
        name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string()
    };
    Some(base)
}

fn mountpoints_for(disk: &str) -> Vec<String> {
    let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };
    mounts
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let src = it.next()?;
            let mp = it.next()?;
            src.starts_with(&format!("/dev/{disk}")).then(|| mp.replace("\\040", " "))
        })
        .collect()
}

pub fn unmount(device: &Device, mountpoint: &str) -> Result<(), EtchyError> {
    let c_mp = std::ffi::CString::new(mountpoint)
        .map_err(|_| EtchyError::Unmount(mountpoint.into(), "bad path".into()))?;
    // Try a normal umount2, then lazy detach as fallback.
    let r = unsafe { libc::umount2(c_mp.as_ptr(), 0) };
    if r == 0 {
        return Ok(());
    }
    let r = unsafe { libc::umount2(c_mp.as_ptr(), libc::MNT_DETACH) };
    if r == 0 {
        return Ok(());
    }
    Err(EtchyError::Unmount(
        format!("{} ({})", mountpoint, device.path),
        std::io::Error::last_os_error().to_string(),
    ))
}

pub fn open_device_rw(device: &Device) -> Result<RawDevice, EtchyError> {
    // O_EXCL on a block device: kernel refuses if any partition is mounted.
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_EXCL)
        .open(&device.path)?;
    Ok(RawDevice(f))
}

pub fn open_device_ro_nocache(device: &Device) -> Result<RawDevice, EtchyError> {
    // Note: O_DIRECT would need sector-aligned *memory* buffers, so instead
    // we open normally and drop the page cache for this file first — reads
    // then come from the physical device. Same approach as other flashers.
    let f = File::open(&device.path)?;
    unsafe {
        libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
    Ok(RawDevice(f))
}

pub fn sync_device(dev: &mut RawDevice) -> Result<(), EtchyError> {
    dev.0.sync_all()?;
    let r = unsafe { libc::ioctl(dev.0.as_raw_fd(), 0x1261 /* BLKFLSBUF */) };
    let _ = r; // best-effort cache flush
    Ok(())
}
