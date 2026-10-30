//! MinIO-compatible admin payload encryption (used by MinIO and RustFS for
//! service-account calls): salt(32) ‖ id(1) ‖ nonce(8) ‖ sio stream, where the
//! key is Argon2id(password, salt) and the stream is the DARE "sio" format
//! with 16 KiB fragments.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::Aes256Gcm;
use chacha20poly1305::ChaCha20Poly1305;
use workroam_core::{Error, Result};

const BUF: usize = 1 << 14;
const ARGON2ID_AES_GCM: u8 = 0x00;
const ARGON2ID_CHACHA: u8 = 0x01;
const PBKDF2_AES_GCM: u8 = 0x02;

fn derive(id: u8, password: &[u8], salt: &[u8]) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    match id {
        ARGON2ID_AES_GCM | ARGON2ID_CHACHA => {
            let params = argon2::Params::new(64 * 1024, 1, 4, Some(32))
                .map_err(|e| Error::internal(e.to_string()))?;
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
                .hash_password_into(password, salt, &mut key)
                .map_err(|e| Error::internal(e.to_string()))?;
        }
        PBKDF2_AES_GCM => pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, 8192, &mut key),
        _ => return Err(Error::invalid("unknown admin payload encryption")),
    }
    Ok(key)
}

enum Cipher {
    Aes(Box<Aes256Gcm>),
    Chacha(Box<ChaCha20Poly1305>),
}

impl Cipher {
    fn new(id: u8, key: &[u8; 32]) -> Cipher {
        match id {
            ARGON2ID_CHACHA => Cipher::Chacha(Box::new(ChaCha20Poly1305::new(key.into()))),
            _ => Cipher::Aes(Box::new(Aes256Gcm::new(key.into()))),
        }
    }
    fn seal(&self, nonce: &[u8; 12], msg: &[u8], aad: &[u8]) -> Vec<u8> {
        let p = Payload { msg, aad };
        match self {
            Cipher::Aes(c) => c.encrypt(nonce.into(), p),
            Cipher::Chacha(c) => c.encrypt(nonce.into(), p),
        }
        .expect("in-memory AEAD")
    }
    fn open(&self, nonce: &[u8; 12], msg: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let p = Payload { msg, aad };
        match self {
            Cipher::Aes(c) => c.decrypt(nonce.into(), p),
            Cipher::Chacha(c) => c.decrypt(nonce.into(), p),
        }
        .map_err(|_| {
            Error::new(
                workroam_core::Code::Denied,
                "admin payload could not be decrypted",
            )
        })
    }
}

fn nonce_for(base: &[u8; 8], seq: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(base);
    n[8..].copy_from_slice(&seq.to_le_bytes());
    n
}

fn header_ad(c: &Cipher, base: &[u8; 8]) -> Vec<u8> {
    let mut ad = vec![0u8];
    ad.extend(c.seal(&nonce_for(base, 0), &[], &[]));
    ad
}

pub fn encrypt(password: &str, data: &[u8]) -> Result<Vec<u8>> {
    encrypt_with(ARGON2ID_AES_GCM, password, data)
}

fn encrypt_with(id: u8, password: &str, data: &[u8]) -> Result<Vec<u8>> {
    let salt: [u8; 32] = workroam_core::crypto::random_bytes();
    let base: [u8; 8] = workroam_core::crypto::random_bytes();
    let key = derive(id, password.as_bytes(), &salt)?;
    let c = Cipher::new(id, &key);
    let mut ad = header_ad(&c, &base);
    let mut out = Vec::with_capacity(41 + data.len() + 32);
    out.extend_from_slice(&salt);
    out.push(id);
    out.extend_from_slice(&base);
    // Every fragment holds 16 KiB except the last one, which is marked final
    // (an empty final fragment only for empty input).
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[][..]]
    } else {
        data.chunks(BUF).collect()
    };
    for (i, chunk) in chunks.iter().enumerate() {
        if i + 1 == chunks.len() {
            ad[0] = 0x80;
        }
        out.extend(c.seal(&nonce_for(&base, i as u32 + 1), chunk, &ad));
    }
    Ok(out)
}

pub fn decrypt(password: &str, data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 41 {
        return Err(Error::invalid("admin payload too short"));
    }
    let (salt, rest) = data.split_at(32);
    let id = rest[0];
    let mut base = [0u8; 8];
    base.copy_from_slice(&rest[1..9]);
    let mut body = &rest[9..];
    let key = derive(id, password.as_bytes(), salt)?;
    let c = Cipher::new(id, &key);
    let mut ad = header_ad(&c, &base);
    let frag = BUF + 16;
    let mut out = Vec::new();
    let mut seq = 1u32;
    loop {
        if body.len() > frag {
            out.extend(c.open(&nonce_for(&base, seq), &body[..frag], &ad)?);
            body = &body[frag..];
            seq += 1;
        } else {
            ad[0] = 0x80;
            out.extend(c.open(&nonce_for(&base, seq), body, &ad)?);
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_sizes() {
        for n in [0usize, 5, BUF, BUF + 1, 3 * BUF] {
            let data: Vec<u8> = (0..n).map(|i| i as u8).collect();
            let id = if n % 2 == 0 {
                PBKDF2_AES_GCM
            } else {
                ARGON2ID_CHACHA
            };
            let enc = encrypt_with(id, "secret", &data).unwrap();
            assert_eq!(decrypt("secret", &enc).unwrap(), data, "n={n}");
            assert!(decrypt("other", &enc).is_err());
        }
    }
}
