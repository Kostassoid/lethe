#[cfg(unix)]
use self::nix::*;
#[cfg(unix)]
mod nix;

#[cfg(windows)]
mod windows;

use anyhow::Result;
use winapi::_core::fmt::Formatter;

pub trait StorageAccess {
    fn position(&mut self) -> Result<u64>;
    fn seek(&mut self, position: u64) -> Result<u64>;
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn flush(&self) -> Result<()>;
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum StorageType {
    Unknown,
    File,
    Partition,
    Drive,
    RAID,
    Other,
}

impl std::fmt::Display for StorageType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum MediaType {
    Unknown,
    Rotational,
    SolidState,
    Other,
}

#[derive(Debug, Clone)]
pub struct StorageDetails {
    pub size: u64,
    pub block_size: usize,
    pub storage_type: StorageType,
    pub media_type: MediaType,
    pub is_trim_supported: bool,
    pub serial: Option<String>,
    pub mount_point: Option<String>,
}

impl Default for StorageDetails {
    fn default() -> Self {
        StorageDetails {
            size: 0,
            block_size: 0,
            storage_type: StorageType::Unknown,
            media_type: MediaType::Unknown,
            is_trim_supported: false,
            serial: None,
            mount_point: None,
        }
    }
}

pub trait StorageRef {
    fn id(&self) -> &str;
    fn details(&self) -> &StorageDetails;
}

pub struct System {}
