use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use rand::Rng;
use windows::Win32::Storage::IscsiDisc::{
    IOCTL_SCSI_PASS_THROUGH_DIRECT, SCSI_PASS_THROUGH_DIRECT,
};
use windows::Win32::System::Ioctl::{
    IOCTL_STORAGE_PROTOCOL_COMMAND, NVMeDataTypeLogPage, ProtocolTypeNvme,
    STORAGE_PROTOCOL_COMMAND, STORAGE_PROTOCOL_STATUS_SUCCESS, STORAGE_PROTOCOL_STRUCTURE_VERSION,
};

use crate::sanitization::mem::AlignedBuffer;
use crate::storage::HardwareEraseMethod;

use super::access::{DeviceAccess, DeviceFile};
use super::physical_drive::{detect_nvme_capabilities, query_protocol_data};

const ATA_IDENTIFY: u8 = 0xec;
const ATA_SECURITY_SET_PASSWORD: u8 = 0xf1;
const ATA_SECURITY_UNLOCK: u8 = 0xf2;
const ATA_SECURITY_ERASE_PREPARE: u8 = 0xf3;
const ATA_SECURITY_ERASE_UNIT: u8 = 0xf4;
const ATA_SECURITY_DISABLE_PASSWORD: u8 = 0xf6;

const ATA_SECTOR_BYTES: usize = 512;
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

pub(super) fn hardware_erase(device: &DeviceFile, method: HardwareEraseMethod) -> Result<()> {
    // Prefer a plain handle for pass-through commands; the wipe handle may be opened
    // with overlapped I/O, which does not mix well with synchronous control requests.
    let query_device = DeviceFile::open(device.path(), DeviceAccess::QueryIo).ok();
    let target = query_device.as_ref().unwrap_or(device);

    match method {
        HardwareEraseMethod::AtaSecure => ata_secure_erase(target),
        HardwareEraseMethod::NvmeSanitize => nvme_sanitize(target),
        HardwareEraseMethod::AtaSanitize => Err(anyhow!("ATA SANITIZE is not implemented yet.")),
        HardwareEraseMethod::NvmeFormat => Err(anyhow!("NVMe Format NVM is not implemented yet.")),
        HardwareEraseMethod::Off | HardwareEraseMethod::Auto => {
            Err(anyhow!("No hardware erase method was selected."))
        }
    }
}

fn ata_secure_erase(device: &DeviceFile) -> Result<()> {
    let identify = ata_identify(device)?;
    let security = AtaSecurity::from_identify(&identify);

    if !security.supported {
        return Err(anyhow!("This device does not report ATA Security support."));
    }
    if security.frozen {
        return Err(anyhow!(
            "The device is in ATA security frozen state. Power-cycle or unplug/replug the drive, then retry."
        ));
    }
    if security.enabled {
        return Err(anyhow!(
            "The device already has an ATA security password. Refusing to overwrite it."
        ));
    }

    let enhanced = security.enhanced;
    let timeout = security.erase_timeout_seconds(enhanced);
    let mut password = [0u8; 32];
    rand::rng().fill_bytes(&mut password);

    let mut data = security_data(&password, false, false);
    ata_pass_through(
        device,
        ATA_SECURITY_SET_PASSWORD,
        0,
        1,
        0,
        Some((data.as_mut_slice(), false)),
        ATA_TIMEOUT,
    )
    .context("Unable to set the temporary ATA security password")?;

    let result = (|| -> Result<()> {
        ata_pass_through(
            device,
            ATA_SECURITY_ERASE_PREPARE,
            0,
            0,
            0,
            None,
            ATA_TIMEOUT,
        )
        .context("SECURITY ERASE PREPARE failed")?;

        let mut data = security_data(&password, enhanced, true);
        ata_pass_through(
            device,
            ATA_SECURITY_ERASE_UNIT,
            0,
            1,
            0,
            Some((data.as_mut_slice(), false)),
            timeout,
        )
        .context("SECURITY ERASE UNIT failed")
    })();

    if let Err(error) = result {
        let cleanup = ata_security_cleanup(device, &password);
        let context = match cleanup {
            Ok(()) => format!(
                "ATA Secure Erase failed. Temporary password: {}",
                hex(&password)
            ),
            Err(cleanup_error) => format!(
                "ATA Secure Erase failed. Temporary password: {}; cleanup also failed: {cleanup_error:#}",
                hex(&password)
            ),
        };

        return Err(error.context(context));
    }

    Ok(())
}

fn ata_security_cleanup(device: &DeviceFile, password: &[u8; 32]) -> Result<()> {
    let mut data = security_data(password, false, false);
    let unlock = ata_pass_through(
        device,
        ATA_SECURITY_UNLOCK,
        0,
        1,
        0,
        Some((data.as_mut_slice(), false)),
        ATA_TIMEOUT,
    );

    let mut data = security_data(password, false, false);
    let disable = ata_pass_through(
        device,
        ATA_SECURITY_DISABLE_PASSWORD,
        0,
        1,
        0,
        Some((data.as_mut_slice(), false)),
        ATA_TIMEOUT,
    );

    match (unlock, disable) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error.context("SECURITY UNLOCK failed during cleanup")),
        (_, Err(error)) => Err(error.context("SECURITY DISABLE PASSWORD failed during cleanup")),
    }
}

