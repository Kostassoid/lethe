//extern crate IOKit_sys as iokit;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::fs::read_dir;
use std::os::unix::io::*;
use ::nix::*;

use crate::storage::*;

pub fn open_file_direct<P: AsRef<Path>>(file_path: P, write_access: bool) -> IoResult<File> {

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

pub fn get_storage_devices() -> IoResult<Vec<FileRef>> {
    discover_file_based_devices(
        "/dev",
        |p| p.to_str().unwrap().contains("/dev/rdisk"),
        |_m| true
    )
}

fn discover_file_based_devices<P: AsRef<Path>>(
    root: P,
    path_filter: fn(&PathBuf) -> bool,
    meta_filter: fn(&StorageDetails) -> bool
) -> IoResult<Vec<FileRef>> {
    let rd = read_dir(&root)?;
    let mut refs = rd.filter_map(std::io::Result::ok)
        .map(|de| de.path())
        .filter(|path|
            (path_filter)(&path.to_path_buf())
        )
        .flat_map(FileRef::new)
        .filter(|r|
            (meta_filter)(&r.details)
        )
        .collect::<Vec<_>>();
    
    refs.sort_by(|a, b| a.path.to_str().cmp(&b.path.to_str()));
    Ok(refs)
}

/*
    fn get_mounts() -> IoResult<()> {
        unsafe {
            let mut stat: [libc::statfs; 16] = std::mem::zeroed();
            let total = libc::statvfs(stat, 16, 1 /* libc::MNT_WAIT */);

            for i in 0..total {
                println!("!!! statfs {} = {:?}", i, stat.get(i).unwrap());
            }
        }

        Ok(())
    }
    */


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

