use libc;

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::slice;
use std::{mem, ptr};

use crate::storage::*;
use anyhow::Result;

extern crate winapi;
use crate::storage::windows::DeviceAccess;
use widestring::WideCString;
use winapi::_core::cmp::Ordering;
use winapi::_core::ptr::{null, null_mut};
use winapi::shared::minwindef::*;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::OPEN_EXISTING;
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::setupapi::*;
use winapi::um::winbase::{FormatMessageW, LocalFree};
use winapi::um::winioctl::GUID_DEVINTERFACE_DISK;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, GENERIC_READ, HANDLE, LPWSTR, PVOID, WCHAR,
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
    type Item = Vec<DiskDeviceInfo>;

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

        let device_number = get_device_number(interface_details.path().as_str()).unwrap();

        Some(
            PhysicalDrive::from_device_number(device_number)
                .unwrap()
                .get_storage_list()
                .unwrap(),
        )
    }
}

#[derive(Debug)]
pub struct DiskDeviceInfo {
    pub id: String,
    pub details: StorageDetails,
}

#[derive(Debug)]
pub struct PartitionInfo {
    pub id: String,
    pub details: StorageDetails,
}

struct PhysicalDrive {
    device_number: DWORD,
    path: String,
    device: DeviceFile,
}

#[repr(C)]
#[allow(dead_code)]
struct StorageDeviceNumber {
    device_type: u32,
    device_number: DWORD,
    partition_number: DWORD,
}

#[repr(C)]
struct Layout {
    info: winioctl::DRIVE_LAYOUT_INFORMATION_EX,
    partitions: [winioctl::PARTITION_INFORMATION_EX; 100],
}

impl PhysicalDrive {
    fn from_device_number(device_number: u32) -> Result<Self> {
        let disk_path = format!("\\\\.\\PhysicalDrive{}", device_number);
        let device = DeviceFile::open(disk_path.as_str())?;
        Ok(PhysicalDrive {
            device_number,
            path: disk_path,
            device,
        })
    }

    fn get_storage_list(&self) -> Result<Vec<DiskDeviceInfo>> {
        let geometry = self.get_drive_geometry()?;
        let drive_details = StorageDetails {
            size: unsafe { *geometry.DiskSize.QuadPart() as u64 },
            block_size: geometry.Geometry.BytesPerSector as usize, //todo: this
            storage_type: StorageType::Drive,
            media_type: MediaType::Unknown,
            is_trim_supported: false,
            serial: None,
            mount_point: None,
        };

        let layout = self.get_drive_layout()?;

        let mut devices: Vec<DiskDeviceInfo> = Vec::new();

        let partitions = unsafe {
            slice::from_raw_parts(
                layout.info.PartitionEntry.as_ptr(),
                layout.info.PartitionCount as usize,
            )
        };

        for i in 0..layout.info.PartitionCount {
            let x = partitions[i as usize];
            let l = unsafe { *x.PartitionLength.QuadPart() };
            let so = unsafe { *x.StartingOffset.QuadPart() };

            match x.PartitionStyle {
                winioctl::PARTITION_STYLE_MBR => unsafe {
                    if x.u.Mbr().PartitionType == 0 {
                        continue;
                    }
                },
                winioctl::PARTITION_STYLE_GPT => unsafe {
                    if x.u.Gpt().PartitionType.Data1 == 0 {
                        continue;
                    }
                },
                _ => continue,
            }

            let partition_path = format!(
                "\\Device\\Harddisk{}\\Partition{}",
                self.device_number, x.PartitionNumber
            );
            devices.push(DiskDeviceInfo {
                id: partition_path,
                details: StorageDetails {
                    size: l as u64,
                    block_size: geometry.Geometry.BytesPerSector as usize, //todo: figure out
                    storage_type: StorageType::Partition,
                    media_type: MediaType::Unknown,
                    is_trim_supported: false,
                    serial: None,
                    mount_point: None,
                },
            })
        }

        devices.push(DiskDeviceInfo {
            id: self.path.to_string(),
            details: drive_details,
        });

        Ok(devices)
    }

