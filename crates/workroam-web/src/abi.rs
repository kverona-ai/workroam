//! A small C ABI for the page (no wasm-bindgen): the page copies inputs
//! into memory from `wr_alloc`, calls a function, and reads the result
//! from `wr_out_ptr` / `wr_out_len`. Functions return 0 on success, 1 for
//! a wrong password, 2 for other errors (the message is the output).

use crate::{ReadError, Reader};
use std::sync::Mutex;

static READER: Mutex<Option<Reader>> = Mutex::new(None);
static OUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn with<T>(f: impl FnOnce(&mut Reader) -> Result<T, ReadError>) -> Result<T, ReadError> {
    let mut g = READER.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Reader::new))
}

fn finish(r: Result<Vec<u8>, ReadError>) -> i32 {
    let mut out = OUT.lock().unwrap_or_else(|e| e.into_inner());
    match r {
        Ok(v) => {
            *out = v;
            0
        }
        Err(e) => {
            *out = e.to_string().into_bytes();
            if e == ReadError::WrongPassword {
                1
            } else {
                2
            }
        }
    }
}

/// # Safety
/// `ptr..ptr+len` must be memory the page wrote through `wr_alloc`.
unsafe fn slice<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

unsafe fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    std::str::from_utf8(slice(ptr, len)).unwrap_or("")
}

#[no_mangle]
pub extern "C" fn wr_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` come from one `wr_alloc` call.
#[no_mangle]
pub unsafe extern "C" fn wr_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

#[no_mangle]
pub extern "C" fn wr_out_ptr() -> *const u8 {
    OUT.lock().unwrap_or_else(|e| e.into_inner()).as_ptr()
}

#[no_mangle]
pub extern "C" fn wr_out_len() -> usize {
    OUT.lock().unwrap_or_else(|e| e.into_inner()).len()
}

/// # Safety
/// Pointers from `wr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn wr_unlock(
    kf: *const u8,
    kf_len: usize,
    pw: *const u8,
    pw_len: usize,
) -> i32 {
    let (kf, pw) = (slice(kf, kf_len), text(pw, pw_len));
    finish(with(|r| {
        r.unlock(kf, pw)
            .or_else(|e| match e {
                ReadError::WrongPassword => r.unlock(kf, &Reader::normalize_recovery(pw)),
                e => Err(e),
            })
            .map(|_| Vec::new())
    }))
}

#[no_mangle]
pub extern "C" fn wr_lock() {
    let _ = with(|r| {
        r.lock();
        Ok(())
    });
}

/// # Safety
/// Pointers from `wr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn wr_file(p: *const u8, len: usize) -> i32 {
    let d = slice(p, len);
    finish(with(|r| r.file(d)))
}

/// Decrypt an index file and learn its blobs; output: the decrypted JSON.
/// # Safety
/// Pointers from `wr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn wr_index(p: *const u8, len: usize) -> i32 {
    let d = slice(p, len);
    finish(with(|r| {
        let plain = r.file(d)?;
        let n = r.add_index(&plain)?;
        Ok(n.to_string().into_bytes())
    }))
}

/// Output: `{"pack": …, "offset": …, "length": …}`.
/// # Safety
/// Pointers from `wr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn wr_locate(id: *const u8, id_len: usize) -> i32 {
    let id = text(id, id_len).to_string();
    finish(with(|r| {
        let (pack, loc) = r.locate(&id)?;
        Ok(
            serde_json::json!({"pack": pack, "offset": loc.offset, "length": loc.length})
                .to_string()
                .into_bytes(),
        )
    }))
}

/// # Safety
/// Pointers from `wr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn wr_blob(id: *const u8, id_len: usize, p: *const u8, len: usize) -> i32 {
    let (id, d) = (text(id, id_len).to_string(), slice(p, len));
    finish(with(|r| r.blob(&id, d)))
}

/// # Safety
/// Pointers from `wr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn wr_control(k: *const u8, k_len: usize, p: *const u8, len: usize) -> i32 {
    let (k, d) = (text(k, k_len).to_string(), slice(p, len));
    finish(with(|r| r.control(&k, d)))
}

/// # Safety
/// Pointers from `wr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn wr_project_id(n: *const u8, len: usize) -> i32 {
    let n = text(n, len).to_string();
    finish(with(|r| r.project_id(&n).map(String::into_bytes)))
}

/// Output: the pairing payload JSON (endpoint, bucket, prefix, keys…).
/// # Safety
/// Pointers from `wr_alloc`.
#[no_mangle]
pub unsafe extern "C" fn wr_open_pairing(
    c: *const u8,
    c_len: usize,
    pw: *const u8,
    pw_len: usize,
) -> i32 {
    finish(Reader::open_pairing(text(c, c_len), text(pw, pw_len)))
}
