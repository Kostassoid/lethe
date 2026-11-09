#![cfg(windows)]

use anyhow::{Context, Result};

use crate::storage::{StorageAccess, StorageDevice, StorageRef, System};

mod access;
mod enumeration;
mod misc;
mod physical_drive;
mod volumes;

use access::{DeviceAccess, DeviceFile};
use enumeration::DiskDeviceEnumerator;
use misc::is_elevated;

impl System {
    pub fn enumerate_storage_devices() -> Result<Vec<StorageRef>> {
        let enumerator = DiskDeviceEnumerator::new().with_context(|| {
            if is_elevated() {
                String::new()
            } else {
                "Make sure you run the application with Administrator permissions!".to_string()
            }
        })?;
        let mut devices: Vec<StorageRef> = enumerator.collect();
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(devices)
    }
}

impl StorageDevice for StorageRef {
    fn access(&self) -> Result<Box<dyn StorageAccess>> {
        CompositeStorageAccess::open(self).map(|access| Box::new(access) as Box<dyn StorageAccess>)
    }
}

/// On Windows, to work with a low level `PhysicalDrive`, we have to acquire locks to all partitions/volumes
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
            .map(|child| DeviceFile::open(&child.id, DeviceAccess::Wipe))
            .collect();

        let device = DeviceFile::open(&device.id, DeviceAccess::Wipe)?;

        Ok(Self {
            device,
            _children: children?,
        })
    }
}

impl StorageAccess for CompositeStorageAccess {
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
