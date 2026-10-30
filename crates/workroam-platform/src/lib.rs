//! Cross-platform layer (architecture §14): path checks, atomic replace,
//! links, sleep inhibition, power events, autostart service, keychain and
//! system info.

pub mod dircheck;
pub mod fsutil;
pub mod inhibit;
pub mod power;
pub mod secrets;
pub mod service;
pub mod sysinfo;
pub mod xp;
