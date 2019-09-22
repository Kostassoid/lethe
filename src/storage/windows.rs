#![cfg(windows)]
extern crate winapi;

use crate::storage::*;
use anyhow::Result;
use winapi::ctypes::c_ulong;
use winapi::ctypes::wchar_t;
use winapi::shared::minwindef::FALSE;
use winapi::um::fileapi::{GetDiskFreeSpaceExW, GetLogicalDriveStringsW, GetVolumeInformationW};
use winapi::um::winioctl::GUID_DEVINTERFACE_DISK;
use winapi::um::winnt::ULARGE_INTEGER;

use std::ffi::{CStr, CString, OsStr};
use std::os::windows::prelude::*;
use std::time::Duration;
use std::{io, mem, ptr};

use winapi::shared::guiddef::*;
use winapi::shared::minwindef::*;
use winapi::shared::ntdef::CHAR;
use winapi::shared::winerror::*;
use winapi::um::cguid::GUID_NULL;
use winapi::um::commapi::*;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::*;
use winapi::um::handleapi::*;
use winapi::um::processthreadsapi::GetCurrentProcess;
use winapi::um::setupapi::*;
use winapi::um::winbase::*;
use winapi::um::winnt::{
    DUPLICATE_SAME_ACCESS, FILE_ATTRIBUTE_NORMAL, GENERIC_READ, GENERIC_WRITE, HANDLE, KEY_READ,
};
use winapi::um::winreg::*;

impl System {
    pub fn get_storage_devices() -> Result<Vec<impl StorageRef>> {
        let diskClassDevices = SetupDiGetClassDevs(
            &diskClassDeviceInterfaceGuid,
            NULL,
            NULL,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        );
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
