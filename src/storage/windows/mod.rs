#![cfg(windows)]
extern crate winapi;

use crate::storage::*;
use winapi::um::fileapi::{GetLogicalDriveStringsW, GetVolumeInformationW};

use std::ffi::{CString, OsStr};
use std::{mem, ptr};

use winapi::shared::minwindef::*;
use winapi::um::fileapi::*;
use winapi::um::setupapi::*;
use winapi::um::winnt::{FILE_ATTRIBUTE_NORMAL, GENERIC_READ, GENERIC_WRITE, HANDLE, KEY_READ};

mod internal;
use internal::*;

use anyhow::Result;

impl System {
    pub fn get_storage_devices() -> Result<Vec<impl StorageRef>> {
        let enumerator = DiskDeviceEnumerator::new()?;
        Ok(enumerator.flatten().collect())
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

impl StorageRef for DiskPartitionInfo {
    type Access = DeviceAccess;

    fn id(&self) -> &str {
        &self.id
    }

    fn details(&self) -> &StorageDetails {
        &self.details
    }

    fn access(&self) -> Result<Box<Self::Access>> {
        DeviceAccess::new().map(Box::new)
    }
}
