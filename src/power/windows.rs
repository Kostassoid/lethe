use anyhow::{Result, anyhow};
use windows::Win32::System::Power::{
    ES_CONTINUOUS, ES_SYSTEM_REQUIRED, EXECUTION_STATE, SetThreadExecutionState,
};

/// Keeps the system awake using the calling thread's execution state.
///
/// `SetThreadExecutionState` is thread-scoped, so the inhibitor must be created and
/// dropped on the same thread that performs the work.
pub(super) struct Inhibitor;

impl Inhibitor {
    pub(super) fn acquire(_reason: &str) -> Result<Self> {
        let flags = ES_CONTINUOUS | ES_SYSTEM_REQUIRED;
        let previous = unsafe { SetThreadExecutionState(flags) };

        if previous == EXECUTION_STATE(0) {
            return Err(anyhow!(
                "SetThreadExecutionState failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(Self)
    }
}

impl Drop for Inhibitor {
    fn drop(&mut self) {
        // Passing ES_CONTINUOUS alone clears the previously requested state.
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}
