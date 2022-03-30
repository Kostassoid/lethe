extern crate winapi;

use std::slice;
use std::{io, mem, ptr};

use anyhow::{Context, Result};
use libc;
use widestring::WideCString;
use winapi::_core::ptr::null_mut;
use winapi::shared::minwindef::*;
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::setupapi::*;
use winapi::um::winioctl::GUID_DEVINTERFACE_DISK;
use winapi::um::winnt::{PVOID, WCHAR};
use winapi::um::{fileapi, ioapiset, winioctl};

use windows::access::*;

use crate::storage::*;

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

pub struct DeviceInterfaceDetailData {
    data: PSP_DEVICE_INTERFACE_DETAIL_DATA_W,
    path_len: usize,
}

struct VolumeExtent {
    device_number: u32,
    starting_offset: u64,
}

struct VolumeDetails {
    extents: Vec<VolumeExtent>,
    label: Option<String>,
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
        #[allow(deref_nullptr)]
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
        unsafe {
            WideCString::from_ptr((*self.data).DevicePath.as_ptr(), (self.path_len / 2) - 1)
                .unwrap()
                .to_string_lossy()
        }
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
    volumes: Vec<(String, VolumeDetails)>,
}

impl DiskDeviceEnumerator {
    pub fn new() -> Result<Self> {
        let volumes = get_volumes()?;

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
            volumes,
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
    type Item = StorageRef;

    fn next(&mut self) -> Option<Self::Item> {
        let mut device_interface_data: SP_DEVICE_INTERFACE_DATA = unsafe { mem::zeroed() };
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

        let interface_details = DeviceInterfaceDetailData::new(required_size as usize).unwrap(); //todo: handle errors

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

        DeviceFile::open(interface_details.path().as_str(), false)
            .and_then(|d| get_device_number(&d))
            .and_then(PhysicalDrive::from_device_number)
            .and_then(|p| p.describe(&self.volumes))
            .ok()
            .or_else(|| self.next()) // skip
    }
}

impl PhysicalDrive {
    fn from_device_number(device_number: u32) -> Result<Self> {
        let disk_path = format!("\\\\.\\PhysicalDrive{}", device_number);
        let device = DeviceFile::open(disk_path.as_str(), false)?;
        Ok(PhysicalDrive {
            device_number,
            path: disk_path,
            device,
        })
    }

    fn describe(&self, volumes: &Vec<(String, VolumeDetails)>) -> Result<StorageRef> {
        let geometry = get_drive_geometry(&self.device)?;
        let bytes_per_sector = get_alignment_descriptor(&self.device)
            .map(|a| a.BytesPerPhysicalSector as usize)
            .unwrap_or(geometry.Geometry.BytesPerSector as usize);

        let storage_type = match geometry.Geometry.MediaType {
            winioctl::RemovableMedia => StorageType::Removable,
            winioctl::FixedMedia => StorageType::Fixed,
            _ => StorageType::Other,
        };

        let drive_details = StorageDetails {
            size: unsafe { *geometry.DiskSize.QuadPart() as u64 },
            block_size: bytes_per_sector,
            storage_type,
            mount_point: None,
            label: None,
        };

        let layout = get_drive_layout(&self.device)?;

        let mut devices: Vec<StorageRef> = Vec::new();

        let partitions = unsafe {
            slice::from_raw_parts(
                layout.info.PartitionEntry.as_ptr(),
                layout.info.PartitionCount as usize,
            )
        };

        for i in 0..layout.info.PartitionCount {
            let x = partitions[i as usize];
            let l = unsafe { *x.PartitionLength.QuadPart() };

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

            let mount_point_and_label = volumes
                .iter()
                .find(|v| {
                    v.1.extents.iter().any(|e| unsafe {
                        e.device_number == self.device_number
                            && e.starting_offset == *x.StartingOffset.QuadPart() as u64
                    })
                })
                .map(|v| (v.0.clone(), v.1.label.clone()));

            devices.push(StorageRef {
                id: partition_path,
                details: StorageDetails {
                    size: l as u64,
                    block_size: drive_details.block_size,
                    storage_type: StorageType::Partition,
                    mount_point: mount_point_and_label.iter().map(|v| v.0.clone()).next(),
                    label: mount_point_and_label
                        .iter()
                        .flat_map(|v| v.1.clone())
                        .next(),
                },
                children: vec![],
            })
        }

        let root = StorageRef {
            id: self.path.to_string(),
            details: drive_details,
            children: devices,
        };

        Ok(root)
    }
}

fn get_drive_layout(device: &DeviceFile) -> Result<&mut Layout> {
    const LAYOUT_BUFFER_SIZE: usize = std::mem::size_of::<Layout>();
    let mut layout_buffer: [BYTE; LAYOUT_BUFFER_SIZE] = [0; LAYOUT_BUFFER_SIZE];
    let mut bytes: DWORD = 0;
    unsafe {
        let layout: &mut Layout = std::mem::transmute(layout_buffer.as_mut_ptr());

        if ioapiset::DeviceIoControl(
            device.handle,
            winioctl::IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
            std::ptr::null_mut(),
            0,
            layout_buffer.as_mut_ptr() as PVOID,
            LAYOUT_BUFFER_SIZE as DWORD,
            &mut bytes,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error()).context("Unable to get device layout.");
        }
        Ok(layout)
    }
}

fn get_volume_extents(device: &DeviceFile) -> Result<Vec<VolumeExtent>> {
    const EXTENTS_BUFFER_SIZE: usize =
        16 + std::mem::size_of::<winioctl::VOLUME_DISK_EXTENTS>() * 32;
    let mut extents_buffer: [BYTE; EXTENTS_BUFFER_SIZE] = [0; EXTENTS_BUFFER_SIZE];
    let mut bytes: DWORD = 0;
    unsafe {
        let extents: &mut winioctl::VOLUME_DISK_EXTENTS =
            std::mem::transmute(extents_buffer.as_mut_ptr());

        if ioapiset::DeviceIoControl(
            device.handle,
            winioctl::IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            std::ptr::null_mut(),
            0,
            extents_buffer.as_mut_ptr() as PVOID,
            EXTENTS_BUFFER_SIZE as DWORD,
            &mut bytes,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error()).context("Unable to get volume extents.");
        }

        let mut r: Vec<VolumeExtent> = Vec::new();
        let ex = slice::from_raw_parts(
            extents.Extents.as_ptr(),
            extents.NumberOfDiskExtents as usize,
        );

        for i in 0..extents.NumberOfDiskExtents as usize {
            r.push(VolumeExtent {
                device_number: ex[i].DiskNumber,
                starting_offset: *ex[i].StartingOffset.QuadPart() as u64,
            });
        }

        Ok(r)
    }
}

fn get_drive_geometry(device: &DeviceFile) -> Result<winioctl::DISK_GEOMETRY_EX> {
    let mut bytes: DWORD = 0;
    unsafe {
        let mut geometry: winioctl::DISK_GEOMETRY_EX = mem::zeroed();
        if ioapiset::DeviceIoControl(
            device.handle,
            winioctl::IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
            std::ptr::null_mut(),
            0,
            &mut geometry as *mut _ as PVOID,
            std::mem::size_of::<winioctl::DISK_GEOMETRY_EX>() as DWORD,
            &mut bytes,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error()).context("Unable to get device geometry.");
        }
        Ok(geometry)
    }
}

fn get_volumes() -> Result<Vec<(String, VolumeDetails)>> {
    let drives = unsafe { fileapi::GetLogicalDrives() };
    let mut volumes: Vec<(String, VolumeDetails)> = Vec::new();

    for c in b'A'..b'Z' + 1 {
        if drives & (1 << (c - b'A') as u32) != 0 {
            let device_path = format!("{}:\\", c as char);
            let volume_path = match get_volume_path_from_mount_point(device_path.as_str()) {
                Ok(x) => x,
                _ => continue,
            };
            let volume_label = get_volume_label(device_path.as_str()).ok();
            DeviceFile::open(volume_path.as_str(), false)
                .and_then(|d| get_volume_extents(&d))
                .map(|e| {
                    volumes.push((
                        device_path,
                        VolumeDetails {
                            label: volume_label,
                            extents: e,
                        },
                    ))
                })
                .unwrap_or_default();
        }
    }

    Ok(volumes)
}

fn get_device_number(device: &DeviceFile) -> Result<DWORD> {
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
            return Err(io::Error::last_os_error()).context("Unable to get device number.");
        }
    }

    Ok(dev_number.device_number)
}

//todo: don't clone
fn normalize_volume_path(path: &str) -> String {
    let mut p = path.to_string();
    p.shrink_to_fit();

    if p.chars().last().unwrap() == '\\' {
        p.pop();
    }

    p
}

fn get_volume_path_from_mount_point(path: &str) -> Result<String> {
    const MAX_PATH: usize = 1024;
    let mut volume_name_buffer: [WCHAR; MAX_PATH] = [0; MAX_PATH];
    unsafe {
        if fileapi::GetVolumeNameForVolumeMountPointW(
            WideCString::from_str(path.clone()).unwrap().as_ptr(),
            volume_name_buffer.as_mut_ptr(),
            MAX_PATH as DWORD,
        ) == 0
        {
            return Err(io::Error::last_os_error())
                .context(format!("Unable to get volume path from {}.", path));
        }
    }

    let full_volume_path =
        unsafe { WideCString::from_ptr_str(volume_name_buffer.as_ptr()) }.to_string_lossy();

    Ok(normalize_volume_path(full_volume_path.as_str()))
}

fn get_volume_label(path: &str) -> Result<String> {
    const MAX_PATH: usize = 1024;
    let mut volume_name_buffer: [WCHAR; MAX_PATH] = [0; MAX_PATH];
    unsafe {
        if fileapi::GetVolumeInformationW(
            WideCString::from_str(path.clone()).unwrap().as_ptr(),
            volume_name_buffer.as_mut_ptr(),
            MAX_PATH as DWORD,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            0 as DWORD,
        ) == 0
        {
            return Err(io::Error::last_os_error())
                .context(format!("Unable to get volume label from {}.", path));
        }
    }

    let volume_label =
        unsafe { WideCString::from_ptr_str(volume_name_buffer.as_ptr()) }.to_string_lossy();

    Ok(volume_label)
}

winapi::STRUCT! {
    #[allow(non_snake_case)]
    #[derive(Debug)]
    struct STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR {
        Version: ULONG,
        Size: ULONG,
        BytesPerCacheLine: ULONG,
        BytesOffsetForCacheAlignment: ULONG,
        BytesPerLogicalSector: ULONG,
        BytesPerPhysicalSector: ULONG,
        BytesOffsetForSectorAlignment: ULONG,
    }
}

fn get_alignment_descriptor(device: &DeviceFile) -> Result<STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR> {
    let mut query = winioctl::STORAGE_PROPERTY_QUERY {
        PropertyId: winioctl::StorageAccessAlignmentProperty,
        QueryType: winioctl::PropertyStandardQuery,
        AdditionalParameters: [0],
    };

    let mut alignment: STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR = unsafe { mem::zeroed() };
    let mut bytes: DWORD = 0;
    unsafe {
        if ioapiset::DeviceIoControl(
            device.handle,
            winioctl::IOCTL_STORAGE_QUERY_PROPERTY,
            &mut query as *mut _ as PVOID,
            mem::size_of_val(&query) as DWORD,
            &mut alignment as *mut _ as PVOID,
            mem::size_of_val(&alignment) as DWORD,
            &mut bytes,
            ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error()).context("Unable to get alignment info.");
        }
    }

    Ok(alignment)
}
