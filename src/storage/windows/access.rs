#![cfg(windows)]
use crate::storage::{StorageAccess, StorageError};
use anyhow::{Context, Result};
use std::{io, mem, ptr};
use widestring::WideCString;

use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{
            CloseHandle, ERROR_CRC, ERROR_READ_FAULT, ERROR_SECTOR_NOT_FOUND, ERROR_SEEK,
            ERROR_WRITE_FAULT, GENERIC_READ, GENERIC_WRITE, HANDLE,
        },
        Storage::FileSystem::{
            CreateFileW, FlushFileBuffers, ReadFile, SetFilePointerEx, WriteFile,
            FILE_ATTRIBUTE_NORMAL, FILE_BEGIN, FILE_CURRENT, FILE_FLAG_NO_BUFFERING,
            FILE_FLAG_RANDOM_ACCESS, FILE_FLAG_SEQUENTIAL_SCAN, FILE_FLAG_WRITE_THROUGH,
            FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        },
        System::Ioctl::{FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME},
        System::IO::DeviceIoControl,
    },
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
                //WideCString::from_str(file_path.clone()).unwrap().as_ptr(),
                &HSTRING::from(file_path),
                access.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL
                    | FILE_FLAG_NO_BUFFERING
                    | FILE_FLAG_WRITE_THROUGH
                    | FILE_FLAG_SEQUENTIAL_SCAN
                    | FILE_FLAG_RANDOM_ACCESS,
                None,
            )
            .context(format!("Cannot open device {}.", path))?;

            let mut is_locked = false;

            if write_access {
                let mut returned: DWORD = 0;

                DeviceIoControl(
                    handle,
                    FSCTL_LOCK_VOLUME,
                    None,
                    0,
                    None,
                    0,
                    Some(&mut returned),
                    None,
                ).context(format!("Cannot lock device {}. Make sure to close other applications accessing the storage.", path))?;

                DeviceIoControl(
                    handle,
                    FSCTL_DISMOUNT_VOLUME,
                    None,
                    0,
                    None,
                    0,
                    Some(&mut returned),
                    None,
                )
                .context(format!("Cannot dismount volume {}.", path))?;

                is_locked = true;
            }

            Ok(DeviceFile { handle, is_locked })
        }
    }
}

impl StorageError {
    fn from(err: io::Error) -> StorageError {
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
        if self.handle.is_invalid() {
            return;
        }

        if self.is_locked {
            unsafe {
                let mut returned: DWORD = 0;
                let _ = DeviceIoControl(
                    self.handle,
                    FSCTL_UNLOCK_VOLUME,
                    None,
                    0,
                    None,
                    0,
                    Some(&mut returned),
                    None,
                );
            }
        }
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

impl StorageAccess for DeviceFile {
    fn position(&mut self) -> Result<u64> {
        unsafe {
            let distance = mem::zeroed();
            let mut current: LARGE_INTEGER = mem::zeroed();
            if SetFilePointerEx(self.handle, distance, Some(&mut current), FILE_CURRENT) == 0 {
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
            if SetFilePointerEx(self.handle, distance, Some(&mut new_position), FILE_BEGIN) == 0 {
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
                buffer.as_ptr() as _,
                buffer.len() as _,
                Some(&mut read),
                None,
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
            if WriteFile(self.handle, data.as_ptr() as _, data.len() as _, None) == 0 {
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
