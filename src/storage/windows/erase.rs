use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use windows::Win32::Storage::IscsiDisc::{
    IOCTL_SCSI_PASS_THROUGH_DIRECT, SCSI_PASS_THROUGH_DIRECT,
};
use windows::Win32::System::Ioctl::{
    IOCTL_STORAGE_PROTOCOL_COMMAND, NVMeDataTypeLogPage, ProtocolTypeNvme,
    STORAGE_PROTOCOL_COMMAND, STORAGE_PROTOCOL_STATUS_SUCCESS, STORAGE_PROTOCOL_STRUCTURE_VERSION,
};

use crate::storage::{HardwareEraseCapabilities, HardwareEraseMethod, HardwareEraseOutcome};

use super::access::{DeviceAccess, DeviceFile};
use super::physical_drive::{
    detect_ata_capabilities, detect_ata_capabilities_scsi, detect_nvme_capabilities,
    query_protocol_data,
};

const ATA_SANITIZE: u8 = 0xb4;
const ATA_SANITIZE_STATUS: u16 = 0x0000;
const ATA_SANITIZE_CRYPTO_SCRAMBLE: u16 = 0x0011;
const ATA_SANITIZE_BLOCK_ERASE: u16 = 0x0012;
const ATA_SANITIZE_OVERWRITE: u16 = 0x0014;

const ATA_SANITIZE_CRYPTO_KEY: u64 = 0x4372_7970; // "Cryp"
const ATA_SANITIZE_BLOCK_KEY: u64 = 0x426b_4572; // "BkEr"
const ATA_SANITIZE_OVERWRITE_KEY: u64 = 0x0000_4f57; // "OW"

const ATA_SANITIZE_SUCCEEDED: u8 = 1 << 7;
const ATA_SANITIZE_IN_PROGRESS: u8 = 1 << 6;
const ATA_SANITIZE_FROZEN: u8 = 1 << 5;

const ATA_SENSE_BYTES: usize = 64;
const ATA_TIMEOUT: u32 = 15;

const NVME_SANITIZE_OPCODE: u8 = 0x84;
const NVME_SANITIZE_BLOCK_ERASE: u32 = 2;
const NVME_SANITIZE_OVERWRITE: u32 = 3;
const NVME_SANITIZE_CRYPTO_ERASE: u32 = 4;

const NVME_SANITIZE_STATUS_LOG: u32 = 0x81;
const NVME_SANITIZE_STATUS_IN_PROGRESS: u16 = 2;
const NVME_SANITIZE_STATUS_COMPLETE: u16 = 1;
const NVME_SANITIZE_STATUS_FAILED: u16 = 3;
const NVME_SANITIZE_STATUS_NO_DEALLOC_COMPLETE: u16 = 4;

const SANITIZE_POLL_INTERVAL: Duration = Duration::from_secs(2);
const SANITIZE_TIMEOUT: Duration = Duration::from_hours(24);

pub(super) fn hardware_erase(
    device: &DeviceFile,
    method: HardwareEraseMethod,
) -> Result<HardwareEraseOutcome> {
    // Prefer a plain handle for pass-through commands; the wipe handle may be opened
    // with overlapped I/O, which does not mix well with synchronous control requests.
    let query_device = DeviceFile::open(device.path(), DeviceAccess::QueryIo).ok();
    let target = query_device.as_ref().unwrap_or(device);

    match method {
        HardwareEraseMethod::AtaSanitize => ata_sanitize(target),
        HardwareEraseMethod::NvmeSanitize => nvme_sanitize(target),
        HardwareEraseMethod::Off | HardwareEraseMethod::Auto => {
            Err(anyhow!("No hardware erase method was selected."))
        }
    }
}

fn ata_sanitize(device: &DeviceFile) -> Result<HardwareEraseOutcome> {
    let capabilities = detect_ata_capabilities(device)
        .or_else(|_| detect_ata_capabilities_scsi(device))
        .context("Unable to query ATA sanitize capabilities")?;
    let (action, key, description) = ata_sanitize_action(capabilities)?;

    ata_pass_through(device, ATA_SANITIZE, action, 0, key, None, ATA_TIMEOUT)
        .with_context(|| format!("Unable to start {description}"))?;

    poll_ata_sanitize(device, description)
}

