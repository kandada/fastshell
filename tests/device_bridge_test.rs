// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Device-capability bridge robustness tests (agent path):
//!   * path resolution — device files land in the CALLER's sandbox,
//!   * process-wide permission sharing — UI grants visible to agent instances,
//!   * Rust-side timeout — a hung host bridge can never block a task forever.

use fastshell::sdk::device_callback::{set_global_device_callback, DeviceCallbackFn};
use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::ffi::{c_char, CStr, CString};
use std::sync::Mutex;

static LAST_CALL: Mutex<Option<(String, String)>> = Mutex::new(None);
static HANG: Mutex<bool> = Mutex::new(false);

extern "C" fn fake_host(method: *const c_char, args: *const c_char) -> *mut c_char {
    let m = unsafe { CStr::from_ptr(method).to_string_lossy().into_owned() };
    let a = unsafe { CStr::from_ptr(args).to_string_lossy().into_owned() };
    *LAST_CALL.lock().unwrap() = Some((m.clone(), a));
    if *HANG.lock().unwrap() {
        loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
    }
    let resp = match m.as_str() {
        "get_battery" => r#"{"level":75.0,"charging":false,"source":"battery"}"#,
        _ => r#"{"ok":true}"#,
    };
    let c = CString::new(resp).unwrap();
    unsafe { libc::strdup(c.as_ptr()) }
}

fn mk(tag: &str) -> Fastshell {
    let mut s = Fastshell::new();
    let dir = std::env::temp_dir().join(format!("fs_dev_{}_{}", tag, std::process::id()));
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
fn agent_instance_inherits_callback_and_resolves_paths_to_own_sandbox() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_global_device_callback(Some(fake_host as DeviceCallbackFn));
    *HANG.lock().unwrap() = false;

    // A UI-side instance grants after the system dialog — process-wide.
    let ui = mk("ui");
    ui.set_permission("camera:photo", true);
    ui.set_permission("microphone:record", true);

    // Agent-spawned instance: created AFTER callback registration, own sandbox.
    let s = mk("agent");
    s.execute("mkdir -p media");
    let sandbox = s.vfs_root();

    // camera with a VFS path → host must receive a HOST-ABSOLUTE path inside
    // THIS instance's sandbox (not the app-global one).
    let out = s.execute("camera /media/photo.jpg");
    assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
    let args = last_args();
    assert!(args.contains(&sandbox), "must resolve into caller sandbox: {args}");
    assert!(args.contains("media/photo.jpg"), "{args}");
    assert!(!args.contains("\"/media/photo.jpg\""), "raw VFS path leaked: {args}");

    // cwd semantics: relative path resolves under the current directory.
    s.execute("cd media");
    let out = s.execute("record -d 3 -o take1.m4a");
    assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
    let args = last_args();
    assert!(args.contains("media/take1.m4a"), "cwd-relative: {args}");
    assert!(args.contains(&sandbox), "{args}");

    // Path escape must be rejected before reaching the host.
    let out = s.execute("camera ../../../../etc/evil.jpg");
    assert_ne!(out.exit_code, 0, "escape must fail: {}", out.stdout);
    set_global_device_callback(None);
}

#[test]
fn ui_grant_is_visible_to_agent_instances() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_global_device_callback(Some(fake_host as DeviceCallbackFn));
    *HANG.lock().unwrap() = false;

    // No grant yet → PERMISSION_NEEDED (app UI shows the dialog).
    let fresh = mk("perm1");
    let out = fresh.execute("location");
    assert!(
        out.stderr.contains("PERMISSION_NEEDED"),
        "ungranted must ask: {} {}", out.stdout, out.stderr
    );

    // UI grants once (any instance) → a brand-new agent instance proceeds.
    fresh.set_permission("location:gps", true);
    let agent = mk("perm2");
    let out = agent.execute("location");
    assert_eq!(out.exit_code, 0, "granted globally: {}", out.stderr);

    // Explicit per-instance deny wins over the global grant.
    agent.set_permission("contacts:read", false);
    let out = agent.execute("contacts search x");
    assert!(out.stderr.contains("Permission denied"), "{}", out.stderr);
    set_global_device_callback(None);
}

#[test]
fn hung_host_bridge_times_out_instead_of_blocking_forever() {
    let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("FASTSHELL_DEVICE_TIMEOUT", "2");
    set_global_device_callback(Some(fake_host as DeviceCallbackFn));
    *HANG.lock().unwrap() = true;

    let s = mk("hang");
    let start = std::time::Instant::now();
    let out = s.execute("battery");
    let elapsed = start.elapsed();
    *HANG.lock().unwrap() = false;
    std::env::remove_var("FASTSHELL_DEVICE_TIMEOUT");
    set_global_device_callback(None);

    assert_ne!(out.exit_code, 0, "hung call must fail, not block");
    assert!(
        out.stderr.contains("timed out"),
        "must report a timeout: {}",
        out.stderr
    );
    assert!(
        elapsed.as_secs() < 20,
        "timeout must bound the hang; took {elapsed:?}"
    );
}
