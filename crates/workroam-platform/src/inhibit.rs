//! Keep the machine awake while syncing (UX-08, architecture §9.5).

/// Holds a sleep inhibitor until dropped.
pub struct SleepGuard {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    child: Option<std::process::Child>,
}

impl SleepGuard {
    pub fn acquire(reason: &str) -> SleepGuard {
        #[cfg(target_os = "linux")]
        {
            let child = std::process::Command::new("systemd-inhibit")
                .args([
                    "--what=sleep:idle:shutdown",
                    "--who=Workroam",
                    "--mode=block",
                ])
                .arg(format!("--why={reason}"))
                .args(["sleep", "infinity"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            SleepGuard { child }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = reason;
            let child = std::process::Command::new("caffeinate")
                .args(["-i", "-s", "-w"])
                .arg(std::process::id().to_string())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            SleepGuard { child }
        }
        #[cfg(windows)]
        {
            let _ = reason;
            use windows_sys::Win32::System::Power::{
                SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
            };
            // SAFETY: plain Win32 call without pointers.
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
            }
            SleepGuard {}
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = reason;
            SleepGuard {}
        }
    }
}

impl Drop for SleepGuard {
    fn drop(&mut self) {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS};
            // SAFETY: plain Win32 call without pointers.
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        }
    }
}