fn ata_sanitize_action(
    capabilities: HardwareEraseCapabilities,
) -> Result<(u16, u64, &'static str)> {
    if capabilities.ata_sanitize_crypto {
        Ok((
            ATA_SANITIZE_CRYPTO_SCRAMBLE,
            ATA_SANITIZE_CRYPTO_KEY,
            "ATA SANITIZE CRYPTO SCRAMBLE",
        ))
    } else if capabilities.ata_sanitize_block_erase {
        Ok((
            ATA_SANITIZE_BLOCK_ERASE,
            ATA_SANITIZE_BLOCK_KEY,
            "ATA SANITIZE BLOCK ERASE",
        ))
    } else if capabilities.ata_sanitize_overwrite {
        Ok((
            ATA_SANITIZE_OVERWRITE,
            (ATA_SANITIZE_OVERWRITE_KEY << 32) | 0x5a5a_5a5a,
            "ATA SANITIZE OVERWRITE",
        ))
    } else {
        Err(anyhow!(
            "This device does not report a supported ATA sanitize action."
        ))
    }
}

fn poll_ata_sanitize(device: &DeviceFile, description: &str) -> Result<HardwareEraseOutcome> {
    let deadline = Instant::now() + SANITIZE_TIMEOUT;
    let mut progress_seen = false;
    let mut idle_polls = 0u32;

    loop {
        let registers = ata_pass_through(
            device,
            ATA_SANITIZE,
            ATA_SANITIZE_STATUS,
            0,
            0,
            None,
            ATA_TIMEOUT,
        )
        .context("Unable to read ATA sanitize status")?;

        let state = if registers.sector_count_hob != 0 {
            registers.sector_count_hob
        } else {
            registers.sector_count
        };
        let progress = (u16::from(registers.lba_mid) << 8) | u16::from(registers.lba_low);

        if state & ATA_SANITIZE_SUCCEEDED != 0 {
            return Ok(HardwareEraseOutcome::confirmed());
        }

        if state & ATA_SANITIZE_FROZEN != 0 {
            return Err(anyhow!(
                "The device is in sanitize frozen state and cannot continue {description}."
            ));
        }

        if state & ATA_SANITIZE_IN_PROGRESS != 0 {
            progress_seen = true;
            idle_polls = 0;
        } else if progress_seen {
            return Err(anyhow!(
                "{description} failed: ATA status {:#04x}, error {:#04x}, progress {progress:#06x}",
                registers.status,
                registers.error
            ));
        } else {
            idle_polls += 1;
            if idle_polls >= 5 {
                return Ok(HardwareEraseOutcome::unconfirmed(format!(
                    "{description} was accepted, but this device or its USB bridge does not report ATA sanitize status (ATA status {:#04x}, error {:#04x}, progress {progress:#06x}). The sanitize is expected to continue in the background; do not retry until it has finished.",
                    registers.status, registers.error
                )));
            }
        }

        if Instant::now() >= deadline {
            return Err(anyhow!(
                "Timed out waiting for {description} to complete (ATA status {:#04x}, error {:#04x}, progress {progress:#06x}).",
                registers.status,
                registers.error
            ));
        }

        std::thread::sleep(SANITIZE_POLL_INTERVAL);
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct AtaRegisters {
    error: u8,
    status: u8,
    sector_count: u8,
    sector_count_hob: u8,
    lba_low: u8,
    lba_mid: u8,
}

#[allow(clippy::cast_possible_truncation)] // CDB fields are explicitly packed
fn ata_pass_through(
    device: &DeviceFile,
    command: u8,
    features: u16,
    sector_count: u16,
    lba: u64,
    data: Option<(&mut [u8], bool)>,
    timeout: u32,
) -> Result<AtaRegisters> {
    let (data_ptr, data_len, data_in) = match data {
        Some((buffer, data_in)) => (
            buffer.as_mut_ptr().cast::<c_void>(),
            u32::try_from(buffer.len()).expect("ATA buffer fits u32"),
            u8::from(data_in),
        ),
        None => (std::ptr::null_mut(), 0, 0),
    };

    let mut cdb = [0u8; 16];
    cdb[0] = 0x85; // ATA PASS-THROUGH (16)

    if data_len == 0 {
        cdb[1] = 3 << 1; // Non-data
        cdb[2] = 0x20; // CK_COND
    } else if data_in != 0 {
        cdb[1] = 4 << 1; // PIO Data-in
        cdb[2] = 0x2e; // CK_COND, T_DIR=1, BYT_BLOK=1, T_LENGTH=2
    } else {
        cdb[1] = 5 << 1; // PIO Data-out
        cdb[2] = 0x26; // CK_COND, T_DIR=0, BYT_BLOK=1, T_LENGTH=2
    }

    cdb[3] = (features >> 8) as u8;
    cdb[4] = features as u8;
    cdb[5] = (sector_count >> 8) as u8;
    cdb[6] = sector_count as u8;
    cdb[7] = (lba >> 24) as u8;
    cdb[8] = lba as u8;
    cdb[9] = (lba >> 32) as u8;
    cdb[10] = (lba >> 8) as u8;
    cdb[11] = (lba >> 40) as u8;
    cdb[12] = (lba >> 16) as u8;
    cdb[14] = command;

    let header_size = std::mem::size_of::<SCSI_PASS_THROUGH_DIRECT>();
    let header = SCSI_PASS_THROUGH_DIRECT {
        Length: u16::try_from(header_size).expect("SCSI header fits u16"),
        ScsiStatus: 0,
        PathId: 0,
        TargetId: 0,
        Lun: 0,
        CdbLength: 16,
        SenseInfoLength: u8::try_from(ATA_SENSE_BYTES).expect("sense buffer fits u8"),
        DataIn: data_in,
        DataTransferLength: data_len,
        TimeOutValue: timeout,
        DataBuffer: data_ptr,
        SenseInfoOffset: u32::try_from(header_size).expect("sense offset fits u32"),
        Cdb: cdb,
    };

    let mut input = vec![0u8; header_size + ATA_SENSE_BYTES];
    unsafe {
        std::ptr::write_unaligned(input.as_mut_ptr().cast(), header);
    }

    let mut output = input.clone();
    device.ioctl_raw(IOCTL_SCSI_PASS_THROUGH_DIRECT, &input, &mut output)?;

    let result: SCSI_PASS_THROUGH_DIRECT =
        unsafe { std::ptr::read_unaligned(output.as_ptr().cast()) };

    let sense_offset = usize::try_from(result.SenseInfoOffset).unwrap_or(header_size);
    let sense_end = sense_offset.saturating_add(usize::from(result.SenseInfoLength));
    let sense = output
        .get(sense_offset..sense_end.min(output.len()))
        .unwrap_or_default();
    let registers = parse_ata_registers(sense);
    let (sense_key, asc, ascq) = sense_info(sense);

    // CK_COND=1 makes successful commands complete with CHECK CONDITION and
    // "ATA PASS-THROUGH INFORMATION AVAILABLE" sense data.
    let informational =
        result.ScsiStatus == 2 && matches!(sense_key, 0x00 | 0x01) && asc == 0 && ascq == 0x1d;

    let failed = if let Some(registers) = registers {
        registers.status & (0x01 | 0x20) != 0
            || (registers.status == 0 && result.ScsiStatus != 0 && !informational)
    } else {
        result.ScsiStatus != 0 && !informational
    };

    if failed {
        let ata_description = registers.map_or_else(
            || "ATA status unavailable".to_string(),
            |registers| {
                format!(
                    "ATA status {:#04x}, error {:#04x}",
                    registers.status, registers.error
                )
            },
        );

        return Err(anyhow!(
            "ATA pass-through command {command:#04x} failed: SCSI status {}, sense key {sense_key:#04x}, ASC/ASCQ {asc:#04x}/{ascq:#04x}, {ata_description}, sense bytes {}",
            result.ScsiStatus,
            hex(sense),
        ));
    }

    Ok(registers.unwrap_or_default())
}

fn parse_ata_registers(sense: &[u8]) -> Option<AtaRegisters> {
    let response = *sense.first()? & 0x7f;

    if response == 0x70 || response == 0x71 {
        if sense.len() < 12 {
            return None;
        }

        return Some(AtaRegisters {
            error: sense[3],
            status: sense[4],
            sector_count: sense[6],
            sector_count_hob: 0,
            lba_low: sense[11],
            lba_mid: sense[10],
        });
    }

    if response != 0x72 && response != 0x73 {
        return None;
    }

    let mut offset = 8usize;
    while offset + 2 <= sense.len() {
        let descriptor_type = sense[offset];
        let descriptor_length = usize::from(sense[offset + 1]);
        let end = offset + 2 + descriptor_length;

        if end > sense.len() {
            break;
        }

        if descriptor_type == 0x09 && descriptor_length >= 10 {
            let data = &sense[offset + 2..end];
            return Some(AtaRegisters {
                error: data[0],
                status: data[9],
                sector_count: data[1],
                sector_count_hob: data[2],
                lba_low: data[3],
                lba_mid: data[5],
            });
        }

        offset = end;
    }

    None
}

fn sense_info(sense: &[u8]) -> (u8, u8, u8) {
    if sense.len() < 4 {
        return (0, 0, 0);
    }

    match sense[0] & 0x7f {
        0x72 | 0x73 => (sense[1] & 0x0f, sense[2], sense[3]),
        0x70 | 0x71 if sense.len() >= 14 => (sense[2] & 0x0f, sense[12], sense[13]),
        _ => (0, 0, 0),
    }
}

fn nvme_sanitize(device: &DeviceFile) -> Result<HardwareEraseOutcome> {
    let capabilities =
        detect_nvme_capabilities(device).context("Unable to query NVMe sanitize capabilities")?;

    let action = if capabilities.nvme_sanitize_crypto {
        NVME_SANITIZE_CRYPTO_ERASE
    } else if capabilities.nvme_sanitize_block_erase {
        NVME_SANITIZE_BLOCK_ERASE
    } else if capabilities.nvme_sanitize_overwrite {
        NVME_SANITIZE_OVERWRITE
    } else {
        return Err(anyhow!(
            "This device does not report a supported NVMe sanitize action."
        ));
    };

    nvme_sanitize_command(device, action)?;
    poll_nvme_sanitize(device)
}

fn nvme_sanitize_command(device: &DeviceFile, action: u32) -> Result<()> {
    let command_offset = std::mem::offset_of!(STORAGE_PROTOCOL_COMMAND, Command);
    let command_length = 64usize;
    let buffer_length = std::mem::size_of::<STORAGE_PROTOCOL_COMMAND>() + command_length;

    let header = STORAGE_PROTOCOL_COMMAND {
        Version: STORAGE_PROTOCOL_STRUCTURE_VERSION,
        Length: u32::try_from(std::mem::size_of::<STORAGE_PROTOCOL_COMMAND>())
            .expect("protocol command header fits u32"),
        ProtocolType: ProtocolTypeNvme,
        Flags: 0,
        ReturnStatus: 0,
        ErrorCode: 0,
        CommandLength: u32::try_from(command_length).expect("NVMe command fits u32"),
        ErrorInfoLength: 0,
        DataToDeviceTransferLength: 0,
        DataFromDeviceTransferLength: 0,
        TimeOutValue: 60,
        ErrorInfoOffset: 0,
        DataToDeviceBufferOffset: 0,
        DataFromDeviceBufferOffset: 0,
        CommandSpecific: 0,
        Reserved0: 0,
        FixedProtocolReturnData: 0,
        Reserved1: [0; 3],
        Command: [0],
    };

    let mut buffer = vec![0u8; buffer_length];
    unsafe {
        std::ptr::write_unaligned(buffer.as_mut_ptr().cast(), header);
    }

    let mut command = [0u8; 64];
    command[0] = NVME_SANITIZE_OPCODE;
    command[4..8].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
    command[40..44].copy_from_slice(&action.to_le_bytes());

    buffer[command_offset..command_offset + command_length].copy_from_slice(&command);

    let mut output = buffer.clone();
    device.ioctl_raw(IOCTL_STORAGE_PROTOCOL_COMMAND, &buffer, &mut output)?;

    let result: STORAGE_PROTOCOL_COMMAND =
        unsafe { std::ptr::read_unaligned(output.as_ptr().cast()) };

    if result.ReturnStatus != STORAGE_PROTOCOL_STATUS_SUCCESS {
        return Err(anyhow!(
            "NVMe Sanitize was rejected (status {}, error {})",
            result.ReturnStatus,
            result.ErrorCode
        ));
    }

    Ok(())
}

fn poll_nvme_sanitize(device: &DeviceFile) -> Result<HardwareEraseOutcome> {
    let deadline = Instant::now() + SANITIZE_TIMEOUT;
    let mut last_error = None;
    let mut zero_status_seen = 0u8;

    loop {
        match query_protocol_data(
            device,
            ProtocolTypeNvme,
            u32::try_from(NVMeDataTypeLogPage.0).expect("NVMe data type is non-negative"),
            NVME_SANITIZE_STATUS_LOG,
            512,
        ) {
            Ok(data) if data.len() >= 4 => {
                let status = u16::from_le_bytes([data[2], data[3]]) & 0x7;

                match status {
                    NVME_SANITIZE_STATUS_COMPLETE | NVME_SANITIZE_STATUS_NO_DEALLOC_COMPLETE => {
                        return Ok(HardwareEraseOutcome::confirmed());
                    }
                    NVME_SANITIZE_STATUS_FAILED => {
                        return Err(anyhow!("NVMe Sanitize completed with a failure status."));
                    }
                    NVME_SANITIZE_STATUS_IN_PROGRESS => {
                        zero_status_seen = 0;
                    }
                    0 => {
                        zero_status_seen += 1;
                        if zero_status_seen >= 5 {
                            return Ok(HardwareEraseOutcome::unconfirmed(
                                "NVMe Sanitize was accepted, but the device does not report sanitize status. The sanitize is expected to continue in the background; do not retry until it has finished.",
                            ));
                        }
                    }
                    other => {
                        return Err(anyhow!("Unexpected NVMe sanitize status {other}."));
                    }
                }
            }
            Ok(_) => last_error = Some(anyhow!("Sanitize status log was too short.")),
            Err(error) => last_error = Some(error),
        }

        if Instant::now() >= deadline {
            return Err(last_error
                .unwrap_or_else(|| anyhow!("Timed out waiting for NVMe Sanitize to complete.")));
        }

        std::thread::sleep(SANITIZE_POLL_INTERVAL);
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_ata_sanitize_action_preferences() {
        let capabilities = HardwareEraseCapabilities {
            ata_sanitize_crypto: true,
            ata_sanitize_block_erase: true,
            ata_sanitize_overwrite: true,
            ..Default::default()
        };
        assert_eq!(
            ata_sanitize_action(capabilities).unwrap(),
            (
                ATA_SANITIZE_CRYPTO_SCRAMBLE,
                ATA_SANITIZE_CRYPTO_KEY,
                "ATA SANITIZE CRYPTO SCRAMBLE"
            )
        );

        let capabilities = HardwareEraseCapabilities {
            ata_sanitize_block_erase: true,
            ..Default::default()
        };
        assert_eq!(
            ata_sanitize_action(capabilities).unwrap().0,
            ATA_SANITIZE_BLOCK_ERASE
        );

        let capabilities = HardwareEraseCapabilities {
            ata_sanitize_overwrite: true,
            ..Default::default()
        };
        assert_eq!(
            ata_sanitize_action(capabilities).unwrap().0,
            ATA_SANITIZE_OVERWRITE
        );
    }

    #[test]
    fn test_parse_ata_registers() {
        let mut sense = [0u8; 24];
        sense[0] = 0x72;
        sense[7] = 0x0e;
        sense[8] = 0x09;
        sense[9] = 0x0c;
        sense[10] = 0x04;
        sense[11] = 0x50;
        sense[12] = 0x00;
        sense[13] = 0x12;
        sense[14] = 0x00;
        sense[15] = 0x34;
        sense[16] = 0x00;
        sense[19] = 0x50;

        let registers = parse_ata_registers(&sense).unwrap();

        assert_eq!(registers.error, 0x04);
        assert_eq!(registers.status, 0x50);
        assert_eq!(registers.sector_count, 0x50);
        assert_eq!(registers.sector_count_hob, 0x00);
        assert_eq!(registers.lba_low, 0x12);
        assert_eq!(registers.lba_mid, 0x34);
    }

    #[test]
    fn test_parse_fixed_sense_registers() {
        let mut sense = [0u8; 18];
        sense[0] = 0x70;
        sense[2] = 0x01;
        sense[3] = 0x04;
        sense[4] = 0x50;
        sense[5] = 0x00;
        sense[6] = 0x12;
        sense[7] = 0x0a;
        sense[9] = 0x00;
        sense[10] = 0x34;
        sense[11] = 0x56;
        sense[12] = 0x00;
        sense[13] = 0x1d;

        let registers = parse_ata_registers(&sense).unwrap();

        assert_eq!(registers.error, 0x04);
        assert_eq!(registers.status, 0x50);
        assert_eq!(registers.sector_count, 0x12);
        assert_eq!(registers.sector_count_hob, 0x00);
        assert_eq!(registers.lba_low, 0x56);
        assert_eq!(registers.lba_mid, 0x34);
    }
}
