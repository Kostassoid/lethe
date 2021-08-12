#![cfg(windows)]
extern crate winapi;

use crate::storage::*;

#[macro_use]
mod helpers;

mod meta;
use super::windows::meta::*;

mod access;
use access::*;

mod misc;
use misc::*;

use anyhow::{Context, Result};

impl System {
    pub fn enumerate_storage_devices() -> Result<Vec<StorageRef>> {
        let enumerator = DiskDeviceEnumerator::new().with_context(|| {
            if !is_elevated() {
                format!("Make sure you run the application with Administrator permissions!")
            } else {
                format!("") //todo: hints?
            }
        })?;
        let mut devices: Vec<StorageRef> = enumerator.collect();
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(devices)
    }
}

impl StorageDevice for StorageRef {
    fn access(&self) -> Result<Box<dyn StorageAccess>> {
        CompositeStorageAccess::open(self).map(|a| Box::new(a) as Box<dyn StorageAccess>)
    }
}

/// On Windows, to work with a low level PhysicalDrive, we have to acquire locks to all partitions/volumes
/// located on this drive and we have to keep these locks for the duration of wiping process.
/// In terms of implementation we just open the whole tree of devices for write which effectively
/// locks and dismounts every volume in that tree.
struct CompositeStorageAccess {
    device: DeviceFile,
    _children: Vec<DeviceFile>,
}

impl CompositeStorageAccess {
    fn open(device: &StorageRef) -> Result<Self> {
        let children: Result<Vec<DeviceFile>> = device
            .children
            .iter()
            .map(|c| DeviceFile::open(&c.id, true))
            .collect();

        let device = DeviceFile::open(&device.id, true)?;

        Ok(Self {
            device,
            _children: children?,
        })
    }
}

impl StorageAccess for CompositeStorageAccess {
    fn position(&mut self) -> Result<u64> {
        self.device.position()
    }

    fn seek(&mut self, position: u64) -> Result<u64> {
        self.device.seek(position)
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.device.read(buffer)
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.device.write(data)
    }

    fn flush(&mut self) -> Result<()> {
        self.device.flush()
    }
}
