//! The web reader's core (architecture §18.4, SRV-04), compiled to
//! WebAssembly: it unlocks the snapshot repository with the workspace
//! password and decrypts what the page fetched from S3. The page does the
//! network part (signed S3 requests with a read-only key); nothing
//! decrypted ever leaves the browser.
//!
//! The format is restic's (repository v2): AES-256-CTR + Poly1305-AES,
//! scrypt key files, zstd-compressed JSON and blobs. Control objects use
//! XChaCha20-Poly1305 with a key derived from the master key, like
//! `workroam-core`.

use aes256ctr_poly1305aes::aead::{Aead, KeyInit};
use aes256ctr_poly1305aes::Aes256CtrPoly1305Aes;
use base64::Engine;
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;

pub mod abi;

#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    WrongPassword,
    Format(String),
    Locked,
    Unknown(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::WrongPassword => f.write_str("wrong password or recovery key"),
            ReadError::Format(s) => write!(f, "unexpected data: {s}"),
            ReadError::Locked => f.write_str("unlock the workspace first"),
            ReadError::Unknown(s) => write!(f, "not in any index: {s}"),
        }
    }
}

type R<T> = Result<T, ReadError>;

fn fmt_err(e: impl std::fmt::Display) -> ReadError {
    ReadError::Format(e.to_string())
}

#[derive(Deserialize)]
struct KeyFile {
    #[serde(rename = "N")]
    n: u32,
    r: u32,
    p: u32,
    data: String,
    salt: String,
}

#[derive(Serialize, Deserialize)]
struct Mac {
    k: String,
    r: String,
}

/// Field order and encoding match rustic's `MasterKey` exactly: its JSON is
/// the input of the control key derivation.
#[derive(Serialize, Deserialize)]
struct MasterKey {
    mac: Mac,
    encrypt: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BlobLoc {
    pub offset: u64,
    pub length: u64,
    /// 0: stored uncompressed.
    pub uncompressed: u64,
}

#[derive(Default)]
pub struct Reader {
    master: Option<[u8; 64]>,
    control: Option<[u8; 32]>,
    /// blob id → (pack id, location)
    blobs: HashMap<String, (String, BlobLoc)>,
}

fn b64(s: &str) -> R<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(fmt_err)
}

fn aead_open(key: &[u8; 64], data: &[u8]) -> R<Vec<u8>> {
    if data.len() < 32 {
        return Err(ReadError::Format("too short".into()));
    }
    let k = aes256ctr_poly1305aes::Key::from_slice(key);
    let nonce = aes256ctr_poly1305aes::aead::Nonce::<Aes256CtrPoly1305Aes>::from_slice(&data[..16]);
    Aes256CtrPoly1305Aes::new(k)
        .decrypt(nonce, &data[16..])
        .map_err(|_| ReadError::WrongPassword)
}

fn zstd(data: &[u8]) -> R<Vec<u8>> {
    let mut d = ruzstd::decoding::StreamingDecoder::new(data).map_err(fmt_err)?;
    let mut out = Vec::new();
    d.read_to_end(&mut out).map_err(fmt_err)?;
    Ok(out)
}

/// HKDF-SHA256 as in `workroam_core::crypto::derive_key`.
fn derive_key(ikm: &[u8], info: &str) -> [u8; 32] {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(b"workroam/v1"), ikm);
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm).expect("32 bytes");
    okm
}

