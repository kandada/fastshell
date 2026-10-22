// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub phones: Vec<String>,
    pub emails: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
    pub accuracy: f64,
    pub speed: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Orientation {
    pub pitch: f64,
    pub roll: f64,
    pub yaw: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Motion {
    pub ax: f64,
    pub ay: f64,
    pub az: f64,
    pub gx: f64,
    pub gy: f64,
    pub gz: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorInfo {
    pub name: String,
    pub sensor_type: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub level: f64,
    pub charging: bool,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkType {
    pub kind: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub model: String,
    pub manufacturer: String,
    pub os_version: String,
    pub screen_width: u32,
    pub screen_height: u32,
}

// ── Mobile Use (UI automation) contract ─────────────────────────────────────
//
// The host implements these with Accessibility / Shizuku / Root and registers
// the plugin via the existing device callback bridge. fastshell only defines
// the contract; it never holds Android permissions.

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiLimits {
    pub max_depth: u32,
    pub max_nodes: u32,
    pub max_text_len: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiNode {
    /// Host-issued, opaque reference for this snapshot only.
    pub node_ref: String,
    pub role: String,
    pub text: String,
    pub description: String,
    pub view_id: String,
    pub bounds: UiBounds,
    pub clickable: bool,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub selected: bool,
    /// Password/OTP/etc. — hosts must redact text/description for these.
    pub sensitive: bool,
    pub actions: Vec<String>,
}

/// Stable node identity used to detect a stale target before acting.
///
/// `tree_revision` is informational only: unrelated tree changes must not
/// reject an otherwise-identical node (see `mobile::fingerprint_matches`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiFingerprint {
    pub package: String,
    pub window_id: i64,
    pub display_id: i32,
    pub width: u32,
    pub height: u32,
    pub orientation: i32,
    pub tree_revision: u64,
    pub view_id: String,
    pub text: String,
    pub description: String,
    pub bounds: UiBounds,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiWindow {
    pub package: String,
    pub window_id: i64,
    pub display_id: i32,
    pub width: u32,
    pub height: u32,
    pub orientation: i32,
    pub focused: bool,
    pub tree_revision: u64,
    pub backend: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSnapshot {
    pub backend: String,
    pub package: String,
    pub window_id: i64,
    pub display_id: i32,
    pub width: u32,
    pub height: u32,
    pub orientation: i32,
    pub tree_revision: u64,
    pub limits: UiLimits,
    pub nodes: Vec<UiNode>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiTarget {
    pub package: Option<String>,
    pub window_id: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiAction {
    pub target: UiTarget,
    pub node_ref: String,
    pub fingerprint: UiFingerprint,
    /// click | long_click | set_text | ime_enter | scroll
    pub action: String,
    pub text: Option<String>,
    pub value: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiGesture {
    pub target: UiTarget,
    pub frame: UiFingerprint,
    pub points: Vec<(f64, f64)>,
    pub duration_ms: u64,
    pub fingers: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiProgress {
    pub target: UiTarget,
    pub node_ref: String,
    pub fingerprint: UiFingerprint,
    pub value: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiActionResult {
    pub ok: bool,
    pub backend: String,
    pub status: String,
    pub error_code: Option<String>,
    pub message: Option<String>,
}

/// Trait for device-specific capabilities.
/// Host app implements this and registers with fastshell.
/// fastshell never holds system permissions — the host owns all authorization.
pub trait DevicePlugin: Send {
    // ── camera ──
    fn take_photo(&self, output_path: &str) -> Result<(), String>;
    fn take_screenshot(&self, output_path: &str) -> Result<(), String>;

    // ── photolib ──
    fn pick_photo(&self, output_dir: &str, max_count: u32) -> Result<Vec<String>, String>;
    fn pick_video(&self, output_dir: &str) -> Result<String, String>;
    fn list_media(&self, media_type: &str, limit: u32) -> Result<Vec<MediaInfo>, String>;

    // ── record / audio ──
    fn record_audio(&self, output_path: &str, duration_secs: u32) -> Result<(), String>;
    fn play_audio(&self, path: &str) -> Result<(), String>;
    fn text_to_speech(&self, text: &str) -> Result<(), String>;
    fn speech_to_text(&self, input_path: &str) -> Result<String, String>;

    // ── contacts ──
    fn list_contacts(&self, query: &str, limit: u32) -> Result<Vec<Contact>, String>;
    fn get_contact(&self, contact_id: &str) -> Result<Contact, String>;

    // ── location ──
    fn get_location(&self) -> Result<Location, String>;

    // ── clipboard ──
    fn get_clipboard(&self) -> Result<String, String>;
    fn set_clipboard(&self, text: &str) -> Result<(), String>;

    // ── sensors ──
    fn get_orientation(&self) -> Result<Orientation, String>;
    fn get_motion(&self) -> Result<Motion, String>;
    fn get_ambient_light(&self) -> Result<f64, String>;
    fn get_proximity(&self) -> Result<bool, String>;
    fn list_sensors(&self) -> Result<Vec<SensorInfo>, String>;

    // ── notifications ──
    fn send_notification(&self, title: &str, body: &str, sound: bool) -> Result<(), String>;

    // ── share / open ──
    fn share_file(&self, path: &str, mime: &str) -> Result<(), String>;
    fn share_text(&self, text: &str) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// Open a settings page. `target` is `"app"` | `"system"` |
    /// `"accessibility"` | `"notification"`. Default implementation reports
    /// "not supported" so existing hosts (which don't implement it yet) are
    /// unaffected and the shell command fails cleanly.
    fn open_settings(&self, target: &str) -> Result<(), String> {
        let _ = target;
        Err("open_settings not supported".to_string())
    }

    // ── biometric ──
    fn authenticate_biometric(&self, reason: &str) -> Result<bool, String>;

    // ── device state ──
    fn get_battery(&self) -> Result<BatteryInfo, String>;
    fn get_network_type(&self) -> Result<NetworkType, String>;
    fn set_brightness(&self, level: f64) -> Result<(), String>;
    fn keep_screen_on(&self, on: bool) -> Result<(), String>;
    fn vibrate(&self, duration_ms: u32) -> Result<(), String>;
    fn get_device_info(&self) -> Result<DeviceInfo, String>;

    // ── javascript / webview ──
    /// Evaluate JavaScript in the host app's WebView engine (Android WebView
    /// = V8, iOS WKWebView = JavaScriptCore) and return the result string.
    /// Defaults to unsupported for hosts that don't register a JS backend.
    fn eval_js(&self, code: &str) -> Result<String, String> {
        let _ = code;
        Err("eval_js: no JavaScript backend registered by host app".to_string())
    }

    /// Render an HTML document in the host WebView and capture a screenshot
    /// to `output_path` (host-absolute). Enables visual feedback for
    /// frontend code. Defaults to unsupported when the host lacks a WebView.
    fn render_html(&self, html: &str, output_path: &str) -> Result<(), String> {
        let _ = (html, output_path);
        Err("render_html: no WebView backend registered by host app".to_string())
    }

    // ── Mobile Use (UI automation) ──────────────────────────────────────
    // All default to "not supported" so existing hosts keep compiling and
    // fail cleanly. The host owns Accessibility/Shizuku/Root authorization.

    /// Ready backends in priority order, e.g. `["root","shizuku","accessibility"]`.
    fn ui_backends(&self) -> Result<Vec<String>, String> {
        Err("ui_backends: not supported by host".to_string())
    }
    fn ui_list_windows(&self) -> Result<Vec<UiWindow>, String> {
        Err("ui_list_windows: not supported by host".to_string())
    }
    fn ui_snapshot(&self, _target: &UiTarget, _limits: &UiLimits) -> Result<UiSnapshot, String> {
        Err("ui_snapshot: not supported by host".to_string())
    }
    fn ui_find(
        &self,
        _target: &UiTarget,
        _selector: &str,
        _limit: u32,
    ) -> Result<Vec<UiNode>, String> {
        Err("ui_find: not supported by host".to_string())
    }
    fn ui_act(&self, _action: &UiAction) -> Result<UiActionResult, String> {
        Err("ui_act: not supported by host".to_string())
    }
    fn ui_gesture(&self, _gesture: &UiGesture) -> Result<(), String> {
        Err("ui_gesture: not supported by host".to_string())
    }
    /// Base64-encoded PNG/JPEG bytes (host captures the screen/window).
    fn ui_screenshot(&self, _target: &UiTarget) -> Result<String, String> {
        Err("ui_screenshot: not supported by host".to_string())
    }
    fn ui_wait(
        &self,
        _target: &UiTarget,
        _selector: &str,
        _present: bool,
        _timeout_ms: u64,
    ) -> Result<bool, String> {
        Err("ui_wait: not supported by host".to_string())
    }
    fn ui_set_progress(&self, _progress: &UiProgress) -> Result<(), String> {
        Err("ui_set_progress: not supported by host".to_string())
    }
}
