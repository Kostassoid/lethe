use std::ffi::c_void;
use std::mem;
use std::slice;

use anyhow::{Context, Result, anyhow};
use windows::Win32::Storage::FileSystem::{
    BusTypeAta, BusTypeNvme, BusTypeSas, BusTypeSata, BusTypeScsi, BusTypeUnknown, BusTypeUsb,
    BusTypeVirtual,
};
use windows::Win32::Storage::IscsiDisc::{
    IOCTL_SCSI_PASS_THROUGH_DIRECT, SCSI_PASS_THROUGH_DIRECT,
};
use windows::Win32::Storage::Nvme::NVME_IDENTIFY_CONTROLLER_DATA;
use windows::Win32::System::Ioctl::{
    AtaDataTypeIdentify, DEVICE_SEEK_PENALTY_DESCRIPTOR, DEVICE_TRIM_DESCRIPTOR, DISK_GEOMETRY_EX,
    DRIVE_LAYOUT_INFORMATION_EX, FixedMedia, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
    IOCTL_DISK_GET_DRIVE_LAYOUT_EX, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY,
    NVMeDataTypeIdentify, PARTITION_INFORMATION_EX, PARTITION_STYLE_GPT, PARTITION_STYLE_MBR,
    PropertyStandardQuery, ProtocolTypeAta, ProtocolTypeNvme, RemovableMedia,
    STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR, STORAGE_ADAPTER_DESCRIPTOR, STORAGE_DEVICE_NUMBER,
    STORAGE_PROPERTY_QUERY, STORAGE_PROTOCOL_DATA_DESCRIPTOR, STORAGE_PROTOCOL_SPECIFIC_DATA,
    STORAGE_PROTOCOL_TYPE, StorageAccessAlignmentProperty, StorageAdapterProperty,
    StorageDeviceProtocolSpecificProperty, StorageDeviceSeekPenaltyProperty,
    StorageDeviceTrimProperty,
};

