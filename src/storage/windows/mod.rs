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

#[derive(Debug)]
pub struct SystemStorageDevice {
    pub id: String,
    pub details: StorageDetails,
    pub children: Vec<SystemStorageDevice>,
}

impl System {
    pub fn get_storage_devices() -> Result<Vec<SystemStorageDevice>> {
        let enumerator = DiskDeviceEnumerator::new().with_context(|| {
            if !is_elevated() {
                format!("Make sure you run the application with Administrator permissions!")
            } else {
                format!("") //todo: hints?
            }
        })?;
        let mut devices: Vec<SystemStorageDevice> = enumerator.collect();
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(devices)
    }

    pub fn access(device: &SystemStorageDevice) -> Result<impl StorageAccess> {
        DeviceFile::open(&device.id, true)
    }
}
