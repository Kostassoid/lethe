use std::mem;
use std::slice;

use anyhow::{Context, Result};
use windows::Win32::Storage::FileSystem::{
    BusTypeAta, BusTypeNvme, BusTypeSas, BusTypeSata, BusTypeScsi, BusTypeUnknown, BusTypeUsb,
    BusTypeVirtual,
};
use windows::Win32::System::Ioctl::{
    DEVICE_SEEK_PENALTY_DESCRIPTOR, DEVICE_TRIM_DESCRIPTOR, DISK_GEOMETRY_EX,
    DRIVE_LAYOUT_INFORMATION_EX, FixedMedia, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
    IOCTL_DISK_GET_DRIVE_LAYOUT_EX, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY,
    PARTITION_INFORMATION_EX, PARTITION_STYLE_GPT, PARTITION_STYLE_MBR, PropertyStandardQuery,
    RemovableMedia, STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR, STORAGE_ADAPTER_DESCRIPTOR,
    STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY, StorageAccessAlignmentProperty,
    StorageAdapterProperty, StorageDeviceSeekPenaltyProperty, StorageDeviceTrimProperty,
};

use crate::storage::{
    BusKind, DriveKind, StorageDetails, StorageReadiness, StorageRef, StorageType,
};

use super::access::{DeviceAccess, DeviceFile};
use super::volumes::Volume;

const MAX_PARTITIONS: usize = 128;

#[repr(C)]
struct DriveLayout {
    info: DRIVE_LAYOUT_INFORMATION_EX,
    extra: [PARTITION_INFORMATION_EX; MAX_PARTITIONS - 1],
}

impl DriveLayout {
    fn zeroed() -> Self {
        unsafe { mem::zeroed() }
    }
}

pub(super) struct PhysicalDrive {
    device_number: u32,
    path: String,
    device: DeviceFile,
}

impl PhysicalDrive {
    pub(super) fn from_device_number(device_number: u32) -> Result<Self> {
        let path = format!("\\\\.\\PhysicalDrive{device_number}");
        let device = DeviceFile::open(&path, DeviceAccess::Query)?;

        Ok(PhysicalDrive {
            device_number,
            path,
            device,
        })
    }

    pub(super) fn describe(&self, volumes: &[Volume]) -> Result<StorageRef> {
        let geometry = get_drive_geometry(&self.device)?;
        let bytes_per_sector = get_alignment_descriptor(&self.device)
            .map_or(geometry.Geometry.BytesPerSector as usize, |descriptor| {
                descriptor.BytesPerPhysicalSector as usize
            });

        let storage_type = if geometry.Geometry.MediaType == RemovableMedia {
            StorageType::Removable
        } else if geometry.Geometry.MediaType == FixedMedia {
            StorageType::Fixed
        } else {
            StorageType::Other
        };

        let (bus, drive_kind) = detect_drive(&self.device);

        let drive_details = StorageDetails {
            size: geometry.DiskSize.cast_unsigned(),
            block_size: bytes_per_sector,
            storage_type,
            mount_point: None,
            label: None,
            drive_kind,
            bus,
        };

        let layout = get_drive_layout(&self.device)?;
        let partitions = unsafe {
            slice::from_raw_parts(
                layout.info.PartitionEntry.as_ptr(),
                layout.info.PartitionCount as usize,
            )
        };

        let mut devices = Vec::new();

        for partition in partitions {
            if !is_used_partition(partition) {
                continue;
            }

            let partition_path = format!(
                "\\Device\\Harddisk{}\\Partition{}",
                self.device_number, partition.PartitionNumber
            );

            let mount_point_and_label = volumes
                .iter()
                .find(|volume| {
                    volume.extents.iter().any(|extent| {
                        extent.device_number == self.device_number
                            && extent.starting_offset == partition.StartingOffset.cast_unsigned()
                    })
                })
                .map(|volume| (volume.mount_point.clone(), volume.label.clone()));

            devices.push(StorageRef {
                id: partition_path,
                readiness: StorageReadiness::Ready(StorageDetails {
                    size: partition.PartitionLength.cast_unsigned(),
                    block_size: drive_details.block_size,
                    storage_type: StorageType::Partition,
                    mount_point: mount_point_and_label.as_ref().map(|value| value.0.clone()),
                    label: mount_point_and_label
                        .as_ref()
                        .and_then(|value| value.1.clone()),
                    drive_kind,
                    bus,
                }),
                children: vec![],
            });
        }

        Ok(StorageRef {
            id: self.path.clone(),
            readiness: StorageReadiness::Ready(drive_details),
            children: devices,
        })
    }
}

