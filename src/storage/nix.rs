extern crate nix;

use std::fs::{File, OpenOptions};
use std::fs::read_dir;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::io::SeekFrom;
use self::super::*;
use std::ffi::CString;
use nix::*;

// FileAccess

pub struct FileAccess {
    file: File
}

impl FileAccess {
    pub fn new<P: AsRef<Path>>(file_path: P) -> IoResult<FileAccess> {
        let file = File::open(file_path.as_ref())?;
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

    fn read(&mut self, buffer: &mut [u8]) -> IoResult<u64> {
        self.file.read(buffer).map(|x| x as u64)
    }

    fn write(&mut self, data: &[u8]) -> IoResult<()> {
        self.file.write_all(data)
    }

    fn flush(&self) -> IoResult<()> {
        self.file.sync_all()
    }
}

// FileRef

pub struct FileRef {
    path: PathBuf,
    details: StorageDetails
}

impl FileRef {
    pub fn new<P: AsRef<Path>>(path: P) -> FileRef {
        let p = path.as_ref().to_path_buf();
        let details = Self::build_details(path).unwrap();
        FileRef { path: p, details }
    }

    fn resolve_storage_type(mode: u16) -> StorageType {
        match mode & libc::S_IFMT {
            libc::S_IFREG => StorageType::File,
            libc::S_IFBLK => StorageType::Block,
            libc::S_IFCHR => StorageType::Raw,
            _ => StorageType::Other
        }
    }

    #[cfg(target_os = "macos")]
    fn get_block_device_size(fd: libc::c_int) -> u64 {
        ioctl_read!(dk_get_block_size, b'd', 24, u32); // DKIOCGETBLOCKSIZE
        ioctl_read!(dk_get_block_count, b'd', 25, u64); // DKIOCGETBLOCKCOUNT

        unsafe {
            let mut block_size: u32 = std::mem::zeroed();
            let mut block_count: u64 = std::mem::zeroed();
            dk_get_block_size(fd, &mut block_size).unwrap();
            dk_get_block_count(fd, &mut block_count).unwrap();
            (block_size as u64) * block_count
        }
    }

    #[cfg(target_os = "linux")]
    fn get_block_device_size(fd: libc::c_int) -> u64 {
        // requires linux 2.4.10+
        ioctl_read!(linux_get_block_size, 0x12, 114, u64); // BLKGETSIZE64

        unsafe {
            let mut block_size: u64 = std::mem::zeroed();
            linux_get_block_size(fd, &mut block_size).unwrap();
            block_size
        }
    }

    fn resolve_storage_size(storage_type: &StorageType, stat: &libc::stat, path: &PathBuf) -> u64 {
        match storage_type {
            StorageType::File => stat.st_size as u64,
            _ => {
                use std::os::unix::io::*;
                let f = OpenOptions::new().read(true).create(false).write(false).open(path).unwrap();
                let fd = f.as_raw_fd();
                Self::get_block_device_size(fd)
            }
        }
    }

    fn build_details<P: AsRef<Path>>(path: P) -> IoResult<StorageDetails> {
        unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            let cpath = CString::new(path.as_ref().to_str().unwrap())?;
            if libc::stat(cpath.as_ptr(), &mut stat) >= 0 {

                let storage_type = Self::resolve_storage_type(stat.st_mode);

                let size = Self::resolve_storage_size(&storage_type, &stat, &path.as_ref().to_path_buf());

                Ok(StorageDetails{
                    size,
                    block_size: stat.st_blksize as u64,
                    storage_type
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

// FileEnumerator

pub struct FileEnumerator {
    root: PathBuf,
    path_filter: fn(&PathBuf) -> bool,
    meta_filter: fn(&StorageDetails) -> bool
}

impl FileEnumerator {
    pub fn custom<P: AsRef<Path>>(
        root: P,
        path_filter: fn(&PathBuf) -> bool,
        meta_filter: fn(&StorageDetails) -> bool
    ) -> FileEnumerator {
        let p = root.as_ref().to_path_buf();
        FileEnumerator { root: p, path_filter, meta_filter }
    }

    #[cfg(target_os = "macos")]
    pub fn system_drives() -> FileEnumerator {
        FileEnumerator::custom(
            "/dev",
            |p| p.to_str().unwrap().contains("disk0s4"),
            |_m| true
        )
    }
}

impl<'a> StorageEnumerator<'a> for FileEnumerator {
    type Ref = FileRef;

    fn iterate(&self) -> IoResult<Box<Iterator<Item=Self::Ref> + 'a>> {
        let rd = read_dir(&self.root)?;
        Ok(Box::new(rd.filter_map(std::io::Result::ok)
            .map(|de| de.path())
            .filter(|path|
                (self.path_filter)(&path.to_path_buf())
            )
            .map(FileRef::new)
            .filter(|r|
                (self.meta_filter)(&r.details)
            )
            .collect::<Vec<_>>().into_iter() // todo: is there a better way to avoid lifetime conflicts?
        ))
    }
}