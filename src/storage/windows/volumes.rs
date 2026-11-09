use std::mem;
use std::slice;

use anyhow::{Context, Result};
use windows::Win32::Storage::FileSystem::{
    GetLogicalDrives, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW,
    IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
};
use windows::Win32::System::Ioctl::{DISK_EXTENT, VOLUME_DISK_EXTENTS};
use windows::core::HSTRING;

use super::access::{DeviceAccess, DeviceFile};

const MAX_PATH: usize = 1024;
const MAX_VOLUME_EXTENTS: usize = 32;

pub(super) struct VolumeExtent {
    pub device_number: u32,
    pub starting_offset: u64,
}

pub(super) struct Volume {
    pub mount_point: String,
    pub label: Option<String>,
    pub extents: Vec<VolumeExtent>,
}

#[repr(C)]
struct VolumeDiskExtents {
    info: VOLUME_DISK_EXTENTS,
    extra: [DISK_EXTENT; MAX_VOLUME_EXTENTS - 1],
}

impl VolumeDiskExtents {
    fn zeroed() -> Self {
        unsafe { mem::zeroed() }
    }
}

pub(super) fn get_volumes() -> Vec<Volume> {
    let drives = unsafe { GetLogicalDrives() };
    let mut volumes = Vec::new();

    for letter in b'A'..=b'Z' {
        let drive_index = u32::from(letter - b'A');
        if drives & (1 << drive_index) == 0 {
            continue;
        }

        let mount_point = format!("{}:\\", letter as char);
        let Ok(volume_path) = get_volume_path(&mount_point) else {
            continue;
        };

        let label = get_volume_label(&mount_point).ok();

        let Ok(device) = DeviceFile::open(&volume_path, DeviceAccess::Query) else {
            continue;
        };

        let Ok(extents) = get_volume_extents(&device) else {
            continue;
        };

        volumes.push(Volume {
            mount_point,
            label,
            extents,
        });
    }

    volumes
}

fn get_volume_extents(device: &DeviceFile) -> Result<Vec<VolumeExtent>> {
    let mut buffer = VolumeDiskExtents::zeroed();

    device
        .ioctl_into(
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            Option::<&()>::None,
            &mut buffer,
        )
        .context("Unable to get volume extents.")?;

    let count = buffer.info.NumberOfDiskExtents as usize;
    let extents = unsafe { slice::from_raw_parts(buffer.info.Extents.as_ptr(), count) };

    Ok(extents
        .iter()
        .map(|extent| VolumeExtent {
            device_number: extent.DiskNumber,
            starting_offset: extent.StartingOffset.cast_unsigned(),
        })
        .collect())
}

fn get_volume_path(mount_point: &str) -> Result<String> {
    let mut buffer = [0u16; MAX_PATH];
    let name = HSTRING::from(mount_point);

    unsafe { GetVolumeNameForVolumeMountPointW(&name, &mut buffer) }
        .with_context(|| format!("Unable to get volume path from {mount_point}."))?;

    Ok(normalize_volume_path(&utf16_to_string(&buffer)))
}

fn get_volume_label(mount_point: &str) -> Result<String> {
    let mut buffer = [0u16; MAX_PATH];
    let name = HSTRING::from(mount_point);

    unsafe { GetVolumeInformationW(&name, Some(&mut buffer), None, None, None, None) }
        .with_context(|| format!("Unable to get volume label from {mount_point}."))?;

    Ok(utf16_to_string(&buffer))
}

fn normalize_volume_path(path: &str) -> String {
    path.strip_suffix('\\').unwrap_or(path).to_string()
}

fn utf16_to_string(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}
