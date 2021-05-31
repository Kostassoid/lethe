#![cfg(windows)]
use crate::storage::{StorageAccess, StorageError};
use anyhow::{Context, Result};
use std::{io, mem, ptr};
use widestring::WideCString;
use winapi::_core::ptr::null_mut;
use winapi::shared::minwindef::{DWORD, LPVOID};
use winapi::shared::winerror::{
    ERROR_CRC, ERROR_READ_FAULT, ERROR_SECTOR_NOT_FOUND, ERROR_SEEK, ERROR_WRITE_FAULT,
};
use winapi::um::fileapi::*;
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::ioapiset::DeviceIoControl;
use winapi::um::winbase::{
    FILE_BEGIN, FILE_CURRENT, FILE_FLAG_NO_BUFFERING, FILE_FLAG_RANDOM_ACCESS,
    FILE_FLAG_SEQUENTIAL_SCAN, FILE_FLAG_WRITE_THROUGH,
};
use winapi::um::winioctl;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE, HANDLE,
    LARGE_INTEGER,
};

pub struct DeviceFile {
    is_locked: bool,
    pub handle: HANDLE,
}

impl DeviceFile {
    pub fn open(path: &str, write_access: bool) -> Result<Self> {
        let mut file_path = path.to_string();
        if !path.starts_with("\\\\") {
            // assuming NT device name like \Harddisk1\Partition1
            file_path.insert_str(0, "\\\\.\\GLOBALROOT"); //todo: check minimal Windows version
        }

        let access = if write_access {
            GENERIC_READ | GENERIC_WRITE
        } else {
            GENERIC_READ
        };

        unsafe {
            let handle = CreateFileW(
                WideCString::from_str(file_path.clone()).unwrap().as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL
                    | FILE_FLAG_NO_BUFFERING
                    | FILE_FLAG_WRITE_THROUGH
                    | FILE_FLAG_SEQUENTIAL_SCAN
                    | FILE_FLAG_RANDOM_ACCESS,
                null_mut(),
            );

            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error())
                    .context(format!("Cannot open device {}.", path));
            }

            let mut is_locked = false;

            if write_access {
                let mut returned: DWORD = 0;

                if DeviceIoControl(
                    handle,
                    winioctl::FSCTL_LOCK_VOLUME,
                    null_mut(),
                    0,
                    null_mut(),
                    0,
                    &mut returned,
                    null_mut(),
                ) == 0
                {
                    return Err(io::Error::last_os_error())
                        .context(format!("Cannot lock device {}. Make sure to close other applications accessing the storage.", path));
                }

                if DeviceIoControl(
                    handle,
                    winioctl::FSCTL_DISMOUNT_VOLUME,
                    null_mut(),
                    0,
                    null_mut(),
                    0,
                    &mut returned,
                    null_mut(),
                ) == 0
                {
                    return Err(io::Error::last_os_error())
                        .context(format!("Cannot dismount volume {}.", path));
                }
                is_locked = true;
            }

            Ok(DeviceFile { handle, is_locked })
        }
    }
}

impl StorageError {
    fn from(err: std::io::Error) -> StorageError {
        match err.raw_os_error() {
            Some(c)
                if c == ERROR_CRC as i32
                    || c == ERROR_SEEK as i32
                    || c == ERROR_SECTOR_NOT_FOUND as i32
                    || c == ERROR_WRITE_FAULT as i32
                    || c == ERROR_READ_FAULT as i32 =>
            {
                StorageError::BadBlock
            }
            _ => StorageError::Other(err),
        }
    }
}

impl Drop for DeviceFile {
    fn drop(&mut self) {
        if self.handle != null_mut() {
            if self.is_locked {
                unsafe {
                    let mut returned: DWORD = 0;
                    if DeviceIoControl(
                        self.handle,
                        winioctl::FSCTL_UNLOCK_VOLUME,
                        null_mut(),
                        0,
                        null_mut(),
                        0,
                        &mut returned,
                        null_mut(),
                    ) == 0
                    {
                        //there doesn't seem to be a good way to recover
                    }
                }
            }
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

impl StorageAccess for DeviceFile {
    fn position(&mut self) -> Result<u64> {
        unsafe {
            let distance = mem::zeroed();
            let mut current: LARGE_INTEGER = mem::zeroed();
            if SetFilePointerEx(self.handle, distance, &mut current, FILE_CURRENT) == 0 {
                return Err(StorageError::from(io::Error::last_os_error()))
                    .context("Unable to get device position.");
            };
            Ok(*current.QuadPart() as u64)
        }
    }

    fn seek(&mut self, position: u64) -> Result<u64> {
        unsafe {
            let mut distance: LARGE_INTEGER = mem::zeroed();
            *distance.QuadPart_mut() = position as i64;

            let mut new_position: LARGE_INTEGER = mem::zeroed();
            if SetFilePointerEx(self.handle, distance, &mut new_position, FILE_BEGIN) == 0 {
                return Err(StorageError::from(io::Error::last_os_error()))
                    .context("Unable to set device position.");
            };
            Ok(*new_position.QuadPart() as u64)
        }
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        unsafe {
            let mut read = 0;
            if ReadFile(
                self.handle,
                buffer.as_ptr() as LPVOID,
                buffer.len() as DWORD,
                &mut read,
                ptr::null_mut(),
            ) == 0
            {
                return Err(StorageError::from(io::Error::last_os_error()))
                    .context("Unable to read from the device.");
            };
            Ok(read as usize)
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        unsafe {
            let mut written = 0;
            if WriteFile(
                self.handle,
                data.as_ptr() as LPVOID,
                data.len() as DWORD,
                &mut written,
                ptr::null_mut(),
            ) == 0
            {
                return Err(StorageError::from(io::Error::last_os_error()))
                    .context("Unable to write to the device.");
            };
            Ok(())
        }
    }

    fn flush(&mut self) -> Result<()> {
        unsafe {
            if FlushFileBuffers(self.handle) == 0 {
                return Err(StorageError::from(io::Error::last_os_error()))
                    .context("Unable to flush device write buffers.");
            }
            Ok(())
        }
    }
}
