use std::ascii::escape_default;
use std::mem;

use windows::{
    Win32::Foundation::{CloseHandle, HANDLE},
    Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
    Win32::System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub fn is_elevated() -> bool {
    let mut result = false;
    let mut handle: HANDLE = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } != 0 {
        let mut elevation: TOKEN_ELEVATION = unsafe { mem::zeroed() };
        let size = mem::size_of::<TOKEN_ELEVATION>() as u32;
        let mut ret_size = size;
        if unsafe {
            GetTokenInformation(
                handle,
                TokenElevation,
                &mut elevation as *mut _ as PVOID,
                size,
                &mut ret_size,
            )
        } != 0
        {
            result = elevation.TokenIsElevated != 0;
        }
    }
    if !handle.is_null() {
        unsafe {
            CloseHandle(handle);
        }
    }
    result
}
