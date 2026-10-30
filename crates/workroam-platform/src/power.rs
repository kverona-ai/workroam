//! System power events (architecture §13): about to sleep, woke up, shutting
//! down. `workroamd` pushes before sleep and checks for new progress after
//! waking.
//!
//! Sources: macOS IOKit power notifications (which hold the sleep for up to
//! 30 s while the handler runs), Windows suspend / resume notifications
//! (the system does not wait), Linux logind `PrepareForSleep` /
//! `PrepareForShutdown` observed through `dbus-monitor` (no waiting either).
//! Shutdown on macOS and Linux also arrives as `SIGTERM`, which the daemon
//! handles itself.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    /// The system is about to sleep. `grace` is how long the handler may
    /// run before the system sleeps anyway (zero: it will not wait).
    WillSleep { grace: Duration },
    /// The system woke up (the network may need a moment).
    DidWake,
    /// The system or the user session is shutting down.
    WillShutdown,
}

/// What macOS leaves us of its 30 s sleep window.
pub const MAC_SLEEP_GRACE: Duration = Duration::from_secs(25);

/// Deliver power events on a dedicated thread, forever. Returns `false`
/// when this platform has no usable source; the handler is then never
/// called. A handler for `WillSleep` may block for `grace` at most.
pub fn watch(handler: impl Fn(PowerEvent) + Send + 'static) -> bool {
    imp::watch(Box::new(handler))
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{PowerEvent, MAC_SLEEP_GRACE};
    use core_foundation_sys::runloop::{
        kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRun,
        CFRunLoopSourceRef,
    };
    use std::ffi::c_void;

    type IoConnect = u32;
    type IoObject = u32;
    type IoNotificationPort = *mut c_void;
    type Callback = extern "C" fn(*mut c_void, IoObject, u32, *mut c_void);

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IORegisterForSystemPower(
            refcon: *mut c_void,
            port: *mut IoNotificationPort,
            callback: Callback,
            notifier: *mut IoObject,
        ) -> IoConnect;
        fn IONotificationPortGetRunLoopSource(port: IoNotificationPort) -> CFRunLoopSourceRef;
        fn IOAllowPowerChange(kernel_port: IoConnect, notification_id: isize) -> i32;
    }

    // IOMessage.h: iokit_common_msg(0x270 / 0x280 / 0x300).
    const CAN_SYSTEM_SLEEP: u32 = 0xe000_0270;
    const SYSTEM_WILL_SLEEP: u32 = 0xe000_0280;
    const SYSTEM_HAS_POWERED_ON: u32 = 0xe000_0300;

    struct State {
        handler: Box<dyn Fn(PowerEvent) + Send>,
        port: IoConnect,
    }

    extern "C" fn callback(
        refcon: *mut c_void,
        _service: IoObject,
        message: u32,
        arg: *mut c_void,
    ) {
        // SAFETY: refcon is the leaked `State` registered below, alive for
        // the life of the thread, and only this run loop thread uses it.
        let st = unsafe { &*(refcon as *const State) };
        match message {
            CAN_SYSTEM_SLEEP => {
                // SAFETY: plain IOKit call with the port and id it gave us.
                unsafe { IOAllowPowerChange(st.port, arg as isize) };
            }
            SYSTEM_WILL_SLEEP => {
                let started = std::time::Instant::now();
                (st.handler)(PowerEvent::WillSleep {
                    grace: MAC_SLEEP_GRACE,
                });
                tracing::debug!("sleep allowed after {:?}", started.elapsed());
                // SAFETY: as above; must be called within 30 s or the
                // system sleeps anyway.
                unsafe { IOAllowPowerChange(st.port, arg as isize) };
            }
            SYSTEM_HAS_POWERED_ON => (st.handler)(PowerEvent::DidWake),
            _ => {}
        }
    }

    pub fn watch(handler: Box<dyn Fn(PowerEvent) + Send>) -> bool {
        std::thread::Builder::new()
            .name("workroam-power".into())
            .spawn(move || {
                let st = Box::into_raw(Box::new(State { handler, port: 0 }));
                let mut port: IoNotificationPort = std::ptr::null_mut();
                let mut notifier: IoObject = 0;
                // SAFETY: out-pointers to locals that outlive the call; the
                // callback only runs once the run loop below is started.
                let conn = unsafe {
                    IORegisterForSystemPower(st as *mut c_void, &mut port, callback, &mut notifier)
                };
                if conn == 0 || port.is_null() {
                    tracing::warn!("IORegisterForSystemPower failed; sleep triggers are off");
                    // SAFETY: nothing else holds the pointer.
                    drop(unsafe { Box::from_raw(st) });
                    return;
                }
                // SAFETY: the run loop has not started, so no callback races this write.
                unsafe {
                    (*st).port = conn;
                    CFRunLoopAddSource(
                        CFRunLoopGetCurrent(),
                        IONotificationPortGetRunLoopSource(port),
                        kCFRunLoopDefaultMode,
                    );
                    CFRunLoopRun();
                }
                tracing::warn!("power run loop ended; sleep triggers are off");
            })
            .is_ok()
    }
}

