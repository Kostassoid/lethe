use libc;
use std::mem;
use std::slice;
use winapi::um::setupapi::*;

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

extern crate winapi;
use winapi::shared::minwindef::*;

macro_rules! offset_of {
    ($ty:ty, $field:ident) => {
        unsafe { &(*(0 as *const $ty)).$field as *const _ as usize }
    };
}

fn from_wide_ptr(ptr: *const u16, len: usize) -> String {
    assert!(!ptr.is_null() && len % 2 == 0);
    let slice = unsafe { slice::from_raw_parts(ptr, len / 2) };
    OsString::from_wide(slice).to_string_lossy().into_owned()
}

pub struct DeviceInterfaceDetailData {
    data: PSP_DEVICE_INTERFACE_DETAIL_DATA_W,
    path_len: usize,
}

impl DeviceInterfaceDetailData {
    pub fn new(size: usize) -> Option<Self> {
        let mut cb_size = mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        if cfg!(target_pointer_width = "32") {
            cb_size = 4 + 2; // 4-byte uint + default TCHAR size. size_of is inaccurate.
        }

        if size < cb_size {
            //warn!("DeviceInterfaceDetailData is too small. {}", size);
            return None;
        }

        let data = unsafe { libc::malloc(size) as PSP_DEVICE_INTERFACE_DETAIL_DATA_W };
        if data.is_null() {
            return None;
        }

        // Set total size of the structure.
        unsafe { (*data).cbSize = cb_size as UINT };

        // Compute offset of `SP_DEVICE_INTERFACE_DETAIL_DATA_W.DevicePath`.
        let offset = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);

        Some(Self {
            data,
            path_len: size - offset,
        })
    }

    pub fn get(&self) -> PSP_DEVICE_INTERFACE_DETAIL_DATA_W {
        self.data
    }

    pub fn path(&self) -> String {
        unsafe { from_wide_ptr((*self.data).DevicePath.as_ptr(), self.path_len - 2) }
    }
}

impl Drop for DeviceInterfaceDetailData {
    fn drop(&mut self) {
        unsafe { libc::free(self.data as *mut libc::c_void) };
    }
}
