//! `dl_iterate_phdr`, which Rust's standard library links against (to symbolize panic
//! backtraces) and which Android 4.4 (API 19, NDK r25c) does not export; it is exported from
//! API 21. Defined here, one library for API 19 also runs on 6.0. The same shim is in
//! glass-services (`crates/glassd/src/compat.rs`). Built only for Android.

use std::ffi::{c_int, c_void};

/// Reports no loaded objects, so a panic backtrace shows addresses without names.
#[no_mangle]
pub extern "C" fn dl_iterate_phdr(
    _callback: Option<unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> c_int>,
    _data: *mut c_void,
) -> c_int {
    0
}
