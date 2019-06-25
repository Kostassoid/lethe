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
    pub fn new<P: AsRef<Path>>(path: P) -> IoResult<FileRef> {
        let p = path.as_ref().to_path_buf();
        let details = Self::build_details(path)?;
        Ok(FileRef { path: p, details })
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

    #[cfg(target_os = "macos")]
    fn is_trim_supported(fd: libc::c_int) -> bool {
        ioctl_read!(dk_get_features, b'd', 76, u32); // DKIOCGETFEATURES

        unsafe {
            let mut features: u32 = std::mem::zeroed();
            dk_get_features(fd, &mut features)
            .map(|_| (features & 0x00000010) > 0) // DK_FEATURE_UNMAP
            .unwrap_or(false)
        }
    }

    #[cfg(target_os = "linux")]
    fn is_trim_supported(fd: libc::c_int) -> bool {
        false
    }

    fn resolve_storage_size(storage_type: &StorageType, stat: &libc::stat, fd: libc::c_int) -> u64 {
        match storage_type {
            StorageType::Block | StorageType::Raw => Self::get_block_device_size(fd),
            _ => stat.st_size as u64
        }
    }

    fn build_details<P: AsRef<Path>>(path: P) -> IoResult<StorageDetails> {
        unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            let cpath = CString::new(path.as_ref().to_str().unwrap())?;
            if libc::stat(cpath.as_ptr(), &mut stat) >= 0 {

                let storage_type = Self::resolve_storage_type(stat.st_mode);

                //println!("!!! {:?}: StorageType = {:?}", path.as_ref().to_str(), storage_type);

                use std::os::unix::io::*;
                let f = OpenOptions::new().read(true).create(false).write(false).open(path)?;
                let fd = f.as_raw_fd();

                let size = Self::resolve_storage_size(&storage_type, &stat, fd);

                Ok(StorageDetails{
                    size,
                    block_size: stat.st_blksize as u64,
                    storage_type,
                    is_trim_supported: Self::is_trim_supported(fd)
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
    #[allow(dead_code)]
    pub fn custom<P: AsRef<Path>>(
        root: P,
        path_filter: fn(&PathBuf) -> bool,
        meta_filter: fn(&StorageDetails) -> bool
    ) -> FileEnumerator {
        let p = root.as_ref().to_path_buf();
        FileEnumerator { root: p, path_filter, meta_filter }
    }

    #[allow(dead_code)]
    #[cfg(target_os = "macos")]
    pub fn system_drives() -> FileEnumerator {
        FileEnumerator::custom(
            "/dev",
            |p| p.to_str().unwrap().contains("disk0s4"),
            |_m| true
        )
    }
}

impl<'a> StorageEnumerator for FileEnumerator {
    type Ref = FileRef;

    fn try_iter(&self) -> IoResult<Box<Iterator<Item=Self::Ref>>> {
        let rd = read_dir(&self.root)?;
        Ok(Box::new(rd.filter_map(std::io::Result::ok)
            .map(|de| de.path())
            .filter(|path|
                (self.path_filter)(&path.to_path_buf())
            )
            .flat_map(FileRef::new)
            .filter(|r|
                (self.meta_filter)(&r.details)
            )
            .collect::<Vec<_>>().into_iter() // todo: is there a better way to avoid lifetime conflicts?
        ))
    }
}