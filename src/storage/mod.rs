#[cfg(unix)]
mod nix;

#[cfg(windows)]
mod windows;

use anyhow::Result;
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

    /// Opens `count` independent handles to the device, for parallel I/O.
    ///
    /// Platforms with exclusive device locking override this to lock once and then open
    /// the remaining handles without re-locking.
    fn access_many(&self, count: usize) -> Result<Vec<Box<dyn StorageAccess>>> {
        (0..count).map(|_| self.access()).collect()
    }
}

/// Random-access I/O over a storage device.
pub trait StorageAccess: Send {
    fn seek(&mut self, position: u64) -> Result<u64>;
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
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
}

/// Entry point for platform-specific storage device discovery.
pub struct System;
