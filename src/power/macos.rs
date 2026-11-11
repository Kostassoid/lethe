use std::ffi::{CString, c_char, c_int, c_void};

use anyhow::{Context, Result, anyhow};

type CfAllocatorRef = *const c_void;
type CfStringRef = *const c_void;
type CfTypeRef = *const c_void;
type IoReturn = c_int;
type IoPmAssertionId = u32;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const IO_PM_ASSERTION_LEVEL_ON: u32 = 255;
const IO_RETURN_SUCCESS: IoReturn = 0;
const PREVENT_USER_IDLE_SYSTEM_SLEEP: &str = "PreventUserIdleSystemSleep";

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CfStringRef,
        assertion_level: u32,
        assertion_name: CfStringRef,
        assertion_id: *mut IoPmAssertionId,
    ) -> IoReturn;

    fn IOPMAssertionRelease(assertion_id: IoPmAssertionId) -> IoReturn;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        allocator: CfAllocatorRef,
        c_str: *const c_char,
        encoding: u32,
    ) -> CfStringRef;

    fn CFRelease(value: CfTypeRef);
}

/// Keeps the system awake using an `IOKit` power assertion.
pub(super) struct Inhibitor {
    assertion_id: IoPmAssertionId,
}

impl Inhibitor {
    pub(super) fn acquire(reason: &str) -> Result<Self> {
        let assertion_type = CfString::new(PREVENT_USER_IDLE_SYSTEM_SLEEP)?;
        let assertion_name = CfString::new(reason)?;

        let mut assertion_id = 0;
        let result = unsafe {
            IOPMAssertionCreateWithName(
                assertion_type.0,
                IO_PM_ASSERTION_LEVEL_ON,
                assertion_name.0,
                &raw mut assertion_id,
            )
        };

        if result != IO_RETURN_SUCCESS {
            return Err(anyhow!(
                "IOPMAssertionCreateWithName failed with IOReturn {result:#x}"
            ));
        }

        Ok(Self { assertion_id })
    }
}

impl Drop for Inhibitor {
    fn drop(&mut self) {
        unsafe {
            IOPMAssertionRelease(self.assertion_id);
        }
    }
}

struct CfString(CfStringRef);

impl CfString {
    fn new(value: &str) -> Result<Self> {
        let value = CString::new(value).context("String contains an interior null byte")?;
        let string = unsafe {
            CFStringCreateWithCString(std::ptr::null(), value.as_ptr(), CF_STRING_ENCODING_UTF8)
        };

        if string.is_null() {
            return Err(anyhow!("CFStringCreateWithCString failed"));
        }

        Ok(Self(string))
    }
}

impl Drop for CfString {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}
