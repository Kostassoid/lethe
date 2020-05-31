#![cfg(unix)]
use crate::storage::*;
use ::nix::*;
use anyhow::{Context, Result};
use std::ffi::CString;
use std::fs::File;
use std::io::prelude::*;
use std::io::SeekFrom;
use std::os::unix::io::*;
use std::path::{Path, PathBuf};

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
mod os;

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
            .context("Seek failed or not supported for the storage")
    }

    fn seek(&mut self, position: u64) -> Result<u64> {
        self.file
            .seek(SeekFrom::Start(position))
            .context("Seek failed or not supported for the storage")
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.file
            .read(buffer)
            .context("Can't read from the storage")
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.file
            .write_all(data)
            .context("Writing to storage failed")
    }

    fn flush(&self) -> Result<()> {
        self.file
            .sync_all()
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
        unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            let cpath = CString::new(path.as_ref().to_str().unwrap())?;
            if libc::stat(cpath.as_ptr(), &mut stat) >= 0 {
                let file_type = resolve_file_type(stat.st_mode);

                let f = os::open_file_direct(&path, false)?;
                let fd = f.as_raw_fd();

                let size = resolve_storage_size(&file_type, &stat, fd);
                let storage_type = os::resolve_storage_type(&path).unwrap_or(StorageType::Unknown);
                let mount_point = os::resolve_mount_point(&path).unwrap_or(None);

                Ok(StorageDetails {
                    size,
                    block_size: stat.st_blksize as usize,
                    storage_type,
                    mount_point,
                })
            } else {
                Err(anyhow!("Unable to get stat info"))
            }
        }
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
    pub fn get_storage_devices() -> Result<Vec<impl StorageRef>> {
        os::get_storage_devices()
    }

    pub fn access(storage_ref: &dyn StorageRef) -> Result<impl StorageAccess> {
        FileAccess::new(&storage_ref.id())
    }
}
