//! The C ABI the platform layer calls. One store per process, opened at the
//! app's start (`robledo_open`, which also decides the bundle to launch);
//! staging and `ready` come later from JS through the JSI module. Every
//! function catches panics: unwinding into C would abort the app.

use std::path::Path;
use std::sync::Mutex;

use crate::{parse_config, Store};

static STORE: Mutex<Option<Store>> = Mutex::new(None);

#[repr(C)]
pub struct RobledoBuf {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl RobledoBuf {
    fn from_string(s: String) -> RobledoBuf {
        let mut v = s.into_bytes();
        let buf = RobledoBuf {
            ptr: v.as_mut_ptr(),
            len: v.len(),
            cap: v.capacity(),
        };
        std::mem::forget(v);
        buf
    }
}

/// # Safety
/// `buf` must come from this library and be freed once.
#[no_mangle]
pub unsafe extern "C" fn robledo_buf_free(buf: RobledoBuf) {
    if !buf.ptr.is_null() {
        drop(Vec::from_raw_parts(buf.ptr, buf.len, buf.cap));
    }
}

unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Run `f` on the open store: 0 and its result, or 1 and the error.
fn with_store(out: *mut RobledoBuf, f: impl FnOnce(&mut Store) -> Result<String, String>) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut guard = STORE.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_mut() {
            Some(store) => f(store),
            None => Err("robledo is not open: the app was built without `mobile.updates` in reamrc.ts, or its start could not open the store".to_string()),
        }
    }))
    .unwrap_or_else(|_| Err("robledo panicked".to_string()));
    let (code, text) = match result {
        Ok(text) => (0, text),
        Err(error) => (1, error),
    };
    if !out.is_null() {
        // SAFETY: the caller passes a valid out-pointer.
        unsafe { *out = RobledoBuf::from_string(text) };
    }
    code
}

/// At the app's start: open the store in `dir` with the app's config, roll
/// back or promote, and return in `out` the bundle to launch ("" for the one
/// inside the app). Returns 0, or 1 with the error in `out`.
///
/// # Safety
/// The pointers must hold their lengths in bytes; `out` must be valid.
#[no_mangle]
pub unsafe extern "C" fn robledo_open(
    dir: *const u8,
    dir_len: usize,
    config: *const u8,
    config_len: usize,
    out: *mut RobledoBuf,
) -> i32 {
    let dir = String::from_utf8_lossy(bytes(dir, dir_len)).to_string();
    let config = bytes(config, config_len).to_vec();
    let opened = std::panic::catch_unwind(|| {
        let config = parse_config(&config)?;
        let mut store = Store::open(Path::new(&dir), config)?;
        let launch = store.launch()?;
        Ok::<_, String>((store, launch))
    })
    .unwrap_or_else(|_| Err("robledo panicked".to_string()));
    match opened {
        Ok((store, launch)) => {
            *STORE.lock().unwrap_or_else(|p| p.into_inner()) = Some(store);
            let path = launch
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            *out = RobledoBuf::from_string(path);
            0
        }
        Err(error) => {
            *out = RobledoBuf::from_string(error);
            1
        }
    }
}

/// Verify and stage an update: the manifest, its base64 signature, and
/// `count` files (path, bytes). Returns 0 with its id in `out`, or 1 with why
/// it was refused.
///
/// # Safety
/// Each pointer must hold its length; the arrays must hold `count` items.
#[no_mangle]
pub unsafe extern "C" fn robledo_stage(
    manifest: *const u8,
    manifest_len: usize,
    signature: *const u8,
    signature_len: usize,
    count: usize,
    paths: *const *const u8,
    path_lens: *const usize,
    data: *const *const u8,
    data_lens: *const usize,
    out: *mut RobledoBuf,
) -> i32 {
    let manifest = bytes(manifest, manifest_len).to_vec();
    let signature = String::from_utf8_lossy(bytes(signature, signature_len)).to_string();
    let mut files = Vec::with_capacity(count);
    for i in 0..count {
        let path = String::from_utf8_lossy(bytes(*paths.add(i), *path_lens.add(i))).to_string();
        files.push((path, bytes(*data.add(i), *data_lens.add(i)).to_vec()));
    }
    with_store(out, |store| store.stage(&manifest, &signature, &files))
}

/// The app showed its first screen. Returns 0, or 1 with the error in `out`.
///
/// # Safety
/// `out` must be valid or null.
#[no_mangle]
pub unsafe extern "C" fn robledo_ready(out: *mut RobledoBuf) -> i32 {
    with_store(out, |store| store.ready().map(|()| String::new()))
}

/// The status as JSON (see `Store::status`).
///
/// # Safety
/// `out` must be valid.
#[no_mangle]
pub unsafe extern "C" fn robledo_status(out: *mut RobledoBuf) -> i32 {
    with_store(out, |store| Ok(store.status().to_string()))
}
