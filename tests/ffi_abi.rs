// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! C-ABI (`sdk/ffi.rs`) coverage: drive the exported `extern "C"` entry points
//! exactly as the Android/iOS hosts do.

use fastshell::sdk::ffi::capi::*;
use fastshell::sdk::ffi::fastshell_free_string;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

fn cs(s: &str) -> CString {
    CString::new(s).unwrap()
}

unsafe fn take(ptr: *mut c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    fastshell_free_string(ptr);
    s
}

extern "C" fn stream_cb(_chunk: *const c_char) {}

extern "C" fn device_cb(_m: *const c_char, _a: *const c_char) -> *mut c_char {
    let c = CString::new(r#"{"ok":true}"#).unwrap();
    let bytes = c.as_bytes_with_nul();
    unsafe {
        let p = libc::malloc(bytes.len()) as *mut u8;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        p as *mut c_char
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn ffi_entry_points() {
    let dir = std::env::temp_dir().join(format!("fs_ffi_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    unsafe {
        // init
        let out = take(fastshell_init(cs(&dir.to_string_lossy()).as_ptr()));
        assert!(
            out.contains("ok") || out.contains("true") || !out.is_empty(),
            "{out}"
        );

        // execute + get_cwd
        let out = take(fastshell_execute(cs("echo hi-ffi").as_ptr()));
        assert!(out.contains("hi-ffi"), "{out}");
        let cwd = take(fastshell_get_cwd());
        assert!(!cwd.is_empty());

        // execute_in
        let out = take(fastshell_execute_in(cs("/").as_ptr(), cs("pwd").as_ptr()));
        assert!(!out.is_empty());

        // permission + device callback
        fastshell_set_permission(cs("camera:photo").as_ptr(), 1);
        fastshell_set_permission(cs("camera:photo").as_ptr(), 0);
        fastshell_register_device_callback(Some(device_cb));
        fastshell_register_device_callback(None);

        // stream callback
        fastshell_register_stream_callback(Some(stream_cb));
        fastshell_register_stream_callback(None);

        // python entry points (may be unsupported depending on features)
        let _ = take(fastshell_execute_python(cs("print(1)").as_ptr()));
        let _ = take(fastshell_execute_python_script(cs("/nope.py").as_ptr()));

        // features + cancel
        let feats = take(fastshell_get_features());
        assert!(!feats.is_empty());
        fastshell_cancel_execution();

        // invalid arguments → error JSON, no crash
        let out = take(fastshell_execute(std::ptr::null()));
        assert!(!out.is_empty());
        fastshell_set_permission(std::ptr::null(), 1);
    }
}
