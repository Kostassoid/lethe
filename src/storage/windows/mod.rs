#![cfg(windows)]
extern crate winapi;

use crate::storage::*;

mod internal;
use internal::*;

use anyhow::Result;

impl System {
    pub fn get_storage_devices() -> Result<Vec<impl StorageRef>> {
        let enumerator = DiskDeviceEnumerator::new()?;
        let mut devices: Vec<DiskDeviceInfo> = enumerator.flatten().collect();
        devices.append(&mut enumerate_volumes()?);
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(devices)
    }

    pub fn access(storageRef: &dyn StorageRef) -> Result<DeviceAccess> {
        DeviceAccess::new()
    }
}

pub struct DeviceAccess {}

impl DeviceAccess {
    pub fn new() -> Result<DeviceAccess> {
        Ok(DeviceAccess {})
    }
}

impl StorageAccess for DeviceAccess {
    fn position(&mut self) -> Result<u64> {
        unimplemented!();
    }

    fn seek(&mut self, position: u64) -> Result<u64> {
        unimplemented!();
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        unimplemented!();
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        unimplemented!();
    }

    fn flush(&self) -> Result<()> {
        unimplemented!();
    }
}

impl StorageRef for DiskDeviceInfo {
    fn id(&self) -> &str {
        &self.id
    }

    fn details(&self) -> &StorageDetails {
        &self.details
    }
}

impl StorageRef for PartitionInfo {
    fn id(&self) -> &str {
        &self.id
    }

    fn details(&self) -> &StorageDetails {
        &self.details
    }
}
