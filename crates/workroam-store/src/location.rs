//! Storage presets, region / addressing detection and bucket name rules
//! (ONB-03, ONB-05, architecture §7.4).

use serde::{Deserialize, Serialize};
use workroam_core::config::Addressing;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub id: &'static str,
    pub label_key: &'static str,
    pub endpoint_hint: &'static str,
    pub addressing: Addressing,
    pub default_region: &'static str,
    /// Can Workroam create a scoped key through an admin API?
    pub can_create_key: bool,
}

pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            id: "rustfs",
            label_key: "preset.rustfs",
            endpoint_hint: "https://sync.example.com",
            addressing: Addressing::Path,
            default_region: "us-east-1",
            can_create_key: true,
        },
        Preset {
            id: "aws",
            label_key: "preset.aws",
            endpoint_hint: "https://s3.us-east-1.amazonaws.com",
            addressing: Addressing::VirtualHost,
            default_region: "us-east-1",
            can_create_key: true,
        },
        Preset {
            id: "r2",
            label_key: "preset.r2",
            endpoint_hint: "https://<account-id>.r2.cloudflarestorage.com",
            addressing: Addressing::Path,
            default_region: "auto",
            can_create_key: false,
        },
        Preset {
            id: "oss",
            label_key: "preset.oss",
            endpoint_hint: "https://oss-cn-hangzhou.aliyuncs.com",
            addressing: Addressing::VirtualHost,
            default_region: "cn-hangzhou",
            can_create_key: false,
        },
        Preset {
            id: "other",
            label_key: "preset.other",
            endpoint_hint: "https://s3.example.com",
            addressing: Addressing::Path,
            default_region: "us-east-1",
            can_create_key: false,
        },
        Preset {
            id: "local",
            label_key: "preset.local",
            endpoint_hint: "file:///mnt/nas/workroam",
            addressing: Addressing::Path,
            default_region: "",
            can_create_key: false,
        },
    ]
}

pub fn preset(id: &str) -> Option<Preset> {
    presets().into_iter().find(|p| p.id == id)
}

/// Region from well known endpoint shapes; `None` when it must be probed.
pub fn detect_region(endpoint: &str) -> Option<String> {
    let host = url::Url::parse(endpoint).ok()?.host_str()?.to_string();
    if host.ends_with(".r2.cloudflarestorage.com") {
        return Some("auto".into());
    }
    if let Some(rest) = host.strip_suffix(".amazonaws.com") {
        // s3.us-west-2 / s3-us-west-2 / s3 / bucket.s3.eu-central-1
        let parts: Vec<&str> = rest.split('.').collect();
        for (i, p) in parts.iter().enumerate() {
            if *p == "s3" {
                return Some(
                    parts
                        .get(i + 1)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "us-east-1".into()),
                );
            }
            if let Some(r) = p.strip_prefix("s3-") {
                return Some(r.to_string());
            }
        }
    }
    if let Some(rest) = host.strip_suffix(".aliyuncs.com") {
        let first = rest.rsplit('.').next().unwrap_or(rest);
        if let Some(r) = first.strip_prefix("oss-") {
            return Some(r.trim_end_matches("-internal").to_string());
        }
    }
    None
}

/// Addressing guess before probing: IPs, localhost and custom ports use path style.
pub fn guess_addressing(endpoint: &str) -> Addressing {
    let Ok(u) = url::Url::parse(endpoint) else {
        return Addressing::Path;
    };
    let host = u.host_str().unwrap_or("");
    if host.ends_with(".amazonaws.com") || host.ends_with(".aliyuncs.com") {
        return Addressing::VirtualHost;
    }
    Addressing::Path
}

/// S3 bucket naming rules (3-63, lowercase letters, digits, hyphens, dots
/// discouraged, not starting/ending with a hyphen, not an IP address).
pub fn validate_bucket_name(name: &str) -> Result<(), &'static str> {
    if name.len() < 3 || name.len() > 63 {
        return Err("bucket.invalid");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("bucket.invalid");
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("bucket.invalid");
    }
    if name.parse::<std::net::Ipv4Addr>().is_ok() {
        return Err("bucket.invalid");
    }
    if name.starts_with("xn--") || name.ends_with("-s3alias") {
        return Err("bucket.invalid");
    }
    Ok(())
}

/// `workroam-<device>-<4 random>` (ONB-05).
pub fn default_bucket_name(device: &str) -> String {
    let mut d: String = device
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while d.contains("--") {
        d = d.replace("--", "-");
    }
    let d = d.trim_matches('-');
    let d = if d.len() > 40 { &d[..40] } else { d };
    let d = d.trim_matches('-');
    const ALPHA: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let rnd: [u8; 4] = workroam_core::crypto::random_bytes();
    let suffix: String = rnd
        .iter()
        .map(|b| ALPHA[(*b as usize) % ALPHA.len()] as char)
        .collect();
    if d.is_empty() {
        format!("workroam-{suffix}")
    } else {
        format!("workroam-{d}-{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions() {
        assert_eq!(
            detect_region("https://s3.eu-central-1.amazonaws.com").as_deref(),
            Some("eu-central-1")
        );
        assert_eq!(
            detect_region("https://s3.amazonaws.com").as_deref(),
            Some("us-east-1")
        );
        assert_eq!(
            detect_region("https://abc.r2.cloudflarestorage.com").as_deref(),
            Some("auto")
        );
        assert_eq!(
            detect_region("https://oss-cn-hangzhou.aliyuncs.com").as_deref(),
            Some("cn-hangzhou")
        );
        assert_eq!(detect_region("https://sync.example.com"), None);
    }

    #[test]
    fn bucket_names() {
        assert!(validate_bucket_name("workroam-office-8k2d").is_ok());
        assert!(validate_bucket_name("ab").is_err());
        assert!(validate_bucket_name("-abc").is_err());
        assert!(validate_bucket_name("Abc").is_err());
        assert!(validate_bucket_name("192.168.1.1").is_err());
        let n = default_bucket_name("Office Win");
        assert!(n.starts_with("workroam-office-win-"), "{n}");
        assert!(validate_bucket_name(&n).is_ok());
    }
}
