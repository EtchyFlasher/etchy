use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A removable block device eligible for flashing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// OS path of the raw device, e.g. `/dev/sdb`, `\\.\PhysicalDrive2`, `/dev/rdisk4`.
    pub path: String,
    /// Human-readable vendor/model, e.g. "SanDisk Ultra USB 3.0".
    pub model: String,
    /// Total size in bytes.
    pub size: u64,
    /// Bus type as reported by the OS ("usb", "sd", ...).
    pub bus: String,
    /// True if the OS marks the device removable. Non-removable devices
    /// are never returned by enumeration; kept for display/debugging.
    pub removable: bool,
    /// Mounted filesystem paths (will be unmounted before flashing).
    pub mountpoints: Vec<String>,
}

/// Phases of a flash job, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Hashing the source ISO.
    HashSource,
    /// Optional destructive bad-block / fake-capacity test.
    BadBlockCheck,
    /// Writing the image to the device.
    Write,
    /// Flushing OS caches to the physical device.
    Sync,
    /// Reading back from the device and comparing hashes.
    Verify,
    /// Creating the optional persistence partition.
    Persistence,
    /// All done.
    Done,
}

/// A progress snapshot delivered to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub phase: Phase,
    /// Bytes processed within the current phase.
    pub bytes_done: u64,
    /// Total bytes for the current phase.
    pub bytes_total: u64,
    /// Instantaneous throughput in bytes/sec (smoothed).
    pub speed_bps: u64,
    /// Estimated seconds remaining in the current phase.
    pub eta_secs: u64,
}

impl Progress {
    pub fn percent(&self) -> f64 {
        if self.bytes_total == 0 {
            return 0.0;
        }
        (self.bytes_done as f64 / self.bytes_total as f64) * 100.0
    }
}

/// Result of a completed flash job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashReport {
    pub source_sha256: String,
    pub device_sha256: String,
    pub verified: bool,
    pub bytes_written: u64,
    pub elapsed_secs: f64,
    pub avg_write_bps: u64,
}

/// Options for a flash job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashOptions {
    /// Absolute path to the source ISO image.
    pub iso_path: String,
    /// Raw device path (must come from `list_devices`).
    pub device_path: String,
    /// If provided, the source ISO's SHA-256 must equal this (hex,
    /// case-insensitive) or the job aborts before writing anything.
    pub expected_sha256: Option<String>,
    /// Run the destructive bad-block / fake-capacity pre-check first.
    pub badblock_check: bool,
    /// Add a persistence partition after flashing (Linux live USBs).
    pub persistence: Option<PersistenceOptions>,
}

/// Persistence partition configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceOptions {
    /// Size in bytes. Use 0 for "all remaining space".
    pub size: u64,
    /// Partition label: "casper-rw" (Ubuntu) or "persistence" (Debian).
    pub label: String,
}

#[derive(Debug, Error)]
pub enum EtchyError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("device not found or not eligible: {0}")]
    DeviceNotFound(String),

    #[error("refusing to touch non-removable/system device: {0}")]
    SystemDeviceRefused(String),

    #[error("ISO is larger than the target device ({iso} > {device} bytes)")]
    IsoTooLarge { iso: u64, device: u64 },

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("verification FAILED: device data does not match source (source {source_hash}, device {device_hash})")]
    VerifyFailed { source_hash: String, device_hash: String },

    #[error("bad-block check failed at offset {offset}: {reason}")]
    BadBlocks { offset: u64, reason: String },

    #[error("failed to unmount {0}: {1}")]
    Unmount(String, String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

/// Cancellation flag shared between UI and engine.
pub type CancelFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

pub fn new_cancel_flag() -> CancelFlag {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}
