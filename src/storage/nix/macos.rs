//extern crate IOKit_sys as iokit;
use ::nix::*;
use anyhow::Result;
use regex::Regex;
use std::collections::HashMap;
use std::fs::read_dir;
use std::fs::{File, OpenOptions};
use std::os::unix::io::*;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::storage::*;

impl System {
    pub fn get_storage_devices() -> Result<Vec<StorageRef>> {
        get_storage_devices()
    }
}

pub fn open_file_direct<P: AsRef<Path>>(file_path: P, write_access: bool) -> Result<File> {
    let file = OpenOptions::new()
        .create(false)
        .append(false)
        .write(write_access)
        .read(true)
        .truncate(false)
        .open(file_path.as_ref())?;

    unsafe {
        let fd = file.as_raw_fd();
        nix::libc::fcntl(fd, nix::libc::F_NOCACHE, 1);
    }

    Ok(file)
}

pub fn get_block_device_size(fd: libc::c_int) -> u64 {
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

#[allow(dead_code)]
pub fn is_trim_supported(fd: RawFd) -> bool {
    ioctl_read!(dk_get_features, b'd', 76, u32); // DKIOCGETFEATURES

    unsafe {
        let mut features: u32 = std::mem::zeroed();
        dk_get_features(fd, &mut features)
            .map(|_| (features & 0x00000010) > 0) // DK_FEATURE_UNMAP
            .unwrap_or(false)
    }
}

pub fn get_storage_devices() -> Result<Vec<StorageRef>> {
    discover_file_based_devices(
        "/dev",
        |p| p.to_str().unwrap().contains("/dev/rdisk"),
        |_m| true,
    )
}

fn discover_file_based_devices<P: AsRef<Path>>(
    root: P,
    path_filter: fn(&PathBuf) -> bool,
    meta_filter: fn(&StorageDetails) -> bool,
) -> Result<Vec<StorageRef>> {
    let rd = read_dir(&root)?;
    let mut refs = rd
        .filter_map(std::io::Result::ok)
        .map(|de| de.path())
        .filter(|path| (path_filter)(&path.to_path_buf()))
        .flat_map(StorageRef::new)
        .filter(|r| (meta_filter)(&r.details))
        .collect::<Vec<_>>();

    refs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(refs)
}

pub fn get_diskutils_info<P: AsRef<Path>>(path: P) -> Result<HashMap<String, String>> {
    let mut command = Command::new("/usr/sbin/diskutil");
    command.arg("info").arg(path.as_ref().to_str().unwrap());

    let output = command.output()?;
    if !output.status.success() {
        return Err(anyhow!("Can't run diskutil"));
    };

    let pattern = Regex::new(r"^\s*([^:]+):\s*(.*)$")?;

    let props: HashMap<_, _> = String::from_utf8(output.stdout)?
        .lines()
        .filter_map(|line| pattern.captures(line))
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .into_iter()
        .collect();

    Ok(props)
}

pub fn enrich_storage_details<P: AsRef<Path>>(path: P, details: &mut StorageDetails) -> Result<()> {
    let du = get_diskutils_info(path)?;

    details.mount_point = du.get("Mount Point").map(|s| s.to_owned());

    if du.get("Whole").unwrap_or(&String::from("Yes")) == "No" {
        details.storage_type = StorageType::Partition;
    } else {
        details.storage_type = match du.get("Removable Media").unwrap_or(&String::new()) {
            x if x == "Removable" => StorageType::Removable,
            x if x == "Fixed" => StorageType::Fixed,
            _ => StorageType::Unknown,
        };
    }

    Ok(())
}
