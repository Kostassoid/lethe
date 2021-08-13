#![cfg(unix)]

use crate::storage::*;
use ::nix::*;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::prelude::*;
use std::io::SeekFrom;
use std::path::Path;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
use linux as os;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
use macos as os;

impl StorageError {
    fn from(err: std::io::Error) -> StorageError {
        match err.raw_os_error() {
            Some(c) if c == libc::EIO || c == libc::ESPIPE => StorageError::BadBlock,
            _ => StorageError::Other(err),
        }
    }
}

#[derive(Debug)]
pub struct FileAccess {
    file: File,
}

impl FileAccess {
    pub fn new<P: AsRef<Path>>(file_path: P) -> Result<FileAccess> {
        let file = os::open_file_direct(file_path, true)?;
        Ok(FileAccess { file })
    }
}

impl StorageAccess for FileAccess {
    fn position(&mut self) -> Result<u64> {
        self.file
            .seek(SeekFrom::Current(0))
            .map_err(|e| StorageError::from(e))
            .context("Seek failed or not supported for the storage")
    }

    fn seek(&mut self, position: u64) -> Result<u64> {
        self.file
            .seek(SeekFrom::Start(position))
            .map_err(|e| StorageError::from(e))
            .context("Seek failed or not supported for the storage")
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.file
            .read(buffer)
            .map_err(|e| StorageError::from(e))
            .context("Can't read from the storage")
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.file
            .write_all(data)
            .map_err(|e| StorageError::from(e))
            .context("Writing to storage failed")
    }

    fn flush(&mut self) -> Result<()> {
        self.file
            .flush()
            .map_err(|e| StorageError::from(e))
            .context("Unable to flush data to the storage")
    }
}

impl StorageDevice for StorageRef {
    fn access(&self) -> Result<Box<dyn StorageAccess>> {
        self.children
            .iter()
            .flat_map(|c| &c.details.mount_point)
            .for_each(|c| {
                let _ = os::unmount(c.as_str());
            });

        match &self.details.mount_point {
            Some(c) => os::unmount(c.as_str())?,
            _ => (),
        };

        FileAccess::new(&self.id).map(|a| Box::new(a) as Box<dyn StorageAccess>)
    }
}
