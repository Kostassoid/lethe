use std::fs::File;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::io::SeekFrom;
use std::os::macos::fs::MetadataExt;

pub mod file;

pub type IoResult<A> = std::io::Result<A>;

pub trait DataContainer {
    fn description(&self) -> &str;
    fn size(&self) -> IoResult<u64>;
    fn block_size(&self) -> IoResult<u64>;
    fn position(&mut self) -> IoResult<u64>;

    fn seek(&mut self, position: u64) -> IoResult<u64>;
    fn read(&self, buffer: &mut [u8]) -> IoResult<u64>;
    fn write(&mut self, data: &[u8]) -> IoResult<()>;
    fn sync(&self) -> IoResult<()>;
}