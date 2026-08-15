// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! End-to-end tests for the JavaScript/WebView bridge (agent path):
//!   * `node`/`js` execution round-trips through the device callback to a
//!     simulated host WebView (V8/JavaScriptCore),
//!   * `node -c` / `jscheck` static syntax checking (feature `js-oxc`),
//!   * `render` forwards HTML + output path and surfaces host errors,
//!   * error propagation when the host rejects a call.

use fastshell::sdk::device_callback::{set_global_device_callback, DeviceCallbackFn};
use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::ffi::{c_char, CStr, CString};
use std::sync::Mutex;

static LAST_CALL: Mutex<Option<(String, String)>> = Mutex::new(None);

extern "C" fn fake_host(method: *const c_char, args: *const c_char) -> *mut c_char {
    let m = unsafe { CStr::from_ptr(method).to_string_lossy().into_owned() };
    let a = unsafe { CStr::from_ptr(args).to_string_lossy().into_owned() };
    *LAST_CALL.lock().unwrap() = Some((m.clone(), a.clone()));
    let resp = match m.as_str() {
        "eval_js" => {
            if a.contains("boom") {
                r#"{"ok":false,"error":"host js engine rejected code"}"#.to_string()
            } else {
                r#"{"result":"EVAL:ok"}"#.to_string()
            }
        }
        "render_html" => r#"{"ok":true}"#.to_string(),
        _ => r#"{"ok":true}"#.to_string(),
    };
    let c = CString::new(resp).unwrap();
    unsafe { libc::strdup(c.as_ptr()) }
}

fn mk(tag: &str) -> Fastshell {
    let mut s = Fastshell::new();
    let dir = std::env::temp_dir().join(format!("fs_jstest_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.init(Config {
        sandbox_path: dir.to_string_lossy().into(),
        python_enabled: false,
        ..Default::default()
    })
    .unwrap();
    s
}

fn last_args() -> String {
    LAST_CALL.lock().unwrap().clone().map(|(_, a)| a).unwrap_or_default()
}

/// Serialized: these tests share the process-global callback + statics.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn node_eval_roundtrips_through_host() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_global_device_callback(Some(fake_host as DeviceCallbackFn));

    let s = mk("eval");
    let out = s.execute("node -e \"1 + 1\"");
    assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.contains("EVAL:ok"), "stdout={}", out.stdout);
    assert!(last_args().contains("1 + 1"), "code must be forwarded: {}", last_args());

    set_global_device_callback(None);
}

#[test]
fn node_runs_script_file_through_host() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_global_device_callback(Some(fake_host as DeviceCallbackFn));

    let s = mk("script");
    s.write_file("script.js", "console.log('hi')").unwrap();
    let out = s.execute("node script.js");
    assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.contains("EVAL:ok"), "stdout={}", out.stdout);
    assert!(last_args().contains("console.log('hi')"), "{}", last_args());

    // Missing file → clear error, no host call.
    let out = s.execute("node missing.js");
    assert_ne!(out.exit_code, 0, "missing file must fail");

    set_global_device_callback(None);
}

#[test]
fn eval_js_host_error_propagates() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_global_device_callback(Some(fake_host as DeviceCallbackFn));

    let s = mk("err");
    let out = s.execute("node -e \"boom()\"");
    assert_ne!(out.exit_code, 0, "host rejection must fail the command");
    assert!(out.stderr.contains("host js engine rejected"), "stderr={}", out.stderr);

    set_global_device_callback(None);
}

#[test]
fn render_forwards_html_and_path() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_global_device_callback(Some(fake_host as DeviceCallbackFn));

    let s = mk("render");
    let sandbox = s.vfs_root();
    s.write_file("page.html", "<html><body>hi</body></html>").unwrap();
    let out = s.execute("render -o /shot.png /page.html");
    assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
    let args = last_args();
    assert!(args.contains("<html><body>hi</body></html>"), "html forwarded: {args}");
    assert!(args.contains(&sandbox), "path resolved into sandbox: {args}");
    assert!(args.contains("shot.png"), "{args}");

    set_global_device_callback(None);
}

#[cfg(feature = "js-oxc")]
#[test]
fn node_check_and_jscheck_report_syntax_errors() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_global_device_callback(Some(fake_host as DeviceCallbackFn));

    let s = mk("check");
    s.write_file("good.js", "const x = 1;").unwrap();
    s.write_file("bad.js", "const = 1;").unwrap();

    // `node -c` syntax-checks without touching the host engine.
    let out = s.execute("node -c good.js");
    assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.contains("syntax OK"), "stdout={}", out.stdout);

    let out = s.execute("node -c bad.js");
    assert_ne!(out.exit_code, 0, "bad.js must be flagged");

    // `jscheck` alias behaves identically.
    let out = s.execute("jscheck good.js");
    assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
    let out = s.execute("jscheck bad.js");
    assert_ne!(out.exit_code, 0, "jscheck must flag bad.js");

    set_global_device_callback(None);
}
