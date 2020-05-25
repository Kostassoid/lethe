#![cfg(windows)]
extern crate winapi;

use crate::storage::*;

#[macro_use]
mod helpers;

mod meta;
use super::windows::meta::*;

mod access;
use access::*;

use anyhow::Result;

impl System {
    pub fn get_storage_devices() -> Result<Vec<impl StorageRef>> {
        let enumerator = DiskDeviceEnumerator::new()?;
        let mut devices: Vec<DiskDeviceInfo> = enumerator.flatten().collect();
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(devices)
    }

    pub fn access(storage_ref: &dyn StorageRef) -> Result<impl StorageAccess> {
        DeviceFile::open(storage_ref.id(), true)
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
