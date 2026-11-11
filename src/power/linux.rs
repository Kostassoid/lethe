use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};

use anyhow::{Context, Result};

const INHIBITOR_COMMAND: &str = "systemd-inhibit";

/// Keeps the system awake by holding a logind inhibitor lock.
///
/// logind's inhibitor API is only reachable over D-Bus, so the lock is acquired by
/// running `systemd-inhibit` for the lifetime of a child process. The child blocks on
/// its standard input; closing that pipe when the inhibitor is dropped lets it exit and
/// release the lock.
pub(super) struct Inhibitor {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Inhibitor {
    pub(super) fn acquire(reason: &str) -> Result<Self> {
        let mut command = Command::new(INHIBITOR_COMMAND);
        command
            .arg("--what=sleep:idle:handle-lid-switch")
            .arg("--who=Lethe")
            .arg("--mode=block")
            .arg(format!("--why={reason}"))
            .arg("--")
            .arg("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Make sure the helper cannot outlive this process even if the process is
        // terminated without running destructors (for example, on confirmation abort).
        let parent_pid = std::process::id();
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                    return Err(std::io::Error::last_os_error());
                }

                if libc::getppid() != parent_pid.cast_signed() {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }

                Ok(())
            });
        }

        let mut child = command.spawn().context("Unable to start systemd-inhibit")?;

        let stdin = child.stdin.take();

        Ok(Self { child, stdin })
    }
}

impl Drop for Inhibitor {
    fn drop(&mut self) {
        // `cat` exits on EOF, which makes `systemd-inhibit` release the inhibitor lock
        // and exit in turn.
        drop(self.stdin.take());

        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
