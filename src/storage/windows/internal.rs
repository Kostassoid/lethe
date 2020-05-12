use libc;

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::slice;
use std::{mem, ptr};

use crate::storage::*;
use anyhow::Result;

extern crate winapi;
use winapi::_core::ptr::{null, null_mut};
use winapi::shared::minwindef::*;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::OPEN_EXISTING;
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::setupapi::*;
use winapi::um::winbase::{FormatMessageW, LocalFree};
use winapi::um::winioctl::GUID_DEVINTERFACE_DISK;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, GENERIC_READ, HANDLE, LPWSTR, WCHAR,
};
use winapi::um::{fileapi, ioapiset, winioctl};

macro_rules! offset_of {
    ($ty:ty, $field:ident) => {
        unsafe { &(*(0 as *const $ty)).$field as *const _ as usize }
    };
}

fn from_wide_ptr(ptr: *const u16, len: usize) -> String {
    assert!(!ptr.is_null() && len % 2 == 0);
    let slice = unsafe { slice::from_raw_parts(ptr, len / 2) };
    OsString::from_wide(slice).to_string_lossy().into_owned()
}

pub struct DeviceInterfaceDetailData {
    data: PSP_DEVICE_INTERFACE_DETAIL_DATA_W,
    path_len: usize,
}

impl DeviceInterfaceDetailData {
    pub fn new(size: usize) -> Result<Self> {
        let mut cb_size = mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        if cfg!(target_pointer_width = "32") {
            cb_size = 4 + 2; // 4-byte uint + default TCHAR size. size_of is inaccurate.
        }

        if size < cb_size {
            return Err(anyhow!("DeviceInterfaceDetailData is too small. {}", size));
        }

        let data = unsafe { libc::malloc(size) as PSP_DEVICE_INTERFACE_DETAIL_DATA_W };
        if data.is_null() {
            return Err(anyhow!(
                "Unable to allocate memory for PSP_DEVICE_INTERFACE_DETAIL_DATA_W."
            ));
        }

        // Set total size of the structure.
        unsafe { (*data).cbSize = cb_size as UINT };

        // Compute offset of `SP_DEVICE_INTERFACE_DETAIL_DATA_W.DevicePath`.
        let offset = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);

        Ok(Self {
            data,
            path_len: size - offset,
        })
    }

    pub fn get(&self) -> PSP_DEVICE_INTERFACE_DETAIL_DATA_W {
        self.data
    }

    pub fn path(&self) -> String {
        unsafe { from_wide_ptr((*self.data).DevicePath.as_ptr(), self.path_len - 2) }
    }
}

impl Drop for DeviceInterfaceDetailData {
    fn drop(&mut self) {
        unsafe { libc::free(self.data as *mut libc::c_void) };
    }
}

pub struct DiskDeviceEnumerator {
    device_info_list: HDEVINFO,
    device_index: DWORD,
}

impl DiskDeviceEnumerator {
    pub fn new() -> Result<Self> {
        let device_info_list = unsafe {
            SetupDiGetClassDevsW(
                &GUID_DEVINTERFACE_DISK,
                ptr::null(),
                ptr::null_mut(),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        };
        if device_info_list == INVALID_HANDLE_VALUE {
            return Err(anyhow!("Unable to initialize disk device enumeration"));
        }

        Ok(DiskDeviceEnumerator {
            device_info_list,
            device_index: 0,
        })
    }
}

impl Drop for DiskDeviceEnumerator {
    fn drop(&mut self) {
        unsafe {
            SetupDiDestroyDeviceInfoList(self.device_info_list);
        }
    }
}

impl Iterator for DiskDeviceEnumerator {
    type Item = Vec<DiskPartitionInfo>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut device_interface_data = unsafe { mem::uninitialized::<SP_DEVICE_INTERFACE_DATA>() };
        device_interface_data.cbSize = mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as UINT;

        let result = unsafe {
            SetupDiEnumDeviceInterfaces(
                self.device_info_list,
                ptr::null_mut(),
                &GUID_DEVINTERFACE_DISK,
                self.device_index,
                &mut device_interface_data,
            )
        };

        self.device_index += 1;

        if result == 0 {
            return None;
        }

        let mut required_size: u32 = 0;

        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                self.device_info_list,
                &mut device_interface_data,
                ptr::null_mut(),
                0,
                &mut required_size,
                ptr::null_mut(),
            )
        };

        if required_size == 0 {
            return None;
        }

        let mut interface_details = DeviceInterfaceDetailData::new(required_size as usize).unwrap(); //todo: handle errors

        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                self.device_info_list,
                &mut device_interface_data,
                interface_details.get(),
                required_size,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };

        Some(
            DiskFileDevice::from_device_path(interface_details.path())
                .unwrap()
                .get_partitions()
                .unwrap(),
        )
    }
}

#[derive(Debug)]
pub struct DiskPartitionInfo {
    pub(crate) id: String,
    pub(crate) details: StorageDetails,
}

struct DiskFileDevice {
    handle: HANDLE,
}

#[repr(C)]
#[allow(dead_code)]
struct StorageDeviceNumber {
    device_type: u32,
    device_number: DWORD,
    partition_number: DWORD,
}

impl DiskFileDevice {
    fn from_device_path(path: String) -> Result<Self> {
        unsafe {
            let handle = winapi::um::fileapi::CreateFileW(
                widestring::WideCString::from_str(path.clone())
                    .unwrap()
                    .as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ,
                null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            );

            if handle == INVALID_HANDLE_VALUE {
                return Err(anyhow!(
                    "Cannot open device {}. Error: {}",
                    path,
                    get_last_error_str()
                ));
            }

            Ok(DiskFileDevice { handle })
        }
    }

    fn get_partitions(&self) -> Result<Vec<DiskPartitionInfo>> {
        let mut dev_number = StorageDeviceNumber {
            device_type: 0,
            device_number: 0,
            partition_number: 0,
        };

        let mut bytes: DWORD = 0;

        unsafe {
            if ioapiset::DeviceIoControl(
                self.handle,
                winapi::um::winioctl::IOCTL_STORAGE_GET_DEVICE_NUMBER,
                null_mut(),
                0,
                &mut dev_number as *mut _ as LPVOID,
                std::mem::size_of::<StorageDeviceNumber>() as DWORD,
                &mut bytes,
                null_mut(),
            ) == 0
            {
                return Err(anyhow!(
                    "Unable to get device number. Error: {}",
                    get_last_error_str()
                ));
            }
        }
        Ok(vec![DiskPartitionInfo {
            id: format!("\\\\?\\PhysicalDrive{}", dev_number.device_number),
            details: Default::default(),
        }])
    }
}

impl Drop for DiskFileDevice {
    fn drop(&mut self) {
        if self.handle != null_mut() {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

fn get_last_error_str() -> String {
    use winapi::um::winbase::{
        FormatMessageW, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
        FORMAT_MESSAGE_IGNORE_INSERTS,
    };

    let mut buffer: LPWSTR = ptr::null_mut();
    unsafe {
        let strlen = FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_ALLOCATE_BUFFER
                | FORMAT_MESSAGE_IGNORE_INSERTS,
            std::ptr::null(),
            GetLastError(),
            0,
            (&mut buffer as *mut LPWSTR) as LPWSTR,
            0,
            std::ptr::null_mut(),
        );
        let widestr = widestring::WideString::from_ptr(buffer, strlen as usize);
        LocalFree(buffer as HLOCAL);
        widestr.to_string_lossy()
    }
}
