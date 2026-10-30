//! Storage access for the control objects (architecture §5) and raw S3
//! calls used by setup and checks (bucket listing/creation, probes).

pub mod control;
pub mod location;
pub mod object;
pub mod s3raw;
pub mod sigv4;

pub use control::Control;
pub use location::{detect_region, presets, validate_bucket_name, Preset};
pub use object::{PutOutcome, Store, StoredObject};
pub use s3raw::S3Client;

/// Credentials for one storage endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creds {
    pub access_key: String,
    pub secret_key: String,
}
