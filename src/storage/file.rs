use std::fs::File;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::io::SeekFrom;
use self::super::*;

struct FileContainer {
    path: PathBuf,
    file: File,
}

impl FileContainer {
    fn new<P: AsRef<Path>>(file_path: P) -> IoResult<FileContainer> {
        let file = File::open(file_path.as_ref())?;
        let path = file_path.as_ref().to_path_buf();
        Ok(FileContainer { path, file })
    }
}

impl StorageContainer for FileContainer {
    fn description(&self) -> &str {
        self.path.to_str().unwrap()
    }

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

    fn read(&self, buffer: &mut [u8]) -> IoResult<u64> {
        unimplemented!()
    }

    fn write(&mut self, data: &[u8]) -> IoResult<()> {
        unimplemented!()
    }

    fn sync(&self) -> IoResult<()> {
        self.file.sync_all()
    }
}
