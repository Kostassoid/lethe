#![cfg(windows)]
extern crate winapi;

use crate::storage::*;
use winapi::um::fileapi::{GetLogicalDriveStringsW, GetVolumeInformationW};
use winapi::um::winioctl::GUID_DEVINTERFACE_DISK;


use std::ffi::{CString, OsStr};
use std::{mem, ptr};

use winapi::shared::minwindef::*;
use winapi::um::fileapi::*;
use winapi::um::setupapi::*;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_NORMAL, GENERIC_READ, GENERIC_WRITE, HANDLE, KEY_READ,
};

mod internal;
use internal::*;

#[derive(Debug)]
pub struct DeviceRef {
    id: String,
    details: StorageDetails
}

impl System {
    pub fn get_storage_devices() -> IoResult<Vec<impl StorageRef>> {

        let device_info_list = unsafe { 
            SetupDiGetClassDevsW(&GUID_DEVINTERFACE_DISK,
                ptr::null(),
                ptr::null_mut(),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)
        };

        let mut refs: Vec<DeviceRef> = Vec::new();
        let mut device_index: DWORD = 0;
        loop {

            let mut device_interface_data = unsafe { mem::uninitialized::<SP_DEVICE_INTERFACE_DATA>() };
            device_interface_data.cbSize = mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as UINT;

            let result = unsafe { SetupDiEnumDeviceInterfaces(
                device_info_list, 
                ptr::null_mut(), 
                &GUID_DEVINTERFACE_DISK, 
                device_index, 
                &mut device_interface_data) };

            if result == 0 {
                break;
            }

            let mut required_size: u32 = 0;

            unsafe { SetupDiGetDeviceInterfaceDetailW(
                device_info_list,
                &mut device_interface_data,
                ptr::null_mut(),
                0,
                &mut required_size,
                ptr::null_mut())
            };
            
            if required_size != 0 {
                let mut interface_details = DeviceInterfaceDetailData::new(required_size as usize).unwrap();

                unsafe { SetupDiGetDeviceInterfaceDetailW(
                    device_info_list,
                    &mut device_interface_data,
                    interface_details.get(),
                    required_size,
                    ptr::null_mut(),
                    ptr::null_mut())
                };

                refs.push(DeviceRef { id: interface_details.path(), details: StorageDetails::default() });
            }
            
            device_index += 1;
        }

        unsafe {
            SetupDiDestroyDeviceInfoList(device_info_list);
        }        

        Ok(refs)
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

impl StorageRef for DeviceRef {
    type Access = DeviceAccess;

    fn id(&self) -> &str {
        &self.id
    }

    fn details(&self) -> &StorageDetails {
        &self.details
    }

    fn access(&self) -> IoResult<Box<Self::Access>> {
        DeviceAccess::new().map(Box::new)
    }
}
