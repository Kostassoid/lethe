use std::fs::{File, OpenOptions};
use std::path::Path;
use crate::storage::*;
use std::os::unix::io::*;
use ::nix::*;
use std::io::BufReader;
use std::io::BufRead;
use regex::Regex;

pub fn open_file_direct<P: AsRef<Path>>(file_path: P, write_access: bool) -> IoResult<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .create(false)
        .append(false)
        .write(write_access)
        .read(true)
        .truncate(false)
        .custom_flags(libc::O_DIRECT/* | libc::O_DSYNC*/) // should be enough in linux 2.6+
        .open(file_path.as_ref())
}

pub fn get_block_device_size(fd: RawFd) -> u64 {
    // requires linux 2.4.10+
    ioctl_read!(linux_get_block_size, 0x12, 114, u64); // BLKGETSIZE64

    unsafe {
        let mut block_size: u64 = std::mem::zeroed();
        linux_get_block_size(fd, &mut block_size).unwrap();
        block_size
    }
}

pub fn is_trim_supported(_fd: RawFd) -> bool {
    false
}

pub fn get_storage_devices() -> IoResult<Vec<FileRef>> {
    let partitions_file = File::open("/proc/partitions")?;
    let buf = BufReader::new(partitions_file);
    let name_regex = Regex::new(r"\s+(?P<name>\w+)$").unwrap();
    let refs = buf.lines()
        .filter_map(|io_line| {
            let line = io_line.unwrap();
            name_regex
                .captures(line.as_str())
                .map(|c| format!("/dev/{}", &c["name"]))
        })
        .skip(1)
        .flat_map(FileRef::new)
        .collect::<Vec<_>>();

    Ok(refs)
}