#[allow(clippy::struct_excessive_bools)] // independent ATA security status flags
struct AtaSecurity {
    supported: bool,
    enabled: bool,
    frozen: bool,
    enhanced: bool,
    erase_minutes: u16,
    enhanced_erase_minutes: u16,
}

impl AtaSecurity {
    fn from_identify(data: &[u8]) -> Self {
        let word = |index: usize| u16::from_le_bytes([data[index * 2], data[index * 2 + 1]]);
        let status = word(128);

        AtaSecurity {
            supported: word(82) & (1 << 1) != 0,
            enabled: status & (1 << 1) != 0,
            frozen: status & (1 << 3) != 0,
            enhanced: status & (1 << 5) != 0,
            erase_minutes: word(89),
            enhanced_erase_minutes: word(90),
        }
    }

    fn erase_timeout_seconds(&self, enhanced: bool) -> u32 {
        let raw = if enhanced {
            self.enhanced_erase_minutes
        } else {
            self.erase_minutes
        };

        let minutes = match raw {
            0 | 0xfffe | 0xffff => 30,
            value => u32::from(value).saturating_mul(2),
        };

        minutes.saturating_mul(60).clamp(60, 24 * 3600)
    }
}

fn security_data(password: &[u8; 32], enhanced: bool, erasing: bool) -> AlignedBuffer {
    let mut data = AlignedBuffer::new(ATA_SECTOR_BYTES, ATA_SECTOR_BYTES);
    let bytes = data.as_mut_slice();
    bytes.fill(0);

    bytes[0] = if erasing && enhanced { 0x02 } else { 0 };
    bytes[2..34].copy_from_slice(password);

    data
}

fn ata_identify(device: &DeviceFile) -> Result<Vec<u8>> {
    let mut data = AlignedBuffer::new(ATA_SECTOR_BYTES, ATA_SECTOR_BYTES);
    data.as_mut_slice().fill(0);

    ata_pass_through(
        device,
        ATA_IDENTIFY,
        0,
        1,
        0,
        Some((data.as_mut_slice(), true)),
        ATA_TIMEOUT,
    )?;

    Ok(data.as_slice().to_vec())
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
) -> Result<()> {
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
    let ata = parse_ata_status(sense);
    let (sense_key, asc, ascq) = sense_info(sense);

    let failed = match ata {
        // Prefer the ATA status register: with CK_COND=1 some SAT layers report
        // CHECK CONDITION plus an ATA descriptor even when the command succeeded.
        Some((_, status)) => status & 0x01 != 0 || status & 0x20 != 0,
        None => result.ScsiStatus != 0,
    };

    if failed {
        let ata_description = ata.map_or_else(
            || "ATA status unavailable".to_string(),
            |(error, status)| format!("ATA status {status:#04x}, error {error:#04x}"),
        );

        return Err(anyhow!(
            "ATA pass-through command {command:#04x} failed: SCSI status {}, sense key {sense_key:#04x}, ASC/ASCQ {asc:#04x}/{ascq:#04x}, {ata_description}, sense bytes {}",
            result.ScsiStatus,
            hex(sense),
        ));
    }

    Ok(())
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

fn parse_ata_status(sense: &[u8]) -> Option<(u8, u8)> {
    let response = *sense.first()? & 0x7f;
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
            return Some((data[0], data[9]));
        }

        offset = end;
    }

    None
}

fn nvme_sanitize(device: &DeviceFile) -> Result<()> {
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

fn poll_nvme_sanitize(device: &DeviceFile) -> Result<()> {
    let deadline = Instant::now() + Duration::from_hours(24);
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
                        return Ok(());
                    }
                    NVME_SANITIZE_STATUS_FAILED => {
                        return Err(anyhow!("NVMe Sanitize completed with a failure status."));
                    }
                    NVME_SANITIZE_STATUS_IN_PROGRESS => {
                        zero_status_seen = 0;
                    }
                    0 => {
                        zero_status_seen += 1;
                        if zero_status_seen >= 3 {
                            return Err(anyhow!(
                                "NVMe Sanitize did not start (sanitize status remains idle)."
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

        std::thread::sleep(Duration::from_secs(2));
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_security_data_layout() {
        let password = [0x5a; 32];
        let data = security_data(&password, true, true);

        assert_eq!(data.as_slice()[0], 0x02);
        assert_eq!(&data.as_slice()[2..34], &password);
        assert!(data.as_slice()[34..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn test_erase_timeout_uses_device_estimate() {
        let security = AtaSecurity {
            supported: true,
            enabled: false,
            frozen: false,
            enhanced: true,
            erase_minutes: 2,
            enhanced_erase_minutes: 5,
        };

        assert_eq!(security.erase_timeout_seconds(false), 4 * 60);
        assert_eq!(security.erase_timeout_seconds(true), 10 * 60);
    }
}
