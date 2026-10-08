// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Coverage for `sdk/device_callback.rs`: the `DevicePlugin` that forwards to a
//! single host C callback, plus the process-global registration path.

use fastshell::sdk::device_callback::{
    global_device_plugin, set_global_device_callback, CallbackDevicePlugin,
};
use fastshell::sdk::plugin::DevicePlugin;
use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// Returns a malloc'd JSON string; frees are done by the consumer.
unsafe fn ret(payload: &str) -> *mut c_char {
    let c = CString::new(payload).unwrap();
    let bytes = c.as_bytes_with_nul();
    let p = libc::malloc(bytes.len()) as *mut u8;
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    p as *mut c_char
}

extern "C" fn cb_ok(method: *const c_char, _args: *const c_char) -> *mut c_char {
    let m = unsafe { CStr::from_ptr(method) }.to_str().unwrap_or("");
    let payload = match m {
        "pick_photo" => r#"{"ok":true,"photos":["/a.jpg","/b.jpg"]}"#,
        "pick_video" => r#"{"ok":true,"path":"/v.mp4"}"#,
        "list_media" => {
            r#"{"ok":true,"media":[{"name":"a","path":"/a.jpg","size":1,"mime":"image/jpeg","width":1,"height":1,"created":"now"}]}"#
        }
        "list_contacts" => {
            r#"{"ok":true,"contacts":[{"id":"c1","name":"Bob","phones":[],"emails":[]}]}"#
        }
        "get_contact" => r#"{"ok":true,"id":"c1","name":"Bob","phones":[],"emails":[]}"#,
        "get_location" => {
            r#"{"ok":true,"latitude":1.0,"longitude":2.0,"altitude":3.0,"accuracy":1.0,"speed":0.0}"#
        }
        "get_clipboard" => r#"{"ok":true,"text":"clip"}"#,
        "get_orientation" => r#"{"ok":true,"pitch":0.0,"roll":0.0,"yaw":0.0}"#,
        "get_motion" => r#"{"ok":true,"ax":0.0,"ay":0.0,"az":0.0,"gx":0.0,"gy":0.0,"gz":0.0}"#,
        "get_ambient_light" => r#"{"ok":true,"lux":42.0}"#,
        "get_proximity" => r#"{"ok":true,"near":true}"#,
        "list_sensors" => {
            r#"{"ok":true,"sensors":[{"name":"accel","type":"motion","available":true}]}"#
        }
        "authenticate_biometric" => r#"{"ok":true,"authenticated":true}"#,
        "get_battery" => r#"{"ok":true,"level":0.9,"charging":true,"source":"usb"}"#,
        "get_network_type" => r#"{"ok":true,"kind":"wifi","connected":true}"#,
        "get_device_info" => {
            r#"{"ok":true,"model":"X","manufacturer":"Y","os_version":"1","screen_width":1,"screen_height":2}"#
        }
        "eval_js" => r#"{"ok":true,"result":"42"}"#,
        _ => r#"{"ok":true}"#,
    };
    unsafe { ret(payload) }
}

extern "C" fn cb_null(_m: *const c_char, _a: *const c_char) -> *mut c_char {
    std::ptr::null_mut()
}

extern "C" fn cb_err(_m: *const c_char, _a: *const c_char) -> *mut c_char {
    unsafe { ret(r#"{"ok":false,"error":"boom"}"#) }
}

/// Host returns an error string that ALREADY includes the method prefix.
extern "C" fn cb_prefixed_err(_m: *const c_char, _a: *const c_char) -> *mut c_char {
    unsafe { ret(r#"{"ok":false,"error":"open_url: could not open"}"#) }
}

#[test]
fn device_error_is_not_double_prefixed() {
    let p = CallbackDevicePlugin::new(cb_prefixed_err);
    let e = p.open_url("app-settings:").unwrap_err();
    assert!(e.contains("could not open"), "{e}");
    assert!(
        !e.contains("open_url: open_url:"),
        "method prefix must not be duplicated: {e}"
    );
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn callback_plugin_direct() {
    let p = CallbackDevicePlugin::new(cb_ok);
    // Exercise every capability (success or error — we only require no panic).
    let _ = p.take_photo("/a");
    let _ = p.take_screenshot("/a");
    let _ = p.pick_photo("/d", 2);
    let _ = p.pick_video("/d");
    let _ = p.list_media("image", 1);
    let _ = p.record_audio("/a", 1);
    let _ = p.play_audio("/a");
    let _ = p.text_to_speech("hi");
    let _ = p.speech_to_text("/a");
    let _ = p.list_contacts("", 5);
    let _ = p.get_contact("c1");
    let _ = p.get_location();
    let _ = p.get_clipboard();
    let _ = p.set_clipboard("x");
    let _ = p.get_orientation();
    let _ = p.get_motion();
    let _ = p.get_ambient_light();
    let _ = p.get_proximity();
    let _ = p.list_sensors();
    let _ = p.send_notification("t", "b", true);
    let _ = p.share_file("/f", "text/plain");
    let _ = p.share_text("x");
    let _ = p.open_url("https://e.com");
    let _ = p.authenticate_biometric("reason");
    let _ = p.get_battery();
    let _ = p.get_network_type();
    let _ = p.set_brightness(0.5);
    let _ = p.keep_screen_on(true);
    let _ = p.vibrate(10);
    let _ = p.get_device_info();
    let _ = p.eval_js("1+1");
    let _ = p.render_html("<h1>x</h1>", "/a.png");
    assert!(p.get_battery().is_ok());
    assert!(p.get_location().is_ok());
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn callback_plugin_error_and_null_paths() {
    let e = CallbackDevicePlugin::new(cb_err);
    assert!(e.get_battery().is_err());
    let n = CallbackDevicePlugin::new(cb_null);
    assert!(n.get_battery().is_err());
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn global_callback_is_picked_up_by_new_instances() {
    set_global_device_callback(Some(cb_ok));
    assert!(global_device_plugin().is_some());

    let dir = std::env::temp_dir().join(format!("fs_devcb_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        command_timeout_ms: 5_000,
        ..Default::default()
    })
    .unwrap();
    let r = s.execute("battery");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("level"), "{}", r.stdout);

    set_global_device_callback(None);
    assert!(global_device_plugin().is_none());
}
