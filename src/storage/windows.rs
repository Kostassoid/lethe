#![cfg(windows)]
extern crate winapi;

use crate::storage::*;
use anyhow::Result;
use winapi::ctypes::c_ulong;
use winapi::shared::minwindef::FALSE;
use winapi::um::fileapi::{GetDiskFreeSpaceExW, GetLogicalDriveStringsW, GetVolumeInformationW};
use winapi::um::winnt::ULARGE_INTEGER;

impl System {
    pub fn get_storage_devices() -> Result<Vec<impl StorageRef>> {
        Ok(vec![DeviceRef {}])
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

pub struct DeviceRef {}

impl StorageRef for DeviceRef {
    type Access = DeviceAccess;

    fn id(&self) -> &str {
        unimplemented!();
    }

    fn details(&self) -> &StorageDetails {
        unimplemented!();
    }

    fn access(&self) -> Result<Box<Self::Access>> {
        DeviceAccess::new().map(Box::new)
    }
}
