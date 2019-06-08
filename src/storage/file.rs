use std::fs::File;
use std::fs::read_dir;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::io::SeekFrom;
use self::super::*;

pub struct FileEnumerator {
    root: PathBuf
}

pub struct FileReference {
    path: PathBuf
}

pub struct FileContainer {
    file: File
}

impl FileContainer {
    pub fn new<P: AsRef<Path>>(file_path: P) -> IoResult<FileContainer> {
        let file = File::open(file_path.as_ref())?;
        Ok(FileContainer { file })
    }
}

impl StorageContainer for FileContainer {

    fn size(&self) -> IoResult<u64> {
        File::metadata(&self.file).map(|m| m.len())
    }

    #[cfg(target_os = "linux")]
    fn block_size(&self) -> IoResult<u64> {
        Ok(4096)
    }

    #[cfg(target_os = "macos")]
    fn block_size(&self) -> IoResult<u64> {
        use std::os::macos::fs::MetadataExt;
        File::metadata(&self.file)
            .map(|m| m.st_blksize())
    }

    #[cfg(target_os = "windows")]
    fn block_size(&self) -> IoResult<u64> {
        Ok(4096)
    }

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

impl FileReference {
    pub fn new<P: AsRef<Path>>(path: P) -> FileReference {
        let p = path.as_ref().to_path_buf();
        FileReference { path: p }
    }
}

impl StorageReference for FileReference {
    type Container = FileContainer;

    fn description(&self) -> &str {
        self.path.to_str().unwrap()
    }

    fn to_container(&self) -> IoResult<Box<Self::Container>> {
        FileContainer::new(&self.path).map(Box::new)
    }
}

impl FileEnumerator {
    pub fn new<P: AsRef<Path>>(root: P) -> FileEnumerator {
        let p = root.as_ref().to_path_buf();
        FileEnumerator { root: p }
    }
}

impl StorageEnumerator for FileEnumerator {
    type Reference = FileReference;

    fn iterate(&self) -> IoResult<Box<Iterator<Item=Self::Reference>>> {
        let rd = read_dir(&self.root)?;
        Ok(
            Box::new(rd
                .filter(|de| de.as_ref().unwrap().file_type().unwrap().is_file())
                .map(|de| FileReference::new(de.unwrap().path()))
            )
        )
    }
}