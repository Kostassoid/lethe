extern crate regex;
extern crate libc;

use std::fs::{File, metadata};
use std::fs::read_dir;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::io::SeekFrom;
use self::super::*;
use std::ffi::CString;

pub struct FileEnumerator {
    root: PathBuf
}

pub struct FileRef {
    path: PathBuf,
    details: StorageDetails
}

pub struct FileAccess {
    file: File
}

impl FileAccess {
    pub fn new<P: AsRef<Path>>(file_path: P) -> IoResult<FileAccess> {
        let file = File::open(file_path.as_ref())?;
        Ok(FileAccess { file })
    }
}

impl std::io::Write for FileAccess {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        unimplemented!()
    }

    fn flush(&mut self) -> IoResult<()> {
        unimplemented!()
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

impl FileRef {
    pub fn new<P: AsRef<Path>>(path: P) -> FileRef {
        let p = path.as_ref().to_path_buf();
        let details = Self::build_details(path).unwrap();
        FileRef { path: p, details }
    }

    fn build_details<P: AsRef<Path>>(path: P) -> IoResult<StorageDetails> {
        unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            let cpath = CString::new(path.as_ref().to_str().unwrap()).unwrap();
            if libc::stat(cpath.as_ptr(), &mut stat) >= 0 {
                Ok(StorageDetails{
                    size: stat.st_size as u64,
                    block_size: stat.st_blksize as u64,
                    is_readonly: false
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

impl FileEnumerator {
    pub fn new<P: AsRef<Path>>(root: P) -> FileEnumerator {
        let p = root.as_ref().to_path_buf();
        FileEnumerator { root: p }
    }

//    fn get_size(&self) -> IoResult<u64> {
//        let meta = metadata(&self.path)?;
//        Ok(meta.len())
//    }
//
//    #[cfg(target_os = "macos")]
//    fn get_block_size(&self) -> IoResult<u64> {
//        use std::os::macos::fs::MetadataExt;
//        let meta = metadata(&self.path)?;
//        Ok(meta.st_blksize())
//    }
//
//    #[cfg(target_os = "windows")]
//    fn get_block_size(&self) -> IoResult<u64> {
//        Ok(4096)
//    }
//
//    #[cfg(target_os = "linux")]
//    fn get_block_size(&self) -> IoResult<u64> {
//        Ok(4096)
//    }
}

impl<'a> StorageEnumerator for FileEnumerator {
    type Ref = FileRef;

    fn iterate(&self) -> IoResult<Box<Iterator<Item=Self::Ref>>> {
        let rd = read_dir(&self.root)?;
        Ok(
            Box::new(rd
                .filter_map(Result::ok)
                .map(|de| {
                    FileRef::new(de.path())
                })
            )
        )
    }
}