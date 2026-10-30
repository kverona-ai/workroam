//! AWS Signature Version 4, used for the raw S3 / IAM calls that OpenDAL
//! does not cover (bucket management, probes, key provisioning).

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// RFC 3986 encoding as required by SigV4 (`/` kept when `keep_slash`).
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric()
            || matches!(c, '-' | '_' | '.' | '~')
            || (keep_slash && c == '/')
        {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub struct SignInput<'a> {
    pub method: &'a str,
    pub host: &'a str,
    /// Already URI-encoded path, starting with `/`.
    pub path: &'a str,
    pub query: &'a BTreeMap<String, String>,
    pub headers: &'a BTreeMap<String, String>,
    pub payload_sha256: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub now: DateTime<Utc>,
}

/// Returns the headers to add: `authorization`, `x-amz-date`, `x-amz-content-sha256`, `host`.
pub fn sign(input: &SignInput) -> BTreeMap<String, String> {
    let amz_date = input.now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = input.now.format("%Y%m%d").to_string();
    let mut headers: BTreeMap<String, String> = input
        .headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.trim().to_string()))
        .collect();
    headers.insert("host".into(), input.host.to_string());
    headers.insert("x-amz-date".into(), amz_date.clone());
    headers.insert(
        "x-amz-content-sha256".into(),
        input.payload_sha256.to_string(),
    );
    let canonical_query = input
        .query
        .iter()
        .map(|(k, v)| format!("{}={}", uri_encode(k, false), uri_encode(v, false)))
        .collect::<Vec<_>>()
        .join("&");
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        input.method,
        input.path,
        canonical_query,
        canonical_headers,
        signed_headers,
        input.payload_sha256
    );
    let scope = format!("{date}/{}/{}/aws4_request", input.region, input.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac(
        format!("AWS4{}", input.secret_key).as_bytes(),
        date.as_bytes(),
    );
    let k_region = hmac(&k_date, input.region.as_bytes());
    let k_service = hmac(&k_region, input.service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));
    let auth = format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}", input.access_key);
    let mut out = BTreeMap::new();
    out.insert("authorization".into(), auth);
    out.insert("x-amz-date".into(), amz_date);
    out.insert(
        "x-amz-content-sha256".into(),
        input.payload_sha256.to_string(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// AWS documented example: GET object with Range header.
    #[test]
    fn aws_example_get_object() {
        let mut headers = BTreeMap::new();
        headers.insert("range".to_string(), "bytes=0-9".to_string());
        let query = BTreeMap::new();
        let now = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let out = sign(&SignInput {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            path: "/test.txt",
            query: &query,
            headers: &headers,
            payload_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            region: "us-east-1",
            service: "s3",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            now,
        });
        assert!(
            out["authorization"].ends_with(
                "Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
            ),
            "{}",
            out["authorization"]
        );
    }
}