/// RFC 4648 base32 without padding.
fn base32(s: &str) -> Option<Vec<u8>> {
    const A: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut out, mut buf, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in s.bytes().filter(|c| *c != b'=') {
        buf = (buf << 5) | A.iter().position(|a| *a == c)? as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

impl Reader {
    pub fn new() -> Reader {
        Reader::default()
    }

    pub fn unlocked(&self) -> bool {
        self.master.is_some()
    }

    /// Try one key file (`keys/<id>`) with the password or recovery key.
    pub fn unlock(&mut self, keyfile: &[u8], secret: &str) -> R<()> {
        let kf: KeyFile = serde_json::from_slice(keyfile).map_err(fmt_err)?;
        if kf.n < 2 || !kf.n.is_power_of_two() {
            return Err(ReadError::Format("scrypt N".into()));
        }
        let params =
            scrypt::Params::new(kf.n.trailing_zeros() as u8, kf.r, kf.p).map_err(fmt_err)?;
        let mut k = [0u8; 64];
        scrypt::scrypt(secret.as_bytes(), &b64(&kf.salt)?, &params, &mut k).map_err(fmt_err)?;
        let plain = aead_open(&k, &b64(&kf.data)?)?;
        let mk: MasterKey = serde_json::from_slice(&plain).map_err(fmt_err)?;
        let (enc, mk_k, mk_r) = (b64(&mk.encrypt)?, b64(&mk.mac.k)?, b64(&mk.mac.r)?);
        if enc.len() != 32 || mk_k.len() != 16 || mk_r.len() != 16 {
            return Err(ReadError::Format("master key size".into()));
        }
        let mut master = [0u8; 64];
        master[..32].copy_from_slice(&enc);
        master[32..48].copy_from_slice(&mk_k);
        master[48..].copy_from_slice(&mk_r);
        let material = serde_json::to_vec(&mk).map_err(fmt_err)?;
        self.control = Some(derive_key(&material, "control"));
        self.master = Some(master);
        Ok(())
    }

    /// The recovery key is typed with or without dashes, in any case.
    pub fn normalize_recovery(s: &str) -> String {
        let c: String = s
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        c.as_bytes()
            .chunks(5)
            .map(|g| String::from_utf8_lossy(g).into_owned())
            .collect::<Vec<_>>()
            .join("-")
    }

    pub fn lock(&mut self) {
        self.master = None;
        self.control = None;
        self.blobs.clear();
    }

    fn master(&self) -> R<&[u8; 64]> {
        self.master.as_ref().ok_or(ReadError::Locked)
    }

    /// A whole repository file (config, snapshot, index): decrypted and,
    /// when compressed, decompressed.
    pub fn file(&self, data: &[u8]) -> R<Vec<u8>> {
        let plain = aead_open(self.master()?, data)?;
        match plain.first() {
            Some(b'{') | Some(b'[') => Ok(plain),
            Some(2) => zstd(&plain[1..]),
            _ => Err(ReadError::Format("unknown file encoding".into())),
        }
    }

    /// Learn blob locations from a decrypted index file.
    pub fn add_index(&mut self, index_json: &[u8]) -> R<usize> {
        let v: Value = serde_json::from_slice(index_json).map_err(fmt_err)?;
        let mut n = 0;
        for pack in v["packs"].as_array().into_iter().flatten() {
            let pid = pack["id"].as_str().unwrap_or_default().to_string();
            for b in pack["blobs"].as_array().into_iter().flatten() {
                let id = b["id"].as_str().unwrap_or_default().to_string();
                let loc = BlobLoc {
                    offset: b["offset"].as_u64().unwrap_or(0),
                    length: b["length"].as_u64().unwrap_or(0),
                    uncompressed: b["uncompressed_length"].as_u64().unwrap_or(0),
                };
                self.blobs.insert(id, (pid.clone(), loc));
                n += 1;
            }
        }
        Ok(n)
    }

    /// Where to fetch a blob: `data/<pack[..2]>/<pack>` bytes offset..offset+length.
    pub fn locate(&self, blob: &str) -> R<(String, BlobLoc)> {
        self.blobs
            .get(blob)
            .cloned()
            .ok_or_else(|| ReadError::Unknown(blob.to_string()))
    }

    /// Decrypt one blob read from its pack and check it against its id.
    pub fn blob(&self, blob: &str, data: &[u8]) -> R<Vec<u8>> {
        let (_, loc) = self.locate(blob)?;
        let plain = aead_open(self.master()?, data)?;
        let plain = if loc.uncompressed > 0 {
            zstd(&plain)?
        } else {
            plain
        };
        if hex::encode(Sha256::digest(&plain)) != blob {
            return Err(ReadError::Format(format!(
                "blob {blob} does not match its id"
            )));
        }
        Ok(plain)
    }

    /// A control object (`control/<key>`), e.g. `projects/<pid>/head`.
    pub fn control(&self, key: &str, data: &[u8]) -> R<Vec<u8>> {
        let k = self.control.as_ref().ok_or(ReadError::Locked)?;
        if data.len() < 4 + 24 + 16 || &data[..4] != b"WRC1" {
            return Err(ReadError::Format("control object".into()));
        }
        use chacha20poly1305::aead::Payload;
        XChaCha20Poly1305::new(k.into())
            .decrypt(
                XNonce::from_slice(&data[4..28]),
                Payload {
                    msg: &data[28..],
                    aad: format!("control/{key}").as_bytes(),
                },
            )
            .map_err(|_| ReadError::Format("control object cannot be decrypted".into()))
    }

    /// Open a `wr1:` pairing code (location and key) with the workspace
    /// password, as `workroam_device::PairingCode::decode` does.
    pub fn open_pairing(code: &str, password: &str) -> R<Vec<u8>> {
        let c: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        let body = c
            .strip_prefix("wr1:")
            .or_else(|| c.strip_prefix("WR1:"))
            .ok_or_else(|| ReadError::Format("a pairing code starts with wr1:".into()))?;
        let raw = base32(&body.to_ascii_uppercase())
            .ok_or_else(|| ReadError::Format("pairing code".into()))?;
        if raw.len() < 16 + 24 + 16 {
            return Err(ReadError::Format("pairing code is too short".into()));
        }
        let params = scrypt::Params::new(15, 8, 1).map_err(fmt_err)?;
        let mut key = [0u8; 32];
        scrypt::scrypt(password.as_bytes(), &raw[..16], &params, &mut key).map_err(fmt_err)?;
        XChaCha20Poly1305::new((&key).into())
            .decrypt(XNonce::from_slice(&raw[16..40]), &raw[40..])
            .map_err(|_| ReadError::WrongPassword)
    }

    /// Project id as in `workroam_core::crypto::project_id`.
    pub fn project_id(&self, name: &str) -> R<String> {
        use hmac::Mac;
        let k = self.control.as_ref().ok_or(ReadError::Locked)?;
        let mut mac = <hmac::Hmac<Sha256> as Mac>::new_from_slice(k).expect("any key size");
        mac.update(format!("project:{name}").as_bytes());
        Ok(hex::encode(&mac.finalize().into_bytes()[..16]))
    }
}
