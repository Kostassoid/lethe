use super::winapi::shared::ntdef::PVOID;
use std::mem;
use std::ptr::null_mut;
use winapi::um::handleapi::CloseHandle;
use winapi::um::processthreadsapi::GetCurrentProcess;
use winapi::um::processthreadsapi::OpenProcessToken;
use winapi::um::securitybaseapi::GetTokenInformation;
use winapi::um::winnt::TokenElevation;
use winapi::um::winnt::HANDLE;
use winapi::um::winnt::TOKEN_ELEVATION;
use winapi::um::winnt::TOKEN_QUERY;

pub fn is_elevated() -> bool {
    let mut result = false;
    let mut handle: HANDLE = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } != 0 {
        let mut elevation: TOKEN_ELEVATION = unsafe { mem::zeroed() };
        let size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
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
