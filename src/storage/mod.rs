#[cfg(unix)]
mod nix;

#[cfg(windows)]
mod windows;

use std::collections::VecDeque;

use anyhow::{Result, anyhow};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("bad block")]
    BadBlock,
    #[error("other i/o error")]
    Other(#[from] std::io::Error),
}

#[derive(Debug)]
pub enum StorageReadiness {
    Ready(StorageDetails),
    /// A device that cannot be accessed, e.g. because it is held by another process.
    /// Only Windows reports this state currently.
    #[allow(dead_code)] // not constructed on unix platforms
    Locked,
}

#[derive(Debug)]
pub struct StorageRef {
    pub id: String,
    pub readiness: StorageReadiness,
    pub children: Vec<StorageRef>,
}

pub trait StorageDevice {
    fn access(&self) -> Result<Box<dyn StorageAccess>>;

    /// Opens the device for asynchronous, positional I/O.
    ///
    /// The default implementation falls back to synchronous requests with a depth of 1.
    fn access_async(&self, _depth: usize) -> Result<Box<dyn AsyncStorageAccess>> {
        Ok(Box::new(SyncAsyncAccess::new(self.access()?)))
    }
}

/// Random-access I/O over a storage device.
pub trait StorageAccess {
    fn seek(&mut self, position: u64) -> Result<u64>;
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;

    fn hardware_erase(&mut self, _method: HardwareEraseMethod) -> Result<HardwareEraseOutcome> {
        Err(anyhow!("Hardware erase is not supported for this device."))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoKind {
    Read,
    Write,
}

/// A completed asynchronous request.
pub struct IoCompletion {
    pub slot: usize,
    pub result: Result<usize>,
}

/// Positional I/O with multiple requests in flight.
///
/// Requests are submitted against reusable slots. The caller must keep the buffer
/// passed to [`AsyncStorageAccess::submit`] alive and untouched until the matching
/// completion has been reaped.
pub trait AsyncStorageAccess {
    /// Maximum number of requests that can be in flight.
    fn depth(&self) -> usize;

    fn submit(&mut self, slot: usize, position: u64, buffer: &mut [u8], kind: IoKind)
    -> Result<()>;

    /// Blocks until at least one request completes.
    fn reap(&mut self) -> Result<IoCompletion>;

    /// Waits for every in-flight request and releases its slot.
    fn drain(&mut self) -> Result<()>;

    fn flush(&mut self) -> Result<()>;

    fn hardware_erase(&mut self, _method: HardwareEraseMethod) -> Result<HardwareEraseOutcome> {
        Err(anyhow!("Hardware erase is not supported for this device."))
    }
}

/// Adapts a synchronous [`StorageAccess`] to the asynchronous interface with a depth
/// of 1. Used on platforms without a native asynchronous disk I/O API.
struct SyncAsyncAccess {
    access: Box<dyn StorageAccess>,
    completions: VecDeque<IoCompletion>,
}

impl SyncAsyncAccess {
    fn new(access: Box<dyn StorageAccess>) -> Self {
        SyncAsyncAccess {
            access,
            completions: VecDeque::new(),
        }
    }
}

impl AsyncStorageAccess for SyncAsyncAccess {
    fn depth(&self) -> usize {
        1
    }

    fn submit(
        &mut self,
        slot: usize,
        position: u64,
        buffer: &mut [u8],
        kind: IoKind,
    ) -> Result<()> {
        let result = self.access.seek(position).and_then(|_| match kind {
            IoKind::Read => self.access.read(buffer),
            IoKind::Write => self.access.write(buffer).map(|()| buffer.len()),
        });

        self.completions.push_back(IoCompletion { slot, result });
        Ok(())
    }

    fn reap(&mut self) -> Result<IoCompletion> {
        self.completions
            .pop_front()
            .ok_or_else(|| anyhow!("No pending I/O completion"))
    }

    fn drain(&mut self) -> Result<()> {
        self.completions.clear();
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.access.flush()
    }
}

#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
#[expect(clippy::upper_case_acronyms)] // `CD`, `RAID` are storage industry terms
pub enum StorageType {
    #[default]
    Unknown,
    File,
    Partition,
    Fixed,
    Removable,
    CD,
    Network,
    RAID,
    Other,
}

impl std::fmt::Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Broad physical characteristics of the storage, used to pick sensible I/O defaults.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DriveKind {
    #[default]
    Unknown,
    Hdd,
    Ssd,
    NvmeSsd,
    Removable,
}

/// The transport the device is attached through, when the platform can tell.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(dead_code)] // some kinds are platform-specific
pub enum BusKind {
    #[default]
    Unknown,
    Usb,
    Nvme,
    Sata,
    Scsi,
    Pcie,
    Virtual,
    Other,
}

impl std::fmt::Display for BusKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BusKind::Unknown => "Unknown",
            BusKind::Usb => "USB",
            BusKind::Nvme => "NVMe",
            BusKind::Sata => "SATA",
            BusKind::Scsi => "SCSI",
            BusKind::Pcie => "PCIe",
            BusKind::Virtual => "Virtual",
            BusKind::Other => "Other",
        })
    }
}

/// A hardware erase command selected for execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HardwareEraseMethod {
    Off,
    Auto,
    AtaSanitize,
    NvmeSanitize,
}

