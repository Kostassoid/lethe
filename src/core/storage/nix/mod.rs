#![cfg(unix)]
use super::*;
use ::nix::*;
use anyhow::{Context, Result};
use std::ffi::CString;
use std::fs::File;
use std::io::prelude::*;
use std::io::SeekFrom;
use std::os::unix::io::*;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;

enum FileType {
    File,
    Block,
    Raw,
    Other,
}

fn resolve_file_type(mode: libc::mode_t) -> FileType {
    match mode & libc::S_IFMT {
        libc::S_IFREG => FileType::File,
        libc::S_IFBLK => FileType::Block,
        libc::S_IFCHR => FileType::Raw,
        _ => FileType::Other,
    }
}

fn resolve_storage_size(file_type: &FileType, stat: &libc::stat, fd: RawFd) -> u64 {
    match file_type {
        FileType::Block | FileType::Raw => os::get_block_device_size(fd),
        _ => stat.st_size as u64,
    }
}

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

#[derive(Debug)]
pub struct FileRef {
    path: PathBuf,
    details: StorageDetails,
}

impl FileRef {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<FileRef> {
        let p = path.as_ref().to_path_buf();
        let details = Self::build_details(path)?;
        Ok(FileRef { path: p, details })
    }

    fn build_details<P: AsRef<Path>>(path: P) -> Result<StorageDetails> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let cpath = CString::new(path.as_ref().to_str().unwrap())?;
        unsafe {
            if libc::stat(cpath.as_ptr(), &mut stat) < 0 {
                Err(anyhow!("Unable to get stat info"))?;
            }
        }

        let file_type = resolve_file_type(stat.st_mode);

        let f = os::open_file_direct(&path, false)?;
        let fd = f.as_raw_fd();

        let size = resolve_storage_size(&file_type, &stat, fd);

        let mut details = StorageDetails {
            size,
            block_size: stat.st_blksize as usize,
            storage_type: StorageType::Unknown,
            mount_point: None,
        };

        os::enrich_storage_details(path, &mut details)?;

        Ok(details)
    }
}

impl StorageRef for FileRef {
    fn id(&self) -> &str {
        self.path.to_str().unwrap()
    }

    fn details(&self) -> &StorageDetails {
        &self.details
    }
}

impl System {
    pub fn access(storage_ref: &dyn StorageRef) -> Result<impl StorageAccess> {
        FileAccess::new(&storage_ref.id())
    }
}
