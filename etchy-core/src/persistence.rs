//! Optional persistence partition for Linux live USBs.
//!
//! After a hybrid ISO is dd-written, the remaining space on the stick is
//! unpartitioned. We append an ext4 partition labelled `casper-rw`
//! (Ubuntu & derivatives) or `persistence` (Debian; also needs a
//! `persistence.conf` containing "/ union" inside the filesystem).
//!
//! Partition table editing is done by shelling out to the platform's
//! standard tools (sgdisk/sfdisk + mkfs.ext4 on Linux). On Windows/macOS
//! persistence is not offered — live-USB persistence is a Linux-boot
//! feature and the ext4 tooling isn't natively available there.

use crate::types::*;

#[cfg(target_os = "linux")]
pub fn add_persistence(
    device: &Device,
    image_len: u64,
    opts: &PersistenceOptions,
) -> Result<(), EtchyError> {
    use std::process::Command;

    // Start the partition 1 MiB after the image, aligned.
    let start_mib = (image_len / (1024 * 1024)) + 2;
    let size_arg = if opts.size == 0 {
        String::new() // to end of disk
    } else {
        format!("{}MiB", opts.size / (1024 * 1024))
    };

    // Re-read partition table first.
    let _ = Command::new("blockdev").args(["--rereadpt", &device.path]).status();

    // Append a partition using sfdisk (works for both MBR and GPT hybrids).
    let script = if size_arg.is_empty() {
        format!("{}MiB,,L\n", start_mib)
    } else {
        format!("{}MiB,{},L\n", start_mib, size_arg)
    };
    let out = Command::new("sfdisk")
        .args(["--append", "--no-reread", &device.path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.as_mut().unwrap().write_all(script.as_bytes())?;
            c.wait_with_output()
        })
        .map_err(|e| EtchyError::Other(format!("sfdisk failed to start: {e}")))?;
    if !out.status.success() {
        return Err(EtchyError::Other(format!(
            "sfdisk failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }

    let _ = Command::new("blockdev").args(["--rereadpt", &device.path]).status();
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Find the new (last) partition node.
    let part = last_partition_node(&device.path)?;

    // Format ext4 with the requested label.
    let out = Command::new("mkfs.ext4")
        .args(["-F", "-L", &opts.label, &part])
        .output()
        .map_err(|e| EtchyError::Other(format!("mkfs.ext4 failed to start: {e}")))?;
    if !out.status.success() {
        return Err(EtchyError::Other(format!(
            "mkfs.ext4 failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }

    // Debian-style persistence needs a persistence.conf inside.
    if opts.label == "persistence" {
        let mnt = std::env::temp_dir().join("etchy-persist-mnt");
        std::fs::create_dir_all(&mnt)?;
        let mnt_s = mnt.to_string_lossy().into_owned();
        let ok = Command::new("mount").args([&part, &mnt_s]).status();
        if matches!(ok, Ok(s) if s.success()) {
            std::fs::write(mnt.join("persistence.conf"), "/ union\n")?;
            let _ = Command::new("umount").arg(&mnt_s).status();
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn last_partition_node(device_path: &str) -> Result<String, EtchyError> {
    let base = device_path.trim_start_matches("/dev/");
    let sys = format!("/sys/block/{base}");
    let mut parts: Vec<String> = std::fs::read_dir(&sys)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(base))
        .collect();
    parts.sort();
    parts
        .last()
        .map(|p| format!("/dev/{p}"))
        .ok_or_else(|| EtchyError::Other("no partition found after sfdisk".into()))
}

#[cfg(not(target_os = "linux"))]
pub fn add_persistence(
    _device: &Device,
    _image_len: u64,
    _opts: &PersistenceOptions,
) -> Result<(), EtchyError> {
    Err(EtchyError::Other(
        "persistence partitions are only supported when flashing from Linux".into(),
    ))
}
