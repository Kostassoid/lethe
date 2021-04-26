#![cfg(windows)]
extern crate winapi;

#[macro_use]
mod helpers;

use super::*;

mod meta;
use meta::*;

mod access;
use access::*;

mod misc;
use misc::*;

use anyhow::{Context, Result};

impl System {
    pub fn get_storage_devices() -> Result<Vec<impl StorageRef>> {
        let enumerator = DiskDeviceEnumerator::new().with_context(|| {
            if !is_elevated() {
                format!("Make sure you run the application with Administrator permissions!")
            } else {
                format!("") //todo: hints?
            }
        })?;
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
