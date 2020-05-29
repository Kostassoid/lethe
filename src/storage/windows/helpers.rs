use std::ptr;
use winapi::shared::minwindef::HLOCAL;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winbase::LocalFree;
use winapi::um::winnt::LPWSTR;

#[macro_export]
macro_rules! offset_of {
    ($ty:ty, $field:ident) => {
        unsafe { &(*(0 as *const $ty)).$field as *const _ as usize }
    };
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
