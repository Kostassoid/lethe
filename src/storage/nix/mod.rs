#![cfg(unix)]

use anyhow::{Context, Result};
use std::fs::File;
use std::io::SeekFrom;
use std::io::prelude::*;
use std::path::Path;

use crate::storage::{StorageAccess, StorageDevice, StorageError, StorageReadiness, StorageRef};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
use linux as os;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
use macos as os;

impl StorageError {
    /// Maps an I/O error to [`StorageError::BadBlock`] for the errno values that
    /// indicate an inaccessible region of the device, [`StorageError::Other`] otherwise.
    fn from_io(err: std::io::Error) -> StorageError {
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
    fn seek(&mut self, position: u64) -> Result<u64> {
        self.file
            .seek(SeekFrom::Start(position))
            .map_err(StorageError::from_io)
            .context("Seek failed or not supported for the storage")
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.file
            .read(buffer)
            .map_err(StorageError::from_io)
            .context("Can't read from the storage")
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.file
            .write_all(data)
            .map_err(StorageError::from_io)
            .context("Writing to storage failed")
    }

    fn flush(&mut self) -> Result<()> {
        self.file
            .flush()
            .map_err(StorageError::from_io)
            .context("Unable to flush data to the storage")
    }
}

impl StorageDevice for StorageRef {
    fn access(&self) -> Result<Box<dyn StorageAccess>> {
        for child in &self.children {
            if let StorageReadiness::Ready(details) = &child.readiness {
                if let Some(mount_point) = &details.mount_point {
                    let _ = os::unmount(mount_point.as_str());
                }
            }
        }

        if let StorageReadiness::Ready(details) = &self.readiness {
            if let Some(mount_point) = &details.mount_point {
                os::unmount(mount_point.as_str())?;
            }
        }

        FileAccess::new(&self.id).map(|a| Box::new(a) as Box<dyn StorageAccess>)
    }
}
