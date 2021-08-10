#[cfg(unix)]
mod nix;
#[cfg(unix)]
pub use crate::storage::nix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

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
pub struct StorageRef {
    pub id: String,
    pub details: StorageDetails,
    pub children: Vec<StorageRef>,
}

pub trait StorageDevice {
    fn access(&self) -> Result<Box<dyn StorageAccess>>;
}

pub trait StorageAccess {
    fn position(&mut self) -> Result<u64>;
    fn seek(&mut self, position: u64) -> Result<u64>;
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum StorageType {
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
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub struct StorageDetails {
    pub size: u64,
    pub block_size: usize,
    pub storage_type: StorageType,
    pub mount_point: Option<String>,
    pub label: Option<String>,
}

impl Default for StorageDetails {
    fn default() -> Self {
        StorageDetails {
            size: 0,
            block_size: 0,
            storage_type: StorageType::Unknown,
            mount_point: None,
            label: None,
        }
    }
}

pub struct System {}