    fn get_drive_layout(&self) -> Result<&mut Layout> {
        const LAYOUT_BUFFER_SIZE: usize = std::mem::size_of::<Layout>();
        let mut layout_buffer: [BYTE; LAYOUT_BUFFER_SIZE] = [0; LAYOUT_BUFFER_SIZE];
        let mut bytes: DWORD = 0;
        unsafe {
            let layout: &mut Layout = std::mem::transmute(layout_buffer.as_mut_ptr());

            if ioapiset::DeviceIoControl(
                self.device.handle,
                winioctl::IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
                std::ptr::null_mut(),
                0,
                layout_buffer.as_mut_ptr() as PVOID,
                LAYOUT_BUFFER_SIZE as DWORD,
                &mut bytes,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(anyhow!(
                    "Unable to get device layout. Error: {}",
                    get_last_error_str()
                ));
            }
            Ok(layout)
        }
    }

    fn get_drive_geometry(&self) -> Result<winioctl::DISK_GEOMETRY_EX> {
        let mut bytes: DWORD = 0;
        unsafe {
            let mut geometry = unsafe { mem::uninitialized::<winioctl::DISK_GEOMETRY_EX>() };
            if ioapiset::DeviceIoControl(
                self.device.handle,
                winioctl::IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
                std::ptr::null_mut(),
                0,
                &mut geometry as *mut _ as PVOID,
                std::mem::size_of::<winioctl::DISK_GEOMETRY_EX>() as DWORD,
                &mut bytes,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(anyhow!(
                    "Unable to get device geometry. Error: {}",
                    get_last_error_str()
                ));
            }
            Ok(geometry)
        }
    }
}

pub(crate) fn enumerate_volumes() -> Result<Vec<DiskDeviceInfo>> {
    let mut devices: Vec<DiskDeviceInfo> = Vec::new();

    const MAX_PATH: usize = 256;
    let mut volume_name_buffer: [WCHAR; MAX_PATH] = [0; MAX_PATH];
    let find_volume_handle =
        unsafe { fileapi::FindFirstVolumeW(volume_name_buffer.as_mut_ptr(), MAX_PATH as DWORD) };

    if find_volume_handle == std::ptr::null_mut() {
        return Err(anyhow!(
            "Unable to get volumes. Error: {}",
            get_last_error_str()
        ));
    }

    loop {
        let volume_name_wstr =
            unsafe { widestring::WideCString::from_ptr_str(volume_name_buffer.as_ptr()) };
        let mut volume_name = volume_name_wstr.to_string_lossy();
        volume_name.shrink_to_fit();

        if volume_name.chars().last().unwrap() == '\\' {
            volume_name.pop();
        }

        println!("Looking for {}", unsafe { &volume_name[4..] });

        let mut device_name_buffer: [WCHAR; MAX_PATH] = [0; MAX_PATH];
        let result = unsafe {
            fileapi::QueryDosDeviceW(
                widestring::WideCString::from_str(&volume_name[4..])
                    .unwrap()
                    .as_ptr(),
                device_name_buffer.as_mut_ptr(),
                MAX_PATH as DWORD,
            )
        };

        devices.push(DiskDeviceInfo {
            id: unsafe { WideCString::from_ptr_str(device_name_buffer.as_ptr()).to_string_lossy() },
            details: Default::default(),
        });

        unsafe {
            if fileapi::FindNextVolumeW(
                find_volume_handle,
                volume_name_buffer.as_mut_ptr(),
                MAX_PATH as DWORD,
            ) == 0
            {
                break Ok(devices);
            }
        }
    }
}

struct DeviceFile {
    handle: HANDLE,
}

impl DeviceFile {
    fn open(path: &str) -> Result<Self> {
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

            Ok(DeviceFile { handle })
        }
    }
}

impl Drop for DeviceFile {
    fn drop(&mut self) {
        if self.handle != null_mut() {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

fn get_device_number(path: &str) -> Result<DWORD> {
    let device = DeviceFile::open(path)?;

    let mut dev_number = StorageDeviceNumber {
        device_type: 0,
        device_number: 0,
        partition_number: 0,
    };

    let mut bytes: DWORD = 0;

    unsafe {
        if ioapiset::DeviceIoControl(
            device.handle,
            winioctl::IOCTL_STORAGE_GET_DEVICE_NUMBER,
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

    Ok(dev_number.device_number)
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
