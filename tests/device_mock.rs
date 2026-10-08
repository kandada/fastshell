// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Coverage for the device-command modules (`devices.rs`, `device_callback.rs`)
//! using a **mock `DevicePlugin`** registered via the public SDK. No real
//! hardware is needed: every capability returns deterministic dummy data.

use fastshell::sdk::plugin::*;
use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static OPEN_URL_CALLS: AtomicUsize = AtomicUsize::new(0);
static OPEN_SETTINGS_TARGETS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct MockPlugin;

impl DevicePlugin for MockPlugin {
    fn take_photo(&self, _p: &str) -> Result<(), String> {
        Ok(())
    }
    fn take_screenshot(&self, _p: &str) -> Result<(), String> {
        Ok(())
    }
    fn pick_photo(&self, _d: &str, _n: u32) -> Result<Vec<String>, String> {
        Ok(vec!["/a.jpg".into(), "/b.jpg".into()])
    }
    fn pick_video(&self, _d: &str) -> Result<String, String> {
        Ok("/v.mp4".into())
    }
    fn list_media(&self, _t: &str, _l: u32) -> Result<Vec<MediaInfo>, String> {
        Ok(vec![MediaInfo {
            name: "a".into(),
            path: "/a.jpg".into(),
            size: 1,
            mime: "image/jpeg".into(),
            width: 1,
            height: 1,
            created: "now".into(),
        }])
    }
    fn record_audio(&self, _p: &str, _d: u32) -> Result<(), String> {
        Ok(())
    }
    fn play_audio(&self, _p: &str) -> Result<(), String> {
        Ok(())
    }
    fn text_to_speech(&self, _t: &str) -> Result<(), String> {
        Ok(())
    }
    fn speech_to_text(&self, _p: &str) -> Result<String, String> {
        Ok("transcribed".into())
    }
    fn list_contacts(&self, _q: &str, _l: u32) -> Result<Vec<Contact>, String> {
        Ok(vec![Contact {
            id: "c1".into(),
            name: "Bob".into(),
            phones: vec!["123".into()],
            emails: vec![],
        }])
    }
    fn get_contact(&self, id: &str) -> Result<Contact, String> {
        Ok(Contact {
            id: id.into(),
            name: "Bob".into(),
            phones: vec![],
            emails: vec![],
        })
    }
    fn get_location(&self) -> Result<Location, String> {
        Ok(Location {
            latitude: 1.0,
            longitude: 2.0,
            altitude: 3.0,
            accuracy: 1.0,
            speed: 0.0,
        })
    }
    fn get_clipboard(&self) -> Result<String, String> {
        Ok("clip".into())
    }
    fn set_clipboard(&self, _t: &str) -> Result<(), String> {
        Ok(())
    }
    fn get_orientation(&self) -> Result<Orientation, String> {
        Ok(Orientation {
            pitch: 0.0,
            roll: 0.0,
            yaw: 0.0,
        })
    }
    fn get_motion(&self) -> Result<Motion, String> {
        Ok(Motion {
            ax: 0.0,
            ay: 0.0,
            az: 0.0,
            gx: 0.0,
            gy: 0.0,
            gz: 0.0,
        })
    }
    fn get_ambient_light(&self) -> Result<f64, String> {
        Ok(42.0)
    }
    fn get_proximity(&self) -> Result<bool, String> {
        Ok(true)
    }
    fn list_sensors(&self) -> Result<Vec<SensorInfo>, String> {
        Ok(vec![SensorInfo {
            name: "accel".into(),
            sensor_type: "motion".into(),
            available: true,
        }])
    }
    fn send_notification(&self, _t: &str, _b: &str, _s: bool) -> Result<(), String> {
        Ok(())
    }
    fn share_file(&self, _p: &str, _m: &str) -> Result<(), String> {
        Ok(())
    }
    fn share_text(&self, _t: &str) -> Result<(), String> {
        Ok(())
    }
    fn open_url(&self, _u: &str) -> Result<(), String> {
        OPEN_URL_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn open_settings(&self, target: &str) -> Result<(), String> {
        OPEN_SETTINGS_TARGETS
            .lock()
            .unwrap()
            .push(target.to_string());
        Ok(())
    }
    fn authenticate_biometric(&self, _r: &str) -> Result<bool, String> {
        Ok(true)
    }
    fn get_battery(&self) -> Result<BatteryInfo, String> {
        Ok(BatteryInfo {
            level: 0.9,
            charging: true,
            source: "usb".into(),
        })
    }
    fn get_network_type(&self) -> Result<NetworkType, String> {
        Ok(NetworkType {
            kind: "wifi".into(),
            connected: true,
        })
    }
    fn set_brightness(&self, _l: f64) -> Result<(), String> {
        Ok(())
    }
    fn keep_screen_on(&self, _on: bool) -> Result<(), String> {
        Ok(())
    }
    fn vibrate(&self, _ms: u32) -> Result<(), String> {
        Ok(())
    }
    fn get_device_info(&self) -> Result<DeviceInfo, String> {
        Ok(DeviceInfo {
            model: "X".into(),
            manufacturer: "Y".into(),
            os_version: "1".into(),
            screen_width: 100,
            screen_height: 200,
        })
    }
    fn eval_js(&self, _c: &str) -> Result<String, String> {
        Ok("42".into())
    }
    fn render_html(&self, _h: &str, _p: &str) -> Result<(), String> {
        Ok(())
    }
}

static DIR_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn sdk() -> Fastshell {
    let seq = DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_devmock_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 800,
        ..Default::default()
    })
    .unwrap();
    s.register_plugin(Box::new(MockPlugin));
    for p in [
        "camera:photo",
        "screen:capture",
        "photolib:read",
        "microphone:record",
        "microphone:speech",
        "contacts:read",
        "location:gps",
    ] {
        s.set_permission(p, true);
    }
    s
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn device_commands_with_mock_plugin() {
    let s = sdk();
    let cases: &[(&str, &str)] = &[
        ("camera /photo.jpg", "Photo saved"),
        ("screencapture /shot.png", "Screenshot saved"),
        ("photolib -n 2 /photos", "photos"),
        ("photolib --video /videos", "video"),
        ("record -d 3 /rec.wav", "Recorded"),
        ("play /rec.wav", ""),
        ("say hello world", ""),
        ("speech /rec.wav", "transcribed"),
        ("contacts", "Bob"),
        ("contacts get c1", "Bob"),
        ("contacts search bob", "Bob"),
        ("location", "latitude"),
        ("clipboard", "clip"),
        ("clipboard set hi", ""),
        ("pbcopy hi", ""),
        ("sensor list", "accel"),
        ("sensor orientation", "pitch"),
        ("sensor motion", "ax"),
        ("sensor light", "lux"),
        ("sensor proximity", "near"),
        ("notify title body", ""),
        ("notify-send one", ""),
        ("share /f.txt --mime text/plain", ""),
        ("share --text hi", ""),
        ("open https://example.com", ""),
        ("auth bio unlock", "authenticated"),
        ("auth please", "authenticated"),
        ("battery", "level"),
        ("vibrate 100", ""),
        ("screen brightness 0.5", ""),
        ("screen on", ""),
        ("screen off", ""),
        ("device info", "model"),
        ("device network", "wifi"),
        ("arecord -d 1 /a.wav", "Recorded"),
    ];
    for (cmd, want) in cases {
        let r = s.execute(cmd);
        assert_eq!(r.exit_code, 0, "cmd={cmd} stderr={}", r.stderr);
        if !want.is_empty() {
            assert!(
                r.stdout.contains(want),
                "cmd={cmd} want~{want:?} got={:?}",
                r.stdout
            );
        }
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn device_commands_without_plugin_are_unsupported() {
    // No plugin registered → device commands must fail cleanly (no panic).
    let dir = std::env::temp_dir().join(format!(
        "fs_devnop_{}_{}",
        std::process::id(),
        DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        command_timeout_ms: 800,
        ..Default::default()
    })
    .unwrap();
    for cmd in ["battery", "device info", "sensor list", "location"] {
        let r = s.execute(cmd);
        assert!(r.exit_code != 0, "cmd={cmd} should fail without a plugin");
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn device_permission_denied_path() {
    let dir = std::env::temp_dir().join(format!(
        "fs_devdeny_{}_{}",
        std::process::id(),
        DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        command_timeout_ms: 800,
        ..Default::default()
    })
    .unwrap();
    s.register_plugin(Box::new(MockPlugin));
    s.set_permission("camera:photo", false);
    let r = s.execute("camera /p.jpg");
    assert_ne!(r.exit_code, 0);
    assert!(
        r.stderr.to_lowercase().contains("permission"),
        "{}",
        r.stderr
    );
}

// ── Feedback + idempotency (the "opened but got no result" bug class) ──

#[test]
fn open_reports_confirmation_and_is_idempotent() {
    OPEN_URL_CALLS.store(0, Ordering::SeqCst);
    let s = sdk();
    let url = "https://feedback-open-test.example/1";
    let r = s.execute(&format!("open {url}"));
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("opened:"),
        "success must be explicit, not empty: stdout={:?}",
        r.stdout
    );
    assert_eq!(OPEN_URL_CALLS.load(Ordering::SeqCst), 1);

    // Re-opening the same URL immediately is debounced → no second host call,
    // and the agent gets a clear "already opening" instead of a silent repeat.
    let r2 = s.execute(&format!("open {url}"));
    assert_eq!(r2.exit_code, 0);
    assert!(r2.stdout.contains("already opening"), "{}", r2.stdout);
    assert_eq!(
        OPEN_URL_CALLS.load(Ordering::SeqCst),
        1,
        "identical open within window must not hit the host twice"
    );
}

#[test]
fn open_help_mentions_open_settings() {
    let s = sdk();
    let r = s.execute("open --help");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("open_settings"), "{}", r.stdout);
}

#[test]
fn open_settings_routes_targets_and_reports() {
    OPEN_SETTINGS_TARGETS.lock().unwrap().clear();
    let s = sdk();
    let r = s.execute("open_settings app");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("settings opened: app"), "{}", r.stdout);
    let r = s.execute("open_settings accessibility");
    assert_eq!(r.exit_code, 0);
    assert!(
        r.stdout.contains("settings opened: accessibility"),
        "{}",
        r.stdout
    );
    let targets = OPEN_SETTINGS_TARGETS.lock().unwrap().clone();
    assert_eq!(
        targets,
        vec!["app".to_string(), "accessibility".to_string()]
    );
}

#[test]
fn open_settings_rejects_unknown_target() {
    let s = sdk();
    let r = s.execute("open_settings frobnicate");
    assert_ne!(r.exit_code, 0);
    assert!(
        r.stderr.contains("app|system|accessibility|notification"),
        "{}",
        r.stderr
    );
}

#[test]
fn which_lists_open_settings() {
    let s = sdk();
    let r = s.execute("which open_settings");
    assert_eq!(r.exit_code, 0);
    assert!(
        r.stdout.contains("built-in fastshell command"),
        "{}",
        r.stdout
    );
}

#[test]
fn open_settings_without_plugin_fails_cleanly() {
    // No host plugin → the default trait impl must produce a clean error (no panic).
    let seq = DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_devfb_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        command_timeout_ms: 800,
        ..Default::default()
    })
    .unwrap();
    let r = s.execute("open_settings app");
    assert_ne!(r.exit_code, 0);
}
