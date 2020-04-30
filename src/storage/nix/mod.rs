#![cfg(unix)]
use std::fs::File;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::io::SeekFrom;
use std::ffi::CString;
use crate::storage::*;
use ::nix::*;
use std::os::unix::io::*;

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
mod os;

enum FileType {
    File, Block, Raw, Other
}

fn resolve_file_type(mode: libc::mode_t) -> FileType {
    match mode & libc::S_IFMT {
        libc::S_IFREG => FileType::File,
        libc::S_IFBLK => FileType::Block,
        libc::S_IFCHR => FileType::Raw,
        _ => FileType::Other
    }
}

fn resolve_storage_size(file_type: &FileType, stat: &libc::stat, fd: RawFd) -> u64 {
    match file_type {
        FileType::Block | FileType::Raw => os::get_block_device_size(fd),
        _ => stat.st_size as u64
    }
}

#[derive(Debug)]
pub struct FileAccess {
    file: File
}

impl FileAccess {
    pub fn new<P: AsRef<Path>>(file_path: P) -> IoResult<FileAccess> {
        let file = os::open_file_direct(file_path, true)?;
        Ok(FileAccess { file })
    }
}

impl StorageAccess for FileAccess {

    fn position(&mut self) -> IoResult<u64> {
        self.file.seek(SeekFrom::Current(0))
    }

    fn seek(&mut self, position: u64) -> IoResult<u64> {
        self.file.seek(SeekFrom::Start(position))
    }

    fn read(&mut self, buffer: &mut [u8]) -> IoResult<usize> {
        self.file.read(buffer)
    }

    fn write(&mut self, data: &[u8]) -> IoResult<()> {
        self.file.write_all(data)
    }

    fn flush(&self) -> IoResult<()> {
        self.file.sync_all()
    }
}

#[derive(Debug)]
pub struct FileRef {
    path: PathBuf,
    details: StorageDetails
}

impl FileRef {
    pub fn new<P: AsRef<Path>>(path: P) -> IoResult<FileRef> {
        let p = path.as_ref().to_path_buf();
        let details = Self::build_details(path)?;
        Ok(FileRef { path: p, details })
    }

    fn build_details<P: AsRef<Path>>(path: P) -> IoResult<StorageDetails> {
        unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            let cpath = CString::new(path.as_ref().to_str().unwrap())?;
            if libc::stat(cpath.as_ptr(), &mut stat) >= 0 {

                let file_type = resolve_file_type(stat.st_mode);

                let f = os::open_file_direct(path, false)?;
                let fd = f.as_raw_fd();

                let size = resolve_storage_size(&file_type, &stat, fd);
                let storage_type = StorageType::Unknown; //TODO: this
                let media_type = MediaType::Unknown; //TODO: this
                let serial = None; //TODO: this
                let mount_point = None; //TODO: this

                Ok(StorageDetails{
                    size,
                    block_size: stat.st_blksize as usize,
                    storage_type,
                    media_type,
                    is_trim_supported: os::is_trim_supported(fd),
                    serial,
                    mount_point
                })
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "Unable to get stat info"))
            }
        }
    }
}

impl StorageRef for FileRef {
    type Access = FileAccess;

    fn id(&self) -> &str {
        self.path.to_str().unwrap()
    }

    fn details(&self) -> &StorageDetails {
        &self.details
    }

    fn access(&self) -> IoResult<Box<Self::Access>> {
        FileAccess::new(&self.path).map(Box::new)
    }
}

impl System {
    pub fn get_storage_devices() -> IoResult<Vec<impl StorageRef>> {
        os::get_storage_devices()
    }
}
