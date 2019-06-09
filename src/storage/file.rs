extern crate regex;

use std::fs::{File, metadata};
use std::fs::read_dir;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::io::SeekFrom;
use self::super::*;
use core::borrow::Borrow;
use regex::Regex;

pub struct FileEnumerator {
    root: PathBuf,
    filter: fn(&PathBuf) -> bool
}

pub struct FileDetails {
    path: PathBuf
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

    fn sync(&self) -> IoResult<()> {
        self.file.sync_all()
    }
}

impl FileDetails {
    pub fn new<P: AsRef<Path>>(path: P) -> FileDetails {
        let p = path.as_ref().to_path_buf();
        FileDetails { path: p }
    }
}

impl StorageDetails for FileDetails {
    type Access = FileAccess;

    fn name(&self) -> &str {
        self.path.to_str().unwrap()
    }

    fn size(&self) -> IoResult<u64> {
        let meta = metadata(&self.path)?;
        Ok(meta.len())
    }

    #[cfg(target_os = "linux")]
    fn block_size(&self) -> IoResult<u64> {
        Ok(4096)
    }

    #[cfg(target_os = "macos")]
    fn block_size(&self) -> IoResult<u64> {
        use std::os::macos::fs::MetadataExt;
        let meta = metadata(&self.path)?;
        Ok(meta.st_blksize())
    }

    #[cfg(target_os = "windows")]
    fn block_size(&self) -> IoResult<u64> {
        Ok(4096)
    }

    fn is_readonly(&self) -> bool {
        unimplemented!()
    }

    fn is_ok(&self) -> bool {
        true
    }

    fn access(&self) -> IoResult<Box<Self::Access>> {
        FileAccess::new(&self.path).map(Box::new)
    }
}

impl FileEnumerator {
    pub fn new<P: AsRef<Path>>(root: P, filter: fn(&PathBuf) -> bool) -> FileEnumerator {
        let p = root.as_ref().to_path_buf();
        FileEnumerator { root: p, filter }
    }
}

impl<'a> StorageEnumerator for FileEnumerator {
    type Details = FileDetails;

    fn iterate(&self) -> IoResult<Box<Iterator<Item=Self::Details>>> {
        let rd = read_dir(&self.root)?;
        let f = self.filter;
        Ok(
            Box::new(rd
                .filter_map(Result::ok)
                .filter(move |de| {
                    println!("Checking {} ({:?})", &de.path().to_str().unwrap(), &de.file_type().unwrap());
                    f(&de.path()) &&
                        de.file_type()
                            .map(|t| t.is_file())
                            .unwrap_or(false)
                })
                .map(|de|
                    FileDetails::new(de.path())
                )
            )
        )
    }
}