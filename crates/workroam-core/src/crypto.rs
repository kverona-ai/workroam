//! Small crypto helpers shared by the client and the server.
//!
//! * Control objects: XChaCha20-Poly1305 with a sub key derived (HKDF-SHA256)
//!   from the repository master key (architecture §5, §19.1).
//! * Project IDs: first 16 bytes of HMAC-SHA256(control key, project name).
//! * Password sealing (pairing code, archive packs): scrypt + XChaCha20-Poly1305.

use crate::{Error, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub type Key = [u8; 32];

const CONTROL_MAGIC: &[u8; 4] = b"WRC1";
const SEALED_MAGIC: &[u8; 4] = b"WRP1";

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

/// Random identifier such as `d_7f3a9c21e4b0` (prefix + 12 hex chars).
pub fn random_id(prefix: &str) -> String {
    let b: [u8; 6] = random_bytes();
    format!("{prefix}_{}", hex::encode(b))
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// HKDF-SHA256 expand of `ikm` into a 32 byte sub key for `info`.
pub fn derive_key(ikm: &[u8], info: &str) -> Key {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(b"workroam/v1"), ikm);
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm)
        .expect("32 bytes is a valid HKDF length");
    okm
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key size");
    mac.update(msg);
    mac.finalize().into_bytes().into()
}

/// Project id = first 16 bytes of HMAC-SHA256(control key, project name), hex.
pub fn project_id(control_key: &Key, name: &str) -> String {
    let mac = hmac_sha256(control_key, format!("project:{name}").as_bytes());
    hex::encode(&mac[..16])
}

/// Encrypt a control object. `aad` binds the ciphertext to its object key.
pub fn seal(key: &Key, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce: [u8; 24] = random_bytes();
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("encryption cannot fail");
    let mut out = Vec::with_capacity(4 + 24 + ct.len());
    out.extend_from_slice(CONTROL_MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

pub fn open(key: &Key, aad: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 4 + 24 + 16 || &data[..4] != CONTROL_MAGIC {
        return Err(Error::new(
            crate::Code::Corrupted,
            "control object has an unknown format",
        ));
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            XNonce::from_slice(&data[4..28]),
            Payload {
                msg: &data[28..],
                aad,
            },
        )
        .map_err(|_| {
            Error::new(
                crate::Code::Corrupted,
                "control object cannot be decrypted (wrong key or tampered)",
            )
        })
}

pub fn seal_json<T: serde::Serialize>(key: &Key, aad: &str, value: &T) -> Result<Vec<u8>> {
    Ok(seal(key, aad.as_bytes(), &serde_json::to_vec(value)?))
}

pub fn open_json<T: serde::de::DeserializeOwned>(key: &Key, aad: &str, data: &[u8]) -> Result<T> {
    let plain = open(key, aad.as_bytes(), data)?;
    Ok(serde_json::from_slice(&plain)?)
}

/// scrypt parameters for password based keys (N = 2^15, r = 8, p = 1).
fn scrypt_params() -> scrypt::Params {
    scrypt::Params::new(15, 8, 1, 32).expect("valid scrypt params")
}

pub fn password_key(password: &str, salt: &[u8]) -> Key {
    let mut out = [0u8; 32];
    scrypt::scrypt(password.as_bytes(), salt, &scrypt_params(), &mut out)
        .expect("valid output length");
    out
}

/// `salt(16) | nonce(24) | ciphertext`, key = scrypt(password, salt).
/// Used by the pairing code (architecture §8.1).
pub fn password_seal_raw(password: &str, plaintext: &[u8]) -> Vec<u8> {
    let salt: [u8; 16] = random_bytes();
    let key = password_key(password, &salt);
    let cipher = XChaCha20Poly1305::new((&key).into());
    let nonce: [u8; 24] = random_bytes();
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .expect("encryption cannot fail");
    let mut out = Vec::with_capacity(16 + 24 + ct.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

pub fn password_open_raw(password: &str, data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 16 + 24 + 16 {
        return Err(Error::invalid("sealed data is too short"));
    }
    let key = password_key(password, &data[..16]);
    let cipher = XChaCha20Poly1305::new((&key).into());
    cipher
        .decrypt(XNonce::from_slice(&data[16..40]), &data[40..])
        .map_err(|_| Error::invalid("wrong password or damaged data"))
}

/// Like [`password_seal_raw`] with a magic header, for files (archive packs).
pub fn password_seal(password: &str, plaintext: &[u8]) -> Vec<u8> {
    let mut out = SEALED_MAGIC.to_vec();
    out.extend(password_seal_raw(password, plaintext));
    out
}

pub fn password_open(password: &str, data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 4 || &data[..4] != SEALED_MAGIC {
        return Err(Error::invalid("not a Workroam sealed file"));
    }
    password_open_raw(password, &data[4..])
}

/// Recovery key: 8 groups of 5 Crockford base32 characters (200 bits).
pub fn new_recovery_key() -> String {
    let b: [u8; 25] = random_bytes();
    let s = data_encoding::BASE32_NOPAD.encode(&b);
    s.as_bytes()
        .chunks(5)
        .map(|c| std::str::from_utf8(c).unwrap().to_string())
        .collect::<Vec<_>>()
        .join("-")
}

/// Normalise a recovery key typed by a user (case, spaces, dashes).
pub fn normalize_recovery_key(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    clean
        .as_bytes()
        .chunks(5)
        .map(|c| std::str::from_utf8(c).unwrap().to_string())
        .collect::<Vec<_>>()
        .join("-")
}

/// Password strength 0..=4 with a short reason key, shared by wizard and CLI.
pub fn password_strength(pw: &str) -> (u8, &'static str) {
    let len = pw.chars().count();
    let classes = [
        pw.chars().any(|c| c.is_ascii_lowercase()),
        pw.chars().any(|c| c.is_ascii_uppercase()),
        pw.chars().any(|c| c.is_ascii_digit()),
        pw.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if len < 8 {
        return (0, "password.too_short");
    }
    let mut score: u8 = match len {
        8..=11 => 1,
        12..=15 => 2,
        16..=19 => 3,
        _ => 4,
    };
    if classes >= 3 && score < 4 {
        score += 1;
    }
    if classes <= 1 && score > 1 {
        score -= 1;
    }
    let reason = match score {
        0 | 1 => "password.weak",
        2 => "password.fair",
        3 => "password.good",
        _ => "password.strong",
    };
    (score, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_roundtrip() {
        let key: Key = random_bytes();
        let ct = seal(&key, b"control/projects/x/head", b"hello");
        assert_eq!(
            open(&key, b"control/projects/x/head", &ct).unwrap(),
            b"hello"
        );
        assert!(open(&key, b"control/projects/y/head", &ct).is_err());
    }

    #[test]
    fn password_roundtrip() {
        let ct = password_seal("correct horse", b"secret");
        assert_eq!(password_open("correct horse", &ct).unwrap(), b"secret");
        assert!(password_open("wrong", &ct).is_err());
    }

    #[test]
    fn project_ids_are_stable() {
        let key = [7u8; 32];
        assert_eq!(project_id(&key, "proj-a"), project_id(&key, "proj-a"));
        assert_ne!(project_id(&key, "proj-a"), project_id(&key, "proj-b"));
        assert_eq!(project_id(&key, "proj-a").len(), 32);
    }

    #[test]
    fn recovery_key_format() {
        let k = new_recovery_key();
        assert_eq!(k.split('-').count(), 8);
        assert_eq!(
            normalize_recovery_key(&k.to_lowercase().replace('-', " ")),
            k
        );
    }

    #[test]
    fn strength() {
        assert_eq!(password_strength("short").0, 0);
        assert!(password_strength("correct-horse-battery-staple").0 >= 3);
    }
}