fn is_used_partition(partition: &PARTITION_INFORMATION_EX) -> bool {
    if partition.PartitionStyle == PARTITION_STYLE_MBR {
        unsafe { partition.Anonymous.Mbr.PartitionType != 0 }
    } else if partition.PartitionStyle == PARTITION_STYLE_GPT {
        unsafe { partition.Anonymous.Gpt.PartitionType.data1 != 0 }
    } else {
        false
    }
}

pub(super) fn get_device_number(device: &DeviceFile) -> Result<u32> {
    let number: STORAGE_DEVICE_NUMBER = device
        .ioctl(IOCTL_STORAGE_GET_DEVICE_NUMBER)
        .context("Unable to get device number.")?;

    Ok(number.DeviceNumber)
}

fn get_drive_geometry(device: &DeviceFile) -> Result<DISK_GEOMETRY_EX> {
    device
        .ioctl(IOCTL_DISK_GET_DRIVE_GEOMETRY_EX)
        .context("Unable to get device geometry.")
}

fn get_drive_layout(device: &DeviceFile) -> Result<DriveLayout> {
    let mut layout = DriveLayout::zeroed();

    device
        .ioctl_into(
            IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
            Option::<&()>::None,
            &mut layout,
        )
        .context("Unable to get device layout.")?;

    Ok(layout)
}

fn get_alignment_descriptor(device: &DeviceFile) -> Result<STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageAccessAlignmentProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };

    device
        .ioctl_with_input(IOCTL_STORAGE_QUERY_PROPERTY, &query)
        .context("Unable to get alignment info.")
}

fn get_seek_penalty(device: &DeviceFile) -> Result<DEVICE_SEEK_PENALTY_DESCRIPTOR> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceSeekPenaltyProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };

    device
        .ioctl_with_input(IOCTL_STORAGE_QUERY_PROPERTY, &query)
        .context("Unable to query the device seek penalty.")
}

fn get_trim(device: &DeviceFile) -> Result<DEVICE_TRIM_DESCRIPTOR> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceTrimProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };

    device
        .ioctl_with_input(IOCTL_STORAGE_QUERY_PROPERTY, &query)
        .context("Unable to query the device trim property.")
}

fn get_adapter_descriptor(device: &DeviceFile) -> Result<STORAGE_ADAPTER_DESCRIPTOR> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageAdapterProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };

    device
        .ioctl_with_input(IOCTL_STORAGE_QUERY_PROPERTY, &query)
        .context("Unable to query the storage adapter.")
}

fn bus_kind(bus: u8) -> BusKind {
    let bus = i32::from(bus);

    if bus == BusTypeNvme.0 {
        BusKind::Nvme
    } else if bus == BusTypeUsb.0 {
        BusKind::Usb
    } else if bus == BusTypeSata.0 || bus == BusTypeAta.0 {
        BusKind::Sata
    } else if bus == BusTypeScsi.0 || bus == BusTypeSas.0 {
        BusKind::Scsi
    } else if bus == BusTypeVirtual.0 {
        BusKind::Virtual
    } else if bus == BusTypeUnknown.0 {
        BusKind::Unknown
    } else {
        BusKind::Other
    }
}

fn detect_drive(device: &DeviceFile) -> (BusKind, DriveKind) {
    let adapter = get_adapter_descriptor(device).ok();
    let bus_type = adapter.as_ref().map(|descriptor| descriptor.BusType);
    let bus = bus_type.map_or(BusKind::Unknown, bus_kind);
    let queueing = adapter
        .as_ref()
        .is_some_and(|descriptor| descriptor.CommandQueueing);
    let seek_penalty = get_seek_penalty(device)
        .ok()
        .map(|descriptor| descriptor.IncursSeekPenalty);
    let trim = get_trim(device)
        .ok()
        .map(|descriptor| descriptor.TrimEnabled);

    if bus_type.is_some_and(|bus| i32::from(bus) == BusTypeNvme.0) {
        return (bus, DriveKind::NvmeSsd);
    }

    if bus_type.is_some_and(|bus| i32::from(bus) == BusTypeUsb.0) {
        // Bulk-only transport cannot keep commands in flight, even on an SSD.
        if !queueing {
            return (bus, DriveKind::Removable);
        }

        // Many USB-SATA bridges do not report seek penalty, but still pass TRIM
        // through, which is a reliable hint that the media is solid state.
        let kind = match (seek_penalty, trim) {
            (Some(true), _) => DriveKind::Hdd,
            (Some(false), _) | (None, Some(true)) => DriveKind::Ssd,
            (None, _) => DriveKind::Removable,
        };

        return (bus, kind);
    }

    let kind = match (seek_penalty, trim) {
        (Some(true), _) => DriveKind::Hdd,
        (Some(false), _) | (None, Some(true)) => DriveKind::Ssd,
        (None, _) => DriveKind::Unknown,
    };

    (bus, kind)
}
