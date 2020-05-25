use super::winapi::um::winnt::GENERIC_WRITE;
use anyhow::Result;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::ptr;
use std::slice;
use winapi::_core::ptr::null_mut;
use winapi::shared::minwindef::HLOCAL;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::OPEN_EXISTING;
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::winbase::LocalFree;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, HANDLE, LPWSTR,
};

#[macro_export]
macro_rules! offset_of {
    ($ty:ty, $field:ident) => {
        unsafe { &(*(0 as *const $ty)).$field as *const _ as usize }
    };
}

pub(crate) fn from_wide_ptr(ptr: *const u16, len: usize) -> String {
    assert!(!ptr.is_null() && len % 2 == 0);
    let slice = unsafe { slice::from_raw_parts(ptr, len / 2) };
    OsString::from_wide(slice).to_string_lossy().into_owned()
}

pub fn get_last_error_str() -> String {
    use winapi::um::winbase::{
        FormatMessageW, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
        FORMAT_MESSAGE_IGNORE_INSERTS,
    };

    let mut buffer: LPWSTR = ptr::null_mut();
    unsafe {
        let strlen = FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_ALLOCATE_BUFFER
                | FORMAT_MESSAGE_IGNORE_INSERTS,
            std::ptr::null(),
            GetLastError(),
            0,
            (&mut buffer as *mut LPWSTR) as LPWSTR,
            0,
            std::ptr::null_mut(),
        );
        let widestr = widestring::WideString::from_ptr(buffer, strlen as usize);
        LocalFree(buffer as HLOCAL);
        widestr.to_string_lossy()
    }
}
