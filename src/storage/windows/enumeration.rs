use std::mem::{offset_of, size_of};
use std::slice;

use anyhow::{Context, Result, anyhow};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::System::Ioctl::GUID_DEVINTERFACE_DISK;
use windows::core::PCWSTR;

use crate::storage::{StorageReadiness, StorageRef};

use super::access::{DeviceAccess, DeviceFile};
use super::physical_drive::{PhysicalDrive, get_device_number};
use super::volumes::{Volume, get_volumes};

struct DeviceInterfaceDetail {
    storage: Vec<u64>,
    size: usize,
}

impl DeviceInterfaceDetail {
    fn new(size: usize) -> Result<Self> {
        let header_size = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();

        if size < header_size {
            return Err(anyhow!("DeviceInterfaceDetailData is too small. {size}"));
        }

        let mut storage = vec![0u64; size.div_ceil(size_of::<u64>())];
        let data = storage
            .as_mut_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();

        unsafe {
            (*data).cbSize =
                u32::try_from(header_size).expect("device interface header is too large");
        }

        Ok(DeviceInterfaceDetail { storage, size })
    }

    fn as_mut_ptr(&mut self) -> *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W {
        self.storage
            .as_mut_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>()
    }

    fn path(&self) -> String {
        let data = self
            .storage
            .as_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        let offset = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
        let length = self.size.saturating_sub(offset) / size_of::<u16>();
        let device_path = unsafe { (*data).DevicePath.as_ptr() };
        let path = unsafe { slice::from_raw_parts(device_path, length) };
        let length = path.iter().position(|value| *value == 0).unwrap_or(length);

        String::from_utf16_lossy(&path[..length])
    }
}

pub(super) struct DiskDeviceEnumerator {
    device_info_list: HDEVINFO,
    device_index: u32,
    volumes: Vec<Volume>,
}

impl DiskDeviceEnumerator {
    pub(super) fn new() -> Result<Self> {
        let volumes = get_volumes();
        let interface_guid = GUID_DEVINTERFACE_DISK;

        let device_info_list = unsafe {
            SetupDiGetClassDevsW(
                Some(&raw const interface_guid),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        }
        .context("Unable to initialize disk device enumeration")?;

        Ok(DiskDeviceEnumerator {
            device_info_list,
            device_index: 0,
            volumes,
        })
    }
}

impl Drop for DiskDeviceEnumerator {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.device_info_list);
        }
    }
}

impl Iterator for DiskDeviceEnumerator {
    type Item = StorageRef;

    fn next(&mut self) -> Option<Self::Item> {
        let interface_guid = GUID_DEVINTERFACE_DISK;
        let mut interface_data = SP_DEVICE_INTERFACE_DATA {
            cbSize: u32::try_from(size_of::<SP_DEVICE_INTERFACE_DATA>())
                .expect("device interface data is too large"),
            ..Default::default()
        };

        let result = unsafe {
            SetupDiEnumDeviceInterfaces(
                self.device_info_list,
                None,
                &raw const interface_guid,
                self.device_index,
                &raw mut interface_data,
            )
        };

        self.device_index += 1;

        if result.is_err() {
            return None;
        }

        let mut required_size = 0u32;

        unsafe {
            let _ = SetupDiGetDeviceInterfaceDetailW(
                self.device_info_list,
                &raw const interface_data,
                None,
                0,
                Some(&raw mut required_size),
                None,
            );
        }

        if required_size == 0 {
            return None;
        }

        let mut details = DeviceInterfaceDetail::new(
            usize::try_from(required_size).expect("device interface detail size is too large"),
        )
        .ok()?;

        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                self.device_info_list,
                &raw const interface_data,
                Some(details.as_mut_ptr()),
                required_size,
                Some(&raw mut required_size),
                None,
            )
        }
        .ok()?;

        let path = details.path();

        Some(
            DeviceFile::open(&path, DeviceAccess::Query)
                .and_then(|device| get_device_number(&device))
                .and_then(PhysicalDrive::from_device_number)
                .and_then(|drive| drive.describe(&self.volumes))
                .unwrap_or_else(|_| StorageRef {
                    id: path,
                    readiness: StorageReadiness::Locked,
                    children: vec![],
                }),
        )
    }
}
