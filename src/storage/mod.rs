#[cfg(unix)]
use self::nix::*;
#[cfg(unix)]
mod nix;

#[cfg(windows)]
use windows::*;
#[cfg(windows)]
mod windows;

pub type IoResult<A> = std::io::Result<A>;

pub trait StorageAccess {
    fn position(&mut self) -> IoResult<u64>;
    fn seek(&mut self, position: u64) -> IoResult<u64>;
    fn read(&mut self, buffer: &mut [u8]) -> IoResult<usize>;
    fn write(&mut self, data: &[u8]) -> IoResult<()>;
    fn flush(&self) -> IoResult<()>;
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum StorageType {
    Unknown,
    File,
    Partition,
    Drive,
    RAID,
    Other
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum MediaType {
    Unknown,
    Rotational,
    SolidState,
    Other
}

#[derive(Debug, Clone)]
pub struct StorageDetails {
    pub size: u64,
    pub block_size: usize,
    pub storage_type: StorageType,
    pub media_type: MediaType,
    pub is_trim_supported: bool,
    pub serial: Option<String>,
    pub mount_point: Option<String>
}

pub trait StorageRef {
    type Access: StorageAccess;
    fn id(&self) -> &str;
    fn details(&self) -> &StorageDetails;
    fn access(&self) -> IoResult<Box<Self::Access>>;
}

pub struct System;
