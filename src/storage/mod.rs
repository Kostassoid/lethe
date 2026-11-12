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
