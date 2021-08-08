use crate::storage::*;
use ::nix::*;
use anyhow::{Context, Result};
use regex::Regex;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::BufRead;
use std::io::BufReader;
use std::os::unix::io::*;
use std::path::Path;
use sysfs_class::{Block, SysClass};

impl System {
    pub fn get_storage_devices() -> Result<Vec<StorageRef>> {
        get_storage_devices()
    }
}

pub fn open_file_direct<P: AsRef<Path>>(file_path: P, write_access: bool) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .create(false)
        .append(false)
        .write(write_access)
        .read(true)
        .truncate(false)
        .custom_flags(libc::O_DIRECT /* | libc::O_DSYNC*/) // should be enough in linux 2.6+
        .open(file_path.as_ref())
        .context(format!(
            "Unable to open file-device {}",
            file_path.as_ref().to_str().unwrap_or("?")
        ))
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

#[allow(dead_code)]
pub fn is_trim_supported(_fd: RawFd) -> bool {
    false
}

pub fn resolve_storage_type<P: AsRef<Path>>(path: P) -> Result<StorageType> {
    use sysfs_class::{Block, SysClass};

    let name = path.as_ref().file_name().unwrap();

    //todo: don't re-iterate for each device
    for block in Block::all()? {
        if block.has_device() {
            if block.path().file_name().unwrap() == name {
                return if block.removable()? == 1 {
                    Ok(StorageType::Removable)
                } else {
                    Ok(StorageType::Fixed)
                };
            }

            if block
                .children()?
                .iter()
                .find(|c| c.path().file_name().unwrap() == name)
                .is_some()
            {
                return Ok(StorageType::Partition);
            }
        }
    }
    Ok(StorageType::Unknown)
}

pub fn resolve_mount_point<P: AsRef<Path>>(path: P) -> Result<Option<String>> {
    let s = path.as_ref().to_str().unwrap();
    let f = File::open("/proc/mounts")?;
    let reader = BufReader::new(f);

    for line in reader.lines() {
        let l = line?;
        let parts: Vec<&str> = l.split_whitespace().collect();
        if parts[0] == s {
            return Ok(Some(parts[1].to_string()));
        }
    }
    Ok(None)
}

pub fn get_storage_devices() -> Result<Vec<StorageRef>> {
    // let partitions_file = File::open("/proc/partitions")?;
    // let buf = BufReader::new(partitions_file);
    // let name_regex = Regex::new(r"\s+(?P<name>\w+)$").unwrap();
    // let refs = buf
    //     .lines()
    //     .filter_map(|io_line| {
    //         let line = io_line.unwrap();
    //         name_regex
    //             .captures(line.as_str())
    //             .map(|c| format!("/dev/{}", &c["name"]))
    //     })
    //     .skip(1)
    //     .flat_map(StorageRef::new)
    //     .collect::<Vec<_>>();

    use sysfs_class::{Block, SysClass};
    let root = Block::all()?;
    // root.iter().for_each(|d| {
    //     println!(
    //         "path: /dev/{}, device: {}",
    //         d.path().file_name().unwrap().to_str().unwrap(),
    //         d.has_device()
    //     )
    // });
    let mut refs = root
        .iter()
        .filter(|d| d.has_device())
        .flat_map(build_device_info)
        .collect::<Vec<_>>();

    refs.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(refs)
}

fn build_device_info(d: &Block) -> Option<StorageRef> {
    let device_path = format!("/dev/{}", d.path().file_name()?.to_str()?);
    let children = d
        .children()
        .unwrap_or(vec![])
        .iter()
        .flat_map(build_device_info)
        .collect();

    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let cpath = CString::new(device_path.as_str()).ok()?;
    unsafe {
        if libc::stat(cpath.as_ptr(), &mut stat) < 0 {
            return None;
        }
    }

    let storage_type = if d.parent_device().is_some() {
        StorageType::Partition
    } else if d.removable().ok()? == 1 {
        StorageType::Removable
    } else {
        StorageType::Fixed
    };

    // println!(
    //     "stat = {}, logical = {}, physical = {}",
    //     stat.st_blksize,
    //     d.queue_logical_block_size().ok()?,
    //     d.queue_physical_block_size().ok()?
    // );

    let details = StorageDetails {
        size: d.size().ok()? * 512,
        block_size: stat.st_blksize as usize,
        storage_type,
        mount_point: resolve_mount_point(&device_path).unwrap_or(None),
        label: None,
    };

    Some(StorageRef {
        id: device_path,
        details,
        children,
    })
}

pub fn enrich_storage_details<P: AsRef<Path>>(path: P, details: &mut StorageDetails) -> Result<()> {
    details.mount_point = resolve_mount_point(&path).unwrap_or(None);
    details.storage_type = resolve_storage_type(&path).unwrap_or(StorageType::Unknown);
    Ok(())
}
