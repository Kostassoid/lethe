#![cfg(windows)]

extern crate winapi;

use crate::storage::*;
use winapi::ctypes::c_ulong;
use winapi::shared::minwindef::FALSE;
use winapi::um::fileapi::{GetDiskFreeSpaceExW, GetLogicalDriveStringsW, GetVolumeInformationW};
use winapi::um::winnt::ULARGE_INTEGER;

pub struct StorageDeviceEnumerator {
}

impl System {
    pub fn system_drives() -> Box<impl StorageEnumerator> {
        Box::new(StorageDeviceEnumerator{})
    }
}

pub struct DeviceAccess {

}

impl DeviceAccess {
    pub fn new() -> IoResult<DeviceAccess> {
        Ok(DeviceAccess{})
    }
}

impl StorageAccess for DeviceAccess {

    fn position(&mut self) -> IoResult<u64> {
        unimplemented!();
    }

    fn seek(&mut self, position: u64) -> IoResult<u64> {
        unimplemented!();
    }

    fn read(&mut self, buffer: &mut [u8]) -> IoResult<usize> {
        unimplemented!();
    }

    fn write(&mut self, data: &[u8]) -> IoResult<()> {
        unimplemented!();
    }

    fn flush(&self) -> IoResult<()> {
        unimplemented!();
    }
}

pub struct DeviceRef {

}

impl StorageRef for DeviceRef {
    type Access = DeviceAccess;

    fn id(&self) -> &str {
        unimplemented!();
    }

    fn details(&self) -> &StorageDetails {
        unimplemented!();
    }

    fn access(&self) -> IoResult<Box<Self::Access>> {
        DeviceAccess::new().map(Box::new)
    }
}

impl StorageEnumerator for StorageDeviceEnumerator {
    type Ref = DeviceRef;

    fn list(&self) -> IoResult<Vec<DeviceRef>> {
        Ok(Vec::default())
    }
}
