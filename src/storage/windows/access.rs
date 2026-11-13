use std::collections::VecDeque;
use std::ffi::c_void;
use std::io;
use std::mem::size_of_val;

use anyhow::{Context, Result, anyhow};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_CRC, ERROR_IO_PENDING, ERROR_READ_FAULT, ERROR_SECTOR_NOT_FOUND, ERROR_SEEK,
    ERROR_WRITE_FAULT, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_BEGIN, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OVERLAPPED,
    FILE_FLAG_RANDOM_ACCESS, FILE_FLAG_SEQUENTIAL_SCAN, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FlushFileBuffers, OPEN_EXISTING, ReadFile, SetFilePointerEx, WriteFile,
};
use windows::Win32::System::IO::{DeviceIoControl, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::{Error as WindowsError, HSTRING};

use crate::storage::{
    AsyncStorageAccess, IoCompletion, IoKind, StorageAccess, StorageError, StorageRef,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeviceAccess {
    Query,
    /// A readable handle for pass-through queries that require more than zero access.
    QueryIo,
    Wipe,
    /// A write handle for asynchronous I/O, opened with `FILE_FLAG_OVERLAPPED`.
    WipeOverlapped,
}

pub(super) struct DeviceFile {
    is_locked: bool,
    handle: HANDLE,
}

impl DeviceFile {
    pub(super) fn open(path: &str, access: DeviceAccess) -> Result<Self> {
        let mut file_path = path.to_string();
        if !path.starts_with("\\\\") {
            file_path.insert_str(0, "\\\\.\\GLOBALROOT");
        }

        let desired_access = match access {
            DeviceAccess::Query => 0,
            DeviceAccess::QueryIo | DeviceAccess::Wipe | DeviceAccess::WipeOverlapped => {
                GENERIC_READ.0 | GENERIC_WRITE.0
            }
        };

        let mut flags = match access {
            // Pass-through queries are buffered by Windows; keep this handle plain.
            DeviceAccess::QueryIo => FILE_ATTRIBUTE_NORMAL | FILE_FLAG_RANDOM_ACCESS,
            _ => {
                FILE_ATTRIBUTE_NORMAL
                    | FILE_FLAG_NO_BUFFERING
                    | FILE_FLAG_WRITE_THROUGH
                    | FILE_FLAG_SEQUENTIAL_SCAN
                    | FILE_FLAG_RANDOM_ACCESS
            }
        };

        if access == DeviceAccess::WipeOverlapped {
            flags |= FILE_FLAG_OVERLAPPED;
        }

        let name = HSTRING::from(file_path.as_str());
        let handle = unsafe {
            CreateFileW(
                &name,
                desired_access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                flags,
                None,
            )
        }
        .with_context(|| format!("Cannot open device {path}."))?;

        let mut device = DeviceFile {
            is_locked: false,
            handle,
        };

        if access == DeviceAccess::Wipe || access == DeviceAccess::WipeOverlapped {
            let mut returned = 0u32;

            unsafe {
                DeviceIoControl(
                    handle,
                    FSCTL_LOCK_VOLUME,
                    None,
                    0,
                    None,
                    0,
                    Some(&raw mut returned),
                    None,
                )
            }
            .with_context(|| {
                format!(
                    "Cannot lock device {path}. Make sure to close other applications accessing the storage."
                )
            })?;

            unsafe {
                DeviceIoControl(
                    handle,
                    FSCTL_DISMOUNT_VOLUME,
                    None,
                    0,
                    None,
                    0,
                    Some(&raw mut returned),
                    None,
                )
            }
            .with_context(|| format!("Cannot dismount volume {path}."))?;

            device.is_locked = true;
        }

        Ok(device)
    }

    pub(super) fn ioctl<T: Default>(&self, code: u32) -> Result<T> {
        let mut output = T::default();
        self.ioctl_into(code, Option::<&()>::None, &mut output)?;
        Ok(output)
    }

    pub(super) fn ioctl_with_input<I, T: Default>(&self, code: u32, input: &I) -> Result<T> {
        let mut output = T::default();
        self.ioctl_into(code, Some(input), &mut output)?;
        Ok(output)
    }

    pub(super) fn ioctl_raw(&self, code: u32, input: &[u8], output: &mut [u8]) -> Result<u32> {
        let mut returned = 0u32;

        unsafe {
            DeviceIoControl(
                self.handle,
                code,
                Some(input.as_ptr().cast::<c_void>()),
                u32::try_from(input.len()).expect("ioctl input is too large"),
                Some(output.as_mut_ptr().cast::<c_void>()),
                u32::try_from(output.len()).expect("ioctl output is too large"),
                Some(&raw mut returned),
                None,
            )
        }
        .map_err(|error| StorageError::from_windows(&error))?;

        Ok(returned)
    }

    pub(super) fn ioctl_into<I, T>(
        &self,
        code: u32,
        input: Option<&I>,
        output: &mut T,
    ) -> Result<u32> {
        let mut returned = 0u32;

        unsafe {
            DeviceIoControl(
                self.handle,
                code,
                input.map(|value| std::ptr::from_ref(value).cast::<c_void>()),
                input.map_or(0, |value| {
                    u32::try_from(size_of_val(value)).expect("ioctl input is too large")
                }),
                Some(std::ptr::from_mut(output).cast::<c_void>()),
                u32::try_from(size_of_val(output)).expect("ioctl output is too large"),
                Some(&raw mut returned),
                None,
            )
        }
        .map_err(|error| StorageError::from_windows(&error))?;

        Ok(returned)
    }
}

impl Drop for DeviceFile {
    fn drop(&mut self) {
        if self.handle.is_invalid() {
            return;
        }

        if self.is_locked {
            let mut returned = 0u32;
            unsafe {
                let _ = DeviceIoControl(
                    self.handle,
                    FSCTL_UNLOCK_VOLUME,
                    None,
                    0,
                    None,
                    0,
                    Some(&raw mut returned),
                    None,
                );
            }
        }

        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

impl StorageError {
    fn from_windows(error: &WindowsError) -> Self {
        let code = win32_error_code(error);

        match code {
            c if c == ERROR_CRC.0
                || c == ERROR_SEEK.0
                || c == ERROR_SECTOR_NOT_FOUND.0
                || c == ERROR_WRITE_FAULT.0
                || c == ERROR_READ_FAULT.0 =>
            {
                StorageError::BadBlock
            }
            _ => StorageError::Other(io::Error::from_raw_os_error(code.cast_signed())),
        }
    }
}

fn win32_error_code(error: &WindowsError) -> u32 {
    let code = error.code().0.cast_unsigned();
    if code & 0xFFFF_0000 == 0x8007_0000 {
        code & 0xFFFF
    } else {
        code
    }
}

struct PendingOp {
    overlapped: Box<OVERLAPPED>,
    event: HANDLE,
    slot: usize,
}

fn close_event(event: HANDLE) {
    if !event.is_invalid() {
        unsafe {
            let _ = CloseHandle(event);
        }
    }
}

/// Asynchronous access over one overlapped device handle. Requests are reaped in
/// submission order, which keeps completion processing deterministic while still
/// allowing several requests to be in flight.
pub(super) struct OverlappedAccess {
    device: DeviceFile,
    _children: Vec<DeviceFile>,
    pending: VecDeque<PendingOp>,
    depth: usize,
}

pub(super) fn open_overlapped(
    device: &StorageRef,
    depth: usize,
) -> Result<Box<dyn AsyncStorageAccess>> {
    let children: Result<Vec<DeviceFile>> = device
        .children
        .iter()
        .map(|child| DeviceFile::open(&child.id, DeviceAccess::Wipe))
        .collect();

    let device = DeviceFile::open(&device.id, DeviceAccess::WipeOverlapped)?;

    Ok(Box::new(OverlappedAccess {
        device,
        _children: children?,
        pending: VecDeque::new(),
        depth: depth.max(1),
    }))
}

impl AsyncStorageAccess for OverlappedAccess {
    fn depth(&self) -> usize {
        self.depth
    }

    fn submit(
        &mut self,
        slot: usize,
        position: u64,
        buffer: &mut [u8],
        kind: IoKind,
    ) -> Result<()> {
        let mut overlapped = Box::<OVERLAPPED>::default();
        overlapped.Anonymous.Anonymous.Offset =
            u32::try_from(position & 0xFFFF_FFFF).expect("low word fits u32");
        overlapped.Anonymous.Anonymous.OffsetHigh =
            u32::try_from(position >> 32).expect("high word fits u32");

        let event = unsafe { CreateEventW(None, true, false, None) }
            .context("Unable to create an overlapped I/O event")?;
        overlapped.hEvent = event;

        let overlapped_ptr: *mut OVERLAPPED = std::ptr::addr_of_mut!(*overlapped);

        let result = unsafe {
            match kind {
                IoKind::Read => {
                    ReadFile(self.device.handle, Some(buffer), None, Some(overlapped_ptr))
                }
                IoKind::Write => {
                    WriteFile(self.device.handle, Some(buffer), None, Some(overlapped_ptr))
                }
            }
        };

        match result {
            Ok(()) => {}
            Err(error) if win32_error_code(&error) == ERROR_IO_PENDING.0 => {}
            Err(error) => {
                close_event(event);
                return Err(StorageError::from_windows(&error).into());
            }
        }

        self.pending.push_back(PendingOp {
            overlapped,
            event,
            slot,
        });
        Ok(())
    }

    fn reap(&mut self) -> Result<IoCompletion> {
        let op = self
            .pending
            .pop_front()
            .ok_or_else(|| anyhow!("No pending overlapped I/O"))?;

        let mut transferred = 0u32;
        let result = unsafe {
            GetOverlappedResult(
                self.device.handle,
                std::ptr::addr_of!(*op.overlapped),
                &raw mut transferred,
                true,
            )
        };

        let result = result
            .map(|()| transferred as usize)
            .map_err(|error| StorageError::from_windows(&error).into());

        close_event(op.event);

        Ok(IoCompletion {
            slot: op.slot,
            result,
        })
    }

    fn drain(&mut self) -> Result<()> {
        while let Some(op) = self.pending.pop_front() {
            let mut transferred = 0u32;
            unsafe {
                let _ = GetOverlappedResult(
                    self.device.handle,
                    std::ptr::addr_of!(*op.overlapped),
                    &raw mut transferred,
                    true,
                );
            }
            close_event(op.event);
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.device.flush()
    }
}

impl StorageAccess for DeviceFile {
    fn seek(&mut self, position: u64) -> Result<u64> {
        let distance = i64::try_from(position).context("Device position is too large.")?;
        let mut new_position = 0i64;

        unsafe {
            SetFilePointerEx(
                self.handle,
                distance,
                Some(&raw mut new_position),
                FILE_BEGIN,
            )
        }
        .map_err(|error| StorageError::from_windows(&error))
        .context("Unable to set device position.")?;

        u64::try_from(new_position).context("Device position is negative.")
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        let mut read = 0u32;

        unsafe { ReadFile(self.handle, Some(buffer), Some(&raw mut read), None) }
            .map_err(|error| StorageError::from_windows(&error))
            .context("Unable to read from the device.")?;

        Ok(usize::try_from(read).expect("read length fits into usize"))
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        let mut written = 0u32;

        unsafe { WriteFile(self.handle, Some(data), Some(&raw mut written), None) }
            .map_err(|error| StorageError::from_windows(&error))
            .context("Unable to write to the device.")?;

        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        unsafe { FlushFileBuffers(self.handle) }
            .map_err(|error| StorageError::from_windows(&error))
            .context("Unable to flush device write buffers.")
    }
}
