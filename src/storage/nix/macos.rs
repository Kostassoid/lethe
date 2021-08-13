use ::nix::*;
use anyhow::{Context, Result};
use plist;
use std::fs::{File, OpenOptions};
use std::os::unix::io::*;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::storage::*;
use std::ffi::CString;

impl System {
    pub fn enumerate_storage_devices() -> Result<Vec<StorageRef>> {
        DiskUtilCli::default().get_list()
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

#[allow(dead_code)]
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

//todo: remove this common dependency, the current implementation is not relying on StorageRef ctor
#[allow(dead_code)]
pub fn enrich_storage_details<P: AsRef<Path>>(
    _path: P,
    _details: &mut StorageDetails,
) -> Result<()> {
    Ok(())
}

pub trait StorageDeviceEnumerator {
    fn get_list(&self) -> Result<Vec<StorageRef>>;
}

pub struct DiskUtilCli {
    path: PathBuf,
}

impl Default for DiskUtilCli {
    fn default() -> Self {
        DiskUtilCli {
            path: "/usr/sbin/diskutil".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DUPartition {
    device_identifier: String,
    // size: u64,
    // volume_name: Option<String>,
    // mount_point: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DUDiskAndPartitions {
    device_identifier: String,
    // size: u64,
    partitions: Option<Vec<DUPartition>>,
    a_p_f_s_volumes: Option<Vec<DUPartition>>,
    // volume_name: Option<String>,
    // mount_point: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DUDiskInfo {
    size: u64,
    device_block_size: usize,
    removable: bool,
    whole_disk: bool,
    volume_name: Option<String>,
    mount_point: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DUList {
    all_disks_and_partitions: Vec<DUDiskAndPartitions>,
}

impl DiskUtilCli {
    fn get_storage_details(&self, id: &str) -> Result<StorageDetails> {
        let mut command = Command::new(&self.path);
        command.arg("info").arg("-plist").arg(id);

        let output = command.output()?;
        if !output.status.success() {
            return Err(anyhow!("Can't run diskutil"));
        };

        let info: DUDiskInfo =
            plist::from_bytes(&output.stdout).context("Unable to parse diskutil info plist")?;

        let storage_type = if !info.whole_disk {
            StorageType::Partition
        } else if info.removable {
            StorageType::Removable
        } else {
            StorageType::Fixed
        };

        Ok(StorageDetails {
            size: info.size,
            block_size: info.device_block_size,
            storage_type,
            mount_point: info.mount_point.to_owned(),
            label: info.volume_name.to_owned(),
        })
    }
}

impl StorageDeviceEnumerator for DiskUtilCli {
    fn get_list(&self) -> Result<Vec<StorageRef>> {
        let mut command = Command::new(&self.path);
        command.arg("list").arg("-plist");

        let output = command.output()?;
        if !output.status.success() {
            return Err(anyhow!("Can't run diskutil"));
        };

        let info: DUList =
            plist::from_bytes(&output.stdout).context("Unable to parse diskutil info plist")?;

        info.all_disks_and_partitions
            .iter()
            .map(|d| {
                let children: Result<Vec<StorageRef>> = d
                    .partitions
                    .as_ref()
                    .unwrap_or(&vec![])
                    .iter()
                    .chain(d.a_p_f_s_volumes.as_ref().unwrap_or(&vec![]).iter())
                    .map(|p| {
                        Ok(StorageRef {
                            id: format!("/dev/r{}", p.device_identifier),
                            details: self.get_storage_details(&p.device_identifier)?,
                            children: vec![],
                        })
                    })
                    .collect();

                Ok(StorageRef {
                    id: format!("/dev/r{}", d.device_identifier),
                    details: self.get_storage_details(&d.device_identifier)?,
                    children: children?,
                })
            })
            .collect()
    }
}

pub fn unmount(path: &str) -> Result<()> {
    let cpath = CString::new(path)?;
    match unsafe { libc::unmount(cpath.as_ptr(), libc::MNT_FORCE) } {
        0 => Ok(()),
        _ if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) => Ok(()), // not found
        _ => Err(anyhow::Error::new(std::io::Error::last_os_error())
            .context("Failed to unmount a volume")),
    }
}
