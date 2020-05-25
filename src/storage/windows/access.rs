use super::helpers::*;
use crate::storage::StorageAccess;
use anyhow::Result;
use std::{mem, ptr};
use winapi::_core::ptr::null_mut;
use winapi::shared::minwindef::{DWORD, LPVOID};
use winapi::um::fileapi::*;
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::winbase::{FILE_BEGIN, FILE_CURRENT};
use winapi::um::winnt::{
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE, HANDLE,
    LARGE_INTEGER,
};

pub(crate) struct DeviceFile {
    pub(crate) handle: HANDLE,
}

impl DeviceFile {
    pub fn open(path: &str, write_access: bool) -> Result<Self> {
        unsafe {
            let access = if write_access {
                GENERIC_READ | GENERIC_WRITE
            } else {
                GENERIC_READ
            };

            let handle = CreateFileW(
                widestring::WideCString::from_str(path.clone())
                    .unwrap()
                    .as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            );

            if handle == INVALID_HANDLE_VALUE {
                return Err(anyhow!(
                    "Cannot open device {}. Error: {}",
                    path,
                    get_last_error_str()
                ));
            }

            Ok(DeviceFile { handle })
        }
    }
}

impl Drop for DeviceFile {
    fn drop(&mut self) {
        if self.handle != null_mut() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

impl StorageAccess for DeviceFile {
    fn position(&mut self) -> Result<u64> {
        unsafe {
            let mut distance = mem::zeroed();
            let mut current: LARGE_INTEGER = mem::zeroed();
            if SetFilePointerEx(self.handle, distance, &mut current, FILE_CURRENT) == 0 {
                return Err(anyhow!(
                    "Unable to get device position. Error: {}",
                    get_last_error_str()
                ));
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
                return Err(anyhow!(
                    "Unable to set device position. Error: {}",
                    get_last_error_str()
                ));
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
                return Err(anyhow!(
                    "Unable to read from the device. Error: {}",
                    get_last_error_str()
                ));
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
                return Err(anyhow!(
                    "Unable to write to the device. Error: {}",
                    get_last_error_str()
                ));
            };
            Ok(())
        }
    }

    fn flush(&self) -> Result<()> {
        unsafe {
            if FlushFileBuffers(self.handle) == 0 {
                return Err(anyhow!(
                    "Unable to flush device write buffers. Error: {}",
                    get_last_error_str()
                ));
            }
            Ok(())
        }
    }
}
