//! Devices and credentials (architecture §8): pairing codes, the device
//! registry, scoped key provisioning per storage type and key rotation.

pub mod madmin;
pub mod pairing;
pub mod provision;
pub mod qr;
pub mod registry;
pub mod rotation;

pub use pairing::{PairingCode, PairingPayload};
pub use provision::{provisioner_for, AdminCreds, KeyProvisioner, ProvisionedKey};