impl std::str::FromStr for HardwareEraseMethod {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "auto" => Ok(Self::Auto),
            "ata-sanitize" => Ok(Self::AtaSanitize),
            "nvme-sanitize" => Ok(Self::NvmeSanitize),
            _ => Err("expected one of: off, auto, ata-sanitize, nvme-sanitize".to_string()),
        }
    }
}

impl std::fmt::Display for HardwareEraseMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::AtaSanitize => "ATA SANITIZE",
            Self::NvmeSanitize => "NVMe Sanitize",
        })
    }
}

/// Result of a hardware erase request.
#[derive(Clone, Debug)]
pub struct HardwareEraseOutcome {
    /// Whether the device confirmed that the operation completed.
    pub confirmed: bool,
    /// Optional explanation, especially when completion could not be confirmed.
    pub message: Option<String>,
}

impl HardwareEraseOutcome {
    pub fn confirmed() -> Self {
        HardwareEraseOutcome {
            confirmed: true,
            message: None,
        }
    }

    pub fn unconfirmed(message: impl Into<String>) -> Self {
        HardwareEraseOutcome {
            confirmed: false,
            message: Some(message.into()),
        }
    }
}

/// Hardware erase methods reported by the device itself.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // an independent capability flag per method
pub struct HardwareEraseCapabilities {
    pub ata_secure_erase: bool,
    pub ata_enhanced_secure_erase: bool,
    pub ata_sanitize_crypto: bool,
    pub ata_sanitize_block_erase: bool,
    pub ata_sanitize_overwrite: bool,
    pub nvme_sanitize_crypto: bool,
    pub nvme_sanitize_block_erase: bool,
    pub nvme_sanitize_overwrite: bool,
    pub nvme_format: bool,
}

impl HardwareEraseCapabilities {
    pub fn ata_sanitize(self) -> bool {
        self.ata_sanitize_crypto || self.ata_sanitize_block_erase || self.ata_sanitize_overwrite
    }

    pub fn nvme_sanitize(self) -> bool {
        self.nvme_sanitize_crypto || self.nvme_sanitize_block_erase || self.nvme_sanitize_overwrite
    }
}

impl HardwareEraseCapabilities {
    /// Short list for the device table. Each method is marked with `*` because the
    /// abbreviations are explained below the table.
    pub fn abbreviations(self) -> String {
        let mut methods = Vec::new();

        // Preferred methods first: sanitize covers the whole device and has better
        // reporting than the older format/secure-erase paths.
        if self.nvme_sanitize() {
            methods.push("NS*");
        }
        if self.nvme_format {
            methods.push("NF*");
        }
        if self.ata_sanitize() {
            methods.push("SA*");
        }
        if self.ata_secure_erase {
            methods.push("SE*");
        }

        if methods.is_empty() {
            "-".to_string()
        } else {
            methods.join(" ")
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StorageDetails {
    pub size: u64,
    /// Logical block size of the device, as reported by the platform. Reserved for
    /// aligning direct I/O transfers; not consumed by the wipe logic yet.
    #[allow(dead_code)]
    pub block_size: usize,
    pub storage_type: StorageType,
    pub mount_point: Option<String>,
    pub label: Option<String>,
    pub drive_kind: DriveKind,
    pub bus: BusKind,
    /// `None` means the platform could not probe the device.
    pub hardware_erase: Option<HardwareEraseCapabilities>,
}

impl std::fmt::Display for DriveKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DriveKind::Unknown => "Unknown",
            DriveKind::Hdd => "HDD",
            DriveKind::Ssd => "SSD",
            DriveKind::NvmeSsd => "NVMe SSD",
            DriveKind::Removable => "Removable",
        })
    }
}

impl StorageDetails {
    /// A conservative estimate of how many requests are worth keeping in flight.
    ///
    /// The values are deliberately small: raw-device I/O is mostly limited by the
    /// device itself, and deeper queues do not help every SSD. Users can override the
    /// result with `--queue-depth`.
    pub fn recommended_queue_depth(&self) -> usize {
        match self.drive_kind {
            DriveKind::NvmeSsd => 8,
            DriveKind::Ssd => 4,
            DriveKind::Hdd | DriveKind::Removable | DriveKind::Unknown => 1,
        }
    }
}

/// Entry point for platform-specific storage device discovery.
pub struct System;

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_hardware_erase_abbreviations_are_in_preferred_order() {
        let all = HardwareEraseCapabilities {
            ata_secure_erase: true,
            ata_enhanced_secure_erase: true,
            ata_sanitize_crypto: true,
            ata_sanitize_block_erase: true,
            ata_sanitize_overwrite: true,
            nvme_sanitize_crypto: true,
            nvme_sanitize_block_erase: true,
            nvme_sanitize_overwrite: true,
            nvme_format: true,
        };

        assert_eq!(all.abbreviations(), "NS* NF* SA* SE*");
        assert_eq!(HardwareEraseCapabilities::default().abbreviations(), "-");
    }

    #[test]
    fn test_recommended_queue_depth() {
        let depth = |drive_kind| {
            StorageDetails {
                drive_kind,
                ..StorageDetails::default()
            }
            .recommended_queue_depth()
        };

        assert_eq!(depth(DriveKind::Hdd), 1);
        assert_eq!(depth(DriveKind::Ssd), 4);
        assert_eq!(depth(DriveKind::NvmeSsd), 8);
        assert_eq!(depth(DriveKind::Removable), 1);
        assert_eq!(depth(DriveKind::Unknown), 1);
    }
}
