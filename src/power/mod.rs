//! Prevents the operating system from sleeping during long-running operations.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux::Inhibitor as PlatformInhibitor;

#[cfg(target_os = "macos")]
use macos::Inhibitor as PlatformInhibitor;

#[cfg(windows)]
use windows::Inhibitor as PlatformInhibitor;

use anyhow::Result;

/// Keeps the system awake for as long as the value is alive.
///
/// Dropping the inhibitor releases the platform-specific request. Acquisition is
/// best-effort: if the platform cannot honour it (for example, a Linux host without
/// `systemd-inhibit`), an error is returned and the caller may continue without it.
pub struct SleepInhibitor {
    _platform: PlatformInhibitor,
}

impl SleepInhibitor {
    pub fn acquire(reason: &str) -> Result<Self> {
        Ok(Self {
            _platform: PlatformInhibitor::acquire(reason)?,
        })
    }
}