use crate::sanitization::mem::AlignedBuffer;
use crate::storage::{
    BusKind, DriveKind, HardwareEraseCapabilities, StorageDetails, StorageReadiness, StorageRef,
    StorageType,
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
        let detection_device = DeviceFile::open(&self.path, DeviceAccess::QueryIo).ok();
        let hardware_erase =
            detect_hardware_erase(detection_device.as_ref().unwrap_or(&self.device), bus);

        let drive_details = StorageDetails {
            size: geometry.DiskSize.cast_unsigned(),
            block_size: bytes_per_sector,
            storage_type,
            mount_point: None,
            label: None,
            drive_kind,
            bus,
            hardware_erase,
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
                    hardware_erase,
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

fn query_protocol_data(
    device: &DeviceFile,
    protocol_type: STORAGE_PROTOCOL_TYPE,
    data_type: u32,
    request_value: u32,
    length: usize,
) -> Result<Vec<u8>> {
    let query_offset = std::mem::offset_of!(STORAGE_PROPERTY_QUERY, AdditionalParameters);
    let protocol_size = std::mem::size_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>();
    let descriptor_size = std::mem::size_of::<STORAGE_PROTOCOL_DATA_DESCRIPTOR>();

    let mut input = vec![0u8; query_offset + protocol_size];
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProtocolSpecificProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    unsafe {
        std::ptr::write_unaligned(input.as_mut_ptr().cast(), query);
    }

    let protocol = STORAGE_PROTOCOL_SPECIFIC_DATA {
        ProtocolType: protocol_type,
        DataType: data_type,
        ProtocolDataRequestValue: request_value,
        ProtocolDataRequestSubValue: 0,
        ProtocolDataOffset: u32::try_from(protocol_size).expect("protocol data size fits u32"),
        ProtocolDataLength: u32::try_from(length).expect("protocol data length fits u32"),
        FixedProtocolReturnData: 0,
        ProtocolDataRequestSubValue2: 0,
        ProtocolDataRequestSubValue3: 0,
        ProtocolDataRequestSubValue4: 0,
    };
    unsafe {
        std::ptr::write_unaligned(input.as_mut_ptr().add(query_offset).cast(), protocol);
    }

    let mut output = vec![0u8; descriptor_size + length];
    device.ioctl_raw(IOCTL_STORAGE_QUERY_PROPERTY, &input, &mut output)?;

    let descriptor: STORAGE_PROTOCOL_DATA_DESCRIPTOR =
        unsafe { std::ptr::read_unaligned(output.as_ptr().cast()) };
    let data_offset = descriptor.ProtocolSpecificData.ProtocolDataOffset as usize;
    let data_start =
        std::mem::offset_of!(STORAGE_PROTOCOL_DATA_DESCRIPTOR, ProtocolSpecificData) + data_offset;
    let data_end = data_start + length;

    if data_end > output.len() {
        return Err(anyhow!("Protocol data is outside the returned buffer."));
    }

    Ok(output[data_start..data_end].to_vec())
}

fn parse_ata_identify(data: &[u8]) -> HardwareEraseCapabilities {
    let word = |index: usize| u16::from_le_bytes([data[index * 2], data[index * 2 + 1]]);

    HardwareEraseCapabilities {
        ata_secure_erase: word(82) & (1 << 1) != 0,
        ata_enhanced_secure_erase: word(128) & (1 << 5) != 0,
        ata_sanitize: word(59) & (1 << 12) != 0,
        ..Default::default()
    }
}

fn detect_ata_capabilities(device: &DeviceFile) -> Result<HardwareEraseCapabilities> {
    let data = query_protocol_data(
        device,
        ProtocolTypeAta,
        u32::try_from(AtaDataTypeIdentify.0).expect("ATA data type is non-negative"),
        0,
        512,
    )?;

    Ok(parse_ata_identify(&data))
}

fn detect_ata_capabilities_scsi(device: &DeviceFile) -> Result<HardwareEraseCapabilities> {
    let mut data = AlignedBuffer::new(512, 512);
    data.as_mut_slice().fill(0);

    let mut cdb = [0u8; 16];
    cdb[0] = 0x85; // ATA PASS-THROUGH (16)
    cdb[1] = 4 << 1; // PIO Data-in
    cdb[2] = 0x2e; // CK_COND, T_DIR=1, BYT_BLOK=1, T_LENGTH=2
    cdb[6] = 1; // one 512-byte block
    cdb[14] = 0xec; // IDENTIFY DEVICE

    let header = SCSI_PASS_THROUGH_DIRECT {
        Length: u16::try_from(std::mem::size_of::<SCSI_PASS_THROUGH_DIRECT>())
            .expect("SCSI header fits u16"),
        ScsiStatus: 0,
        PathId: 0,
        TargetId: 0,
        Lun: 0,
        CdbLength: 16,
        SenseInfoLength: 0,
        DataIn: 1,
        DataTransferLength: 512,
        TimeOutValue: 10,
        DataBuffer: data.as_mut_slice().as_mut_ptr().cast::<c_void>(),
        SenseInfoOffset: 0,
        Cdb: cdb,
    };

    let header_size = std::mem::size_of::<SCSI_PASS_THROUGH_DIRECT>();
    let mut input = vec![0u8; header_size];
    unsafe {
        std::ptr::write_unaligned(input.as_mut_ptr().cast(), header);
    }

    let mut output = input.clone();
    device.ioctl_raw(IOCTL_SCSI_PASS_THROUGH_DIRECT, &input, &mut output)?;

    let result: SCSI_PASS_THROUGH_DIRECT =
        unsafe { std::ptr::read_unaligned(output.as_ptr().cast()) };

    if result.ScsiStatus != 0 {
        return Err(anyhow!(
            "SCSI ATA pass-through failed with status {}",
            result.ScsiStatus
        ));
    }

    Ok(parse_ata_identify(data.as_slice()))
}

fn parse_nvme_identify(data: &[u8]) -> HardwareEraseCapabilities {
    let identify: NVME_IDENTIFY_CONTROLLER_DATA =
        unsafe { std::ptr::read_unaligned(data.as_ptr().cast()) };

    HardwareEraseCapabilities {
        nvme_sanitize: identify.SANICAP._bitfield & 0b111 != 0,
        nvme_format: identify.OACS._bitfield & (1 << 1) != 0,
        ..Default::default()
    }
}

fn detect_nvme_capabilities(device: &DeviceFile) -> Result<HardwareEraseCapabilities> {
    let data = query_protocol_data(
        device,
        ProtocolTypeNvme,
        u32::try_from(NVMeDataTypeIdentify.0).expect("NVMe data type is non-negative"),
        1, // Identify Controller
        4096,
    )?;

    Ok(parse_nvme_identify(&data))
}

fn merge_capabilities(
    left: Option<HardwareEraseCapabilities>,
    right: Option<HardwareEraseCapabilities>,
) -> Option<HardwareEraseCapabilities> {
    match (left, right) {
        (None, None) => None,
        (Some(capabilities), None) | (None, Some(capabilities)) => Some(capabilities),
        (Some(left), Some(right)) => Some(HardwareEraseCapabilities {
            ata_secure_erase: left.ata_secure_erase || right.ata_secure_erase,
            ata_enhanced_secure_erase: left.ata_enhanced_secure_erase
                || right.ata_enhanced_secure_erase,
            ata_sanitize: left.ata_sanitize || right.ata_sanitize,
            nvme_sanitize: left.nvme_sanitize || right.nvme_sanitize,
            nvme_format: left.nvme_format || right.nvme_format,
        }),
    }
}

fn detect_hardware_erase(device: &DeviceFile, bus: BusKind) -> Option<HardwareEraseCapabilities> {
    let ata = || detect_ata_capabilities(device).ok();
    let ata_scsi = || detect_ata_capabilities_scsi(device).ok();
    let nvme = || detect_nvme_capabilities(device).ok();

    match bus {
        BusKind::Nvme => nvme().or_else(|| merge_capabilities(ata(), ata_scsi())),
        _ => merge_capabilities(merge_capabilities(ata(), ata_scsi()), nvme()),
    }
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_ata_identify() {
        let mut data = [0u8; 512];
        data[82 * 2..82 * 2 + 2].copy_from_slice(&(1u16 << 1).to_le_bytes());
        data[128 * 2..128 * 2 + 2].copy_from_slice(&(1u16 << 5).to_le_bytes());
        data[59 * 2..59 * 2 + 2].copy_from_slice(&(1u16 << 12).to_le_bytes());

        let capabilities = parse_ata_identify(&data);

        assert!(capabilities.ata_secure_erase);
        assert!(capabilities.ata_enhanced_secure_erase);
        assert!(capabilities.ata_sanitize);
        assert!(!capabilities.nvme_sanitize);
    }

    #[test]
    fn test_parse_nvme_identify() {
        assert_eq!(std::mem::size_of::<NVME_IDENTIFY_CONTROLLER_DATA>(), 4096);

        let mut identify: NVME_IDENTIFY_CONTROLLER_DATA = unsafe { std::mem::zeroed() };
        identify.SANICAP._bitfield = 0b101;
        identify.OACS._bitfield = 1 << 1;

        let data =
            unsafe { std::slice::from_raw_parts(std::ptr::from_ref(&identify).cast::<u8>(), 4096) };
        let capabilities = parse_nvme_identify(data);

        assert!(capabilities.nvme_sanitize);
        assert!(capabilities.nvme_format);
        assert!(!capabilities.ata_secure_erase);
    }
}