#[cfg(windows)]
mod imp {
    use super::PowerEvent;
    use std::ffi::c_void;
    use std::time::Duration;
    use windows_sys::Win32::System::Power::{
        RegisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    };

    struct State {
        handler: Box<dyn Fn(PowerEvent) + Send>,
    }

    unsafe extern "system" fn callback(
        context: *const c_void,
        kind: u32,
        _setting: *const c_void,
    ) -> u32 {
        // SAFETY: context is the leaked `State` registered below.
        let st = unsafe { &*(context as *const State) };
        match kind {
            // Windows gives about two seconds here: start work, do not wait.
            PBT_APMSUSPEND => (st.handler)(PowerEvent::WillSleep {
                grace: Duration::ZERO,
            }),
            PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => (st.handler)(PowerEvent::DidWake),
            _ => {}
        }
        0
    }

    pub fn watch(handler: Box<dyn Fn(PowerEvent) + Send>) -> bool {
        let st = Box::into_raw(Box::new(State { handler }));
        let mut params = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(callback),
            Context: st as *mut c_void,
        };
        // SAFETY: the parameters block outlives the call (it is copied), and
        // the context pointer stays valid because it is never freed.
        let h = unsafe {
            RegisterSuspendResumeNotification(
                &mut params as *mut _ as *mut c_void,
                DEVICE_NOTIFY_CALLBACK,
            )
        };
        if h == 0 {
            tracing::warn!("RegisterSuspendResumeNotification failed; sleep triggers are off");
            // SAFETY: registration failed, so nothing else holds the pointer.
            drop(unsafe { Box::from_raw(st) });
            return false;
        }
        true
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::PowerEvent;
    use std::io::BufRead;
    use std::time::Duration;

    enum Pending {
        Sleep,
        Shutdown,
    }

    pub fn watch(handler: Box<dyn Fn(PowerEvent) + Send>) -> bool {
        let child = std::process::Command::new("dbus-monitor")
            .args([
                "--system",
                "type='signal',sender='org.freedesktop.login1',interface='org.freedesktop.login1.Manager'",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                tracing::info!("dbus-monitor unavailable ({e}); sleep triggers are off");
                return false;
            }
        };
        let Some(out) = child.stdout.take() else {
            return false;
        };
        std::thread::Builder::new()
            .name("workroam-power".into())
            .spawn(move || {
                let mut pending: Option<Pending> = None;
                for line in std::io::BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    let t = line.trim();
                    if t.contains("member=PrepareForSleep") {
                        pending = Some(Pending::Sleep);
                    } else if t.contains("member=PrepareForShutdown") {
                        pending = Some(Pending::Shutdown);
                    } else if let Some(flag) = t.strip_prefix("boolean ") {
                        match (pending.take(), flag.trim()) {
                            (Some(Pending::Sleep), "true") => handler(PowerEvent::WillSleep {
                                grace: Duration::ZERO,
                            }),
                            (Some(Pending::Sleep), "false") => handler(PowerEvent::DidWake),
                            (Some(Pending::Shutdown), "true") => handler(PowerEvent::WillShutdown),
                            _ => {}
                        }
                    } else if t.starts_with("signal ") || t.starts_with("method ") {
                        pending = None;
                    }
                }
                let _ = child.wait();
                tracing::info!("dbus-monitor ended; sleep triggers are off");
            })
            .is_ok()
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod imp {
    use super::PowerEvent;
    pub fn watch(_handler: Box<dyn Fn(PowerEvent) + Send>) -> bool {
        false
    }
}
