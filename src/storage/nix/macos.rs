//extern crate IOKit_sys as iokit;
use ::nix::*;
use anyhow::Result;
use std::fs::read_dir;
use std::fs::{File, OpenOptions};
use std::os::unix::io::*;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::storage::*;

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

pub fn is_trim_supported(fd: RawFd) -> bool {
    ioctl_read!(dk_get_features, b'd', 76, u32); // DKIOCGETFEATURES

    unsafe {
        let mut features: u32 = std::mem::zeroed();
        dk_get_features(fd, &mut features)
            .map(|_| (features & 0x00000010) > 0) // DK_FEATURE_UNMAP
            .unwrap_or(false)
    }
}

pub fn get_storage_devices() -> Result<Vec<FileRef>> {
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
) -> Result<Vec<FileRef>> {
    let rd = read_dir(&root)?;
    let mut refs = rd
        .filter_map(std::io::Result::ok)
        .map(|de| de.path())
        .filter(|path| (path_filter)(&path.to_path_buf()))
        .flat_map(FileRef::new)
        .filter(|r| (meta_filter)(&r.details))
        .collect::<Vec<_>>();

    refs.sort_by(|a, b| a.path.to_str().cmp(&b.path.to_str()));
    Ok(refs)
}

// #[cfg(target_os = "macos")]
// pub struct IOKitEnumerator {}

// #[cfg(target_os = "macos")]
// impl StorageEnumerator for IOKitEnumerator {
//     type Ref = FileRef;

//     fn list(&self) -> IoResult<Vec<Self::Ref>> {
//         use mach::port::{mach_port_t,MACH_PORT_NULL};
//         use mach::kern_return::KERN_SUCCESS;
//         use iokit::*;
//         unsafe {
//             let mut master_port: mach_port_t = MACH_PORT_NULL;

//             let classes_to_match = IOServiceMatching(kIOSerialBSDServiceValue());
//         }

//         Ok(Vec::new())
//     }
// }

pub fn get_bsd_device_name<P: AsRef<Path>>(path: P) -> Result<String> {
    let n = path.as_ref()
        .file_name()
        .ok_or(anyhow!("Invalid path"))?
        .to_string_lossy();
    if n.starts_with("rdisk") {
        Ok(n[1..].into())
    } else {
        Ok(n.into())
    }
}

pub fn resolve_storage_type<P: AsRef<Path>>(path: P) -> Result<StorageType> {

    let bsd_name = get_bsd_device_name(path)?;
    let output = Command::new("/usr/sbin/diskutil")
                     .arg(fmt!("info {}", bsd_name))
                     .output()?;

    assert!(output.status.success());

    println!("status: {}", output.status);
    println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    println!("stderr: {}", String::from_utf8_lossy(&output.stderr));

    Ok(StorageType::Unknown)
}

pub fn resolve_mount_point<P: AsRef<Path>>(path: P) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_bsd_name_resolver() {
        assert_eq!(get_bsd_device_name("/dev/rdisk0").unwrap(), "disk0".to_owned());
        assert_eq!(get_bsd_device_name("/dev/rdisk0s1").unwrap(), "disk0s1".to_owned());
        assert_eq!(get_bsd_device_name("/dev/disk2").unwrap(), "disk2".to_owned());
        assert_eq!(get_bsd_device_name("/rdisk3").unwrap(), "disk3".to_owned());
        
        assert!(get_bsd_device_name("").is_err());
    }
}
