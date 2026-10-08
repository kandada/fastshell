// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! A [`DevicePlugin`] implementation that forwards every call through a single
//! C callback: `fn(method, args_json) -> malloc'd json string`.
//!
//! This is how the Android host registers device capabilities (camera,
//! microphone, location, sensors, …): the JNI glue installs a trampoline that
//! dispatches to Kotlin `PluginRegistrar.dispatch(method, argsJson)` and the
//! shell's built-in device commands (`camera`, `record`, `location`, …) reach
//! the phone through it.
//!
//! Contract:
//!   * `args_json` is a JSON object with method-specific fields.
//!   * The callback returns a malloc-allocated JSON string; this side frees it
//!     with `libc::free`. `{"ok":false,"error":..}` signals failure; any other
//!     JSON is treated as the (method-specific) success payload.
//!
//! # 经验固化：同步设备桥的反模式与既定规矩
//!
//! 这里的 `DeviceCallbackFn` 是**同步**的 `fn(method, args) -> *mut c_char`，它把
//! 两件本应正交的事绑死了 —— 「设备工作在哪个线程跑」与「同步还是异步返回」。
//! 历史上导致：剪贴板 / 定位等被宿主放到主线程执行 → 直接阻塞 UI（卡死）。
//!
//! 规矩（业界共识，Termux 拆进程+异步 socket、Flutter platform channel 异步消息）：
//!   1. 主线程只做 UI；数据类能力（剪贴板/定位/传感器/IO/网络）一律后台。
//!   2. 桥尽量异步（request/response + callback），别让 shell 命令同步阻塞。
//!
//! 现状：本 `call()` 已把宿主回调放到独立 worker 线程 + 超时（`timeout_secs`），
//!   iOS 侧也已把剪贴板/定位迁出主线程。**彻底根治（异步桥）**：把
//!   `DeviceCallbackFn` 改成 `fn(method, args, request_id)` + `host_send_result(id, json)`
//!   回调，shell 设备命令改为可挂起/恢复 —— 这需要 shell 命令执行模型支持异步，
//!   属较大重构（见 DeviceCallbackHandler.swift 头注释）。

use super::plugin::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};

/// Legacy **synchronous** device callback: `fn(method, args_json) -> *mut c_char`.
pub type DeviceCallbackFn = extern "C" fn(*const c_char, *const c_char) -> *mut c_char;

/// **Asynchronous** device callback (generic across hosts): the host receives
/// `(method, args_json, request_id)` and MUST return promptly. When the device
/// work completes it calls [`host_send_result`] (C ABI
/// `fastshell_device_response`) with the same `request_id`. This keeps the
/// host free to run the work on any thread/queue (or suspend) and is the
/// preferred contract for new integrations (iOS/Android).
pub type DeviceCallbackAsyncFn = extern "C" fn(*const c_char, *const c_char, u64);

/// Process-global device callback. Set once by the host and re-used by every
/// `Fastshell` instance created afterwards — including the private ones
/// aacode-rs spins up per task. This is what makes device commands work
/// through the agent's own fastshell backend.
static DEVICE_CB: OnceLock<Mutex<Option<DeviceCallbackFn>>> = OnceLock::new();
static DEVICE_CB_ASYNC: OnceLock<Mutex<Option<DeviceCallbackAsyncFn>>> = OnceLock::new();

fn device_cb_slot() -> &'static Mutex<Option<DeviceCallbackFn>> {
    DEVICE_CB.get_or_init(|| Mutex::new(None))
}

fn device_cb_async_slot() -> &'static Mutex<Option<DeviceCallbackAsyncFn>> {
    DEVICE_CB_ASYNC.get_or_init(|| Mutex::new(None))
}

/// Pending async requests: `request_id -> oneshot sender` (std mpsc).
fn pending_slot() -> &'static Mutex<HashMap<u64, mpsc::Sender<Option<String>>>> {
    static PENDING: OnceLock<Mutex<HashMap<u64, mpsc::Sender<Option<String>>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Install (or clear) the global **synchronous** device callback.
pub fn set_global_device_callback(cb: Option<DeviceCallbackFn>) {
    if let Ok(mut g) = device_cb_slot().lock() {
        *g = cb;
    }
}

/// Install (or clear) the global **asynchronous** device callback. Takes
/// precedence over the sync callback when both are set.
pub fn set_global_device_callback_async(cb: Option<DeviceCallbackAsyncFn>) {
    if let Ok(mut g) = device_cb_async_slot().lock() {
        *g = cb;
    }
}

/// Host-side responder. Called by the host (Swift/Kotlin/C) when an async
/// device request completes. `json` is a UTF-8 C string that this side only
/// *copies* (the host keeps ownership of its own allocation). Unknown or
/// already-timed-out ids are ignored.
pub fn host_send_result(request_id: u64, json: *const c_char) {
    let value = if json.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(json) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    let tx = pending_slot()
        .lock()
        .ok()
        .and_then(|mut g| g.remove(&request_id));
    if let Some(tx) = tx {
        let _ = tx.send(value);
    }
}

/// Build a fresh plugin bound to the global callback, if one is set.
/// Prefers the async callback; falls back to the legacy sync one.
pub fn global_device_plugin() -> Option<Box<dyn DevicePlugin>> {
    if let Some(cb) = *device_cb_async_slot().lock().ok()? {
        return Some(Box::new(CallbackDevicePlugin::with_async(cb)));
    }
    let cb = (*device_cb_slot().lock().ok()?)?;
    Some(Box::new(CallbackDevicePlugin::new(cb)))
}

/// Which host callback this plugin drives.
enum Backend {
    Sync(DeviceCallbackFn),
    Async(DeviceCallbackAsyncFn),
}

pub struct CallbackDevicePlugin {
    backend: Backend,
}

// The callback is a plain extern "C" fn pointer — safe to call from any thread
// as long as the host side is thread-safe (the JNI trampoline attaches).
unsafe impl Send for CallbackDevicePlugin {}

impl CallbackDevicePlugin {
    /// Legacy sync-callback constructor.
    pub fn new(cb: DeviceCallbackFn) -> Self {
        Self {
            backend: Backend::Sync(cb),
        }
    }

    /// Async-callback constructor (preferred for new hosts).
    pub fn with_async(cb: DeviceCallbackAsyncFn) -> Self {
        Self {
            backend: Backend::Async(cb),
        }
    }

    /// Default upper bound for a single device call. Interactive flows
    /// (photo picker, biometric prompt) have their own host-side timeouts;
    /// this is the defensive net for a hung host bridge so an agent's shell
    /// command can never block forever. Override with FASTSHELL_DEVICE_TIMEOUT.
    fn timeout_secs() -> u64 {
        std::env::var("FASTSHELL_DEVICE_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(25)
    }

    fn call(&self, method: &str, args: Value) -> Result<Value, String> {
        let m = CString::new(method).map_err(|e| e.to_string())?;
        let a = CString::new(args.to_string()).map_err(|e| e.to_string())?;

        let s = match self.backend {
            Backend::Sync(cb) => Self::call_sync(cb, method, m, a)?,
            Backend::Async(cb) => Self::call_async(cb, method, m, a)?,
        };

        let v: Value = serde_json::from_str(&s).map_err(|e| {
            format!(
                "{method}: bad device response ({e}): {}",
                s.chars().take(200).collect::<String>()
            )
        })?;
        if v.get("ok").and_then(|b| b.as_bool()) == Some(false) {
            let err = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("device error")
                .to_string();
            // Do not double-prefix when the host already prefixed the method
            // (e.g. host returns "open_url: could not open" → keep it as-is
            // instead of "open_url: open_url: could not open").
            let prefix = format!("{method}:");
            return Err(if err.starts_with(&prefix) {
                err
            } else {
                format!("{method}: {err}")
            });
        }
        Ok(v)
    }

    /// Legacy path: run the blocking host callback on a worker thread with a
    /// deadline. On timeout the worker is abandoned — its eventual reply lands
    /// in a disconnected channel and is dropped safely.
    fn call_sync(
        cb: DeviceCallbackFn,
        method: &str,
        m: CString,
        a: CString,
    ) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        let spawn = std::thread::Builder::new()
            .name(format!("device-{method}"))
            .spawn(move || {
                let ptr = cb(m.as_ptr(), a.as_ptr());
                let out = if ptr.is_null() {
                    None
                } else {
                    let s = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
                    unsafe { libc::free(ptr as *mut c_void) };
                    Some(s)
                };
                let _ = tx.send(out);
            });
        if spawn.is_err() {
            return Err(format!("{method}: failed to spawn device call thread"));
        }

        match rx.recv_timeout(std::time::Duration::from_secs(Self::timeout_secs())) {
            Ok(Some(s)) => Ok(s),
            Ok(None) => Err(format!("{method}: device host returned null")),
            Err(_) => Err(format!(
                "{method}: device call timed out after {}s (host bridge unresponsive)",
                Self::timeout_secs()
            )),
        }
    }

    /// Async path: register a one-shot request, invoke the host callback
    /// (returns immediately), then wait for `host_send_result` on this thread
    /// with the same deadline. The host is free to run the work on any
    /// thread/queue; on timeout we drop the registry entry so it can't leak.
    fn call_async(
        cb: DeviceCallbackAsyncFn,
        method: &str,
        m: CString,
        a: CString,
    ) -> Result<String, String> {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            let mut g = pending_slot().lock().map_err(|e| e.to_string())?;
            g.insert(request_id, tx);
        }
        // Host callback returns promptly; result arrives via host_send_result.
        cb(m.as_ptr(), a.as_ptr(), request_id);

        match rx.recv_timeout(std::time::Duration::from_secs(Self::timeout_secs())) {
            Ok(Some(s)) => Ok(s),
            Ok(None) => Err(format!("{method}: device host returned null")),
            Err(_) => {
                if let Ok(mut g) = pending_slot().lock() {
                    g.remove(&request_id);
                }
                Err(format!(
                    "{method}: device call timed out after {}s (host bridge unresponsive)",
                    Self::timeout_secs()
                ))
            }
        }
    }

    fn call_unit(&self, method: &str, args: Value) -> Result<(), String> {
        self.call(method, args).map(|_| ())
    }

    fn str_field(v: &Value, keys: &[&str]) -> String {
        for k in keys {
            if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
                return s.to_string();
            }
        }
        String::new()
    }

    fn f64_field(v: &Value, key: &str) -> f64 {
        v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
    }
}

impl DevicePlugin for CallbackDevicePlugin {
    fn take_photo(&self, output_path: &str) -> Result<(), String> {
        self.call_unit("take_photo", json!({"path": output_path}))
    }

    fn take_screenshot(&self, output_path: &str) -> Result<(), String> {
        self.call_unit("take_screenshot", json!({"path": output_path}))
    }

    fn pick_photo(&self, output_dir: &str, max_count: u32) -> Result<Vec<String>, String> {
        let v = self.call("pick_photo", json!({"dir": output_dir, "count": max_count}))?;
        if let Some(arr) = v.get("photos").and_then(|x| x.as_array()) {
            return Ok(arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect());
        }
        let single = Self::str_field(&v, &["path", "photo"]);
        if single.is_empty() {
            Ok(vec![])
        } else {
            Ok(vec![single])
        }
    }

    fn pick_video(&self, output_dir: &str) -> Result<String, String> {
        let v = self.call("pick_video", json!({"dir": output_dir}))?;
        Ok(Self::str_field(&v, &["path", "video"]))
    }

    fn list_media(&self, media_type: &str, limit: u32) -> Result<Vec<MediaInfo>, String> {
        let v = self.call("list_media", json!({"type": media_type, "limit": limit}))?;
        let arr = v
            .as_array()
            .cloned()
            .or_else(|| v.get("media").and_then(|x| x.as_array()).cloned())
            .unwrap_or_default();
        Ok(arr
            .iter()
            .map(|m| MediaInfo {
                name: Self::str_field(m, &["name"]),
                path: Self::str_field(m, &["path"]),
                size: m.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
                mime: Self::str_field(m, &["mime"]),
                width: m.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                height: m.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                created: Self::str_field(m, &["created"]),
            })
            .collect())
    }

    fn record_audio(&self, output_path: &str, duration_secs: u32) -> Result<(), String> {
        self.call_unit(
            "record_audio",
            json!({"path": output_path, "duration": duration_secs}),
        )
    }

    fn play_audio(&self, path: &str) -> Result<(), String> {
        self.call_unit("play_audio", json!({"path": path}))
    }

    fn text_to_speech(&self, text: &str) -> Result<(), String> {
        self.call_unit("text_to_speech", json!({"text": text}))
    }

    fn speech_to_text(&self, input_path: &str) -> Result<String, String> {
        let v = self.call("speech_to_text", json!({"path": input_path}))?;
        Ok(Self::str_field(&v, &["text", "status"]))
    }

    fn list_contacts(&self, query: &str, limit: u32) -> Result<Vec<Contact>, String> {
        let v = self.call("list_contacts", json!({"query": query, "limit": limit}))?;
        let arr = v
            .as_array()
            .cloned()
            .or_else(|| v.get("contacts").and_then(|x| x.as_array()).cloned())
            .unwrap_or_default();
        Ok(arr.iter().map(parse_contact).collect())
    }

    fn get_contact(&self, contact_id: &str) -> Result<Contact, String> {
        let v = self.call("get_contact", json!({"id": contact_id}))?;
        Ok(parse_contact(&v))
    }

    fn get_location(&self) -> Result<Location, String> {
        let v = self.call("get_location", json!({}))?;
        Ok(Location {
            latitude: Self::f64_field(&v, "latitude"),
            longitude: Self::f64_field(&v, "longitude"),
            altitude: Self::f64_field(&v, "altitude"),
            accuracy: Self::f64_field(&v, "accuracy"),
            speed: Self::f64_field(&v, "speed"),
        })
    }

    fn get_clipboard(&self) -> Result<String, String> {
        let v = self.call("get_clipboard", json!({}))?;
        Ok(Self::str_field(&v, &["text"]))
    }

    fn set_clipboard(&self, text: &str) -> Result<(), String> {
        self.call_unit("set_clipboard", json!({"text": text}))
    }

    fn get_orientation(&self) -> Result<Orientation, String> {
        let v = self.call("get_orientation", json!({}))?;
        Ok(Orientation {
            pitch: Self::f64_field(&v, "pitch"),
            roll: Self::f64_field(&v, "roll"),
            yaw: Self::f64_field(&v, "yaw"),
        })
    }

    fn get_motion(&self) -> Result<Motion, String> {
        let v = self.call("get_motion", json!({}))?;
        Ok(Motion {
            ax: Self::f64_field(&v, "ax"),
            ay: Self::f64_field(&v, "ay"),
            az: Self::f64_field(&v, "az"),
            gx: Self::f64_field(&v, "gx"),
            gy: Self::f64_field(&v, "gy"),
            gz: Self::f64_field(&v, "gz"),
        })
    }

    fn get_ambient_light(&self) -> Result<f64, String> {
        let v = self.call("get_ambient_light", json!({}))?;
        Ok(v.get("lux")
            .or_else(|| v.get("value"))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0))
    }

    fn get_proximity(&self) -> Result<bool, String> {
        let v = self.call("get_proximity", json!({}))?;
        Ok(v.get("near").and_then(|x| x.as_bool()).unwrap_or(false))
    }

    fn list_sensors(&self) -> Result<Vec<SensorInfo>, String> {
        let v = self.call("list_sensors", json!({}))?;
        let arr = v
            .as_array()
            .cloned()
            .or_else(|| v.get("sensors").and_then(|x| x.as_array()).cloned())
            .unwrap_or_default();
        Ok(arr
            .iter()
            .map(|s| SensorInfo {
                name: Self::str_field(s, &["name"]),
                sensor_type: Self::str_field(s, &["type", "sensor_type"]),
                available: s.get("available").and_then(|x| x.as_bool()).unwrap_or(true),
            })
            .collect())
    }

    fn send_notification(&self, title: &str, body: &str, sound: bool) -> Result<(), String> {
        self.call_unit(
            "send_notification",
            json!({"title": title, "body": body, "sound": sound}),
        )
    }

    fn share_file(&self, path: &str, mime: &str) -> Result<(), String> {
        self.call_unit("share_file", json!({"path": path, "mime": mime}))
    }

    fn share_text(&self, text: &str) -> Result<(), String> {
        self.call_unit("share_text", json!({"text": text}))
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.call_unit("open_url", json!({"url": url}))
    }

    fn open_settings(&self, target: &str) -> Result<(), String> {
        self.call_unit("open_settings", json!({"target": target}))
    }

    fn authenticate_biometric(&self, reason: &str) -> Result<bool, String> {
        let v = self.call("authenticate_biometric", json!({"reason": reason}))?;
        Ok(v.get("authenticated")
            .or_else(|| v.get("available"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false))
    }

    fn get_battery(&self) -> Result<BatteryInfo, String> {
        let v = self.call("get_battery", json!({}))?;
        Ok(BatteryInfo {
            level: Self::f64_field(&v, "level"),
            charging: v.get("charging").and_then(|x| x.as_bool()).unwrap_or(false),
            source: Self::str_field(&v, &["source"]),
        })
    }

    fn get_network_type(&self) -> Result<NetworkType, String> {
        let v = self.call("get_network_type", json!({}))?;
        Ok(NetworkType {
            kind: Self::str_field(&v, &["kind"]),
            connected: v
                .get("connected")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
        })
    }

    fn set_brightness(&self, level: f64) -> Result<(), String> {
        self.call_unit("set_brightness", json!({"level": level}))
    }

    fn keep_screen_on(&self, on: bool) -> Result<(), String> {
        self.call_unit("keep_screen_on", json!({"on": on}))
    }

    fn vibrate(&self, duration_ms: u32) -> Result<(), String> {
        self.call_unit("vibrate", json!({"duration": duration_ms}))
    }

    fn get_device_info(&self) -> Result<DeviceInfo, String> {
        let v = self.call("get_device_info", json!({}))?;
        Ok(DeviceInfo {
            model: Self::str_field(&v, &["model"]),
            manufacturer: Self::str_field(&v, &["manufacturer"]),
            os_version: Self::str_field(&v, &["os_version"]),
            screen_width: v.get("screen_width").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            screen_height: v.get("screen_height").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        })
    }

    fn eval_js(&self, code: &str) -> Result<String, String> {
        let v = self.call("eval_js", json!({"code": code}))?;
        Ok(Self::str_field(&v, &["result", "value", "output"]))
    }

    fn render_html(&self, html: &str, output_path: &str) -> Result<(), String> {
        self.call_unit("render_html", json!({"html": html, "path": output_path}))
    }
}

fn parse_contact(v: &Value) -> Contact {
    let strings = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };
    Contact {
        id: CallbackDevicePlugin::str_field(v, &["id"]),
        name: CallbackDevicePlugin::str_field(v, &["name"]),
        phones: strings("phones"),
        emails: strings("emails"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" fn fake_cb(method: *const c_char, _args: *const c_char) -> *mut c_char {
        let m = unsafe { CStr::from_ptr(method).to_string_lossy().into_owned() };
        let resp = match m.as_str() {
            "get_battery" => r#"{"level":88.0,"charging":true,"source":"battery"}"#,
            "get_clipboard" => r#"{"ok":true,"text":"hello"}"#,
            "vibrate" => r#"{"ok":true,"vibrated":200}"#,
            "take_photo" => r#"{"ok":false,"error":"no camera"}"#,
            _ => r#"{"ok":true}"#,
        };
        let c = CString::new(resp).unwrap();
        // Transfer ownership as a malloc'd string (strdup semantics).
        unsafe { libc::strdup(c.as_ptr()) }
    }

    #[test]
    fn battery_roundtrip() {
        let p = CallbackDevicePlugin::new(fake_cb);
        let b = p.get_battery().unwrap();
        assert_eq!(b.level, 88.0);
        assert!(b.charging);
    }

    #[test]
    fn clipboard_roundtrip() {
        let p = CallbackDevicePlugin::new(fake_cb);
        assert_eq!(p.get_clipboard().unwrap(), "hello");
    }

    #[test]
    fn unit_ok() {
        let p = CallbackDevicePlugin::new(fake_cb);
        assert!(p.vibrate(200).is_ok());
    }

    #[test]
    fn error_propagates() {
        let p = CallbackDevicePlugin::new(fake_cb);
        let e = p.take_photo("/x.jpg").unwrap_err();
        assert!(e.contains("no camera"));
    }

    // ── async callback path ────────────────────────────────────────────

    /// Async host: returns immediately; responds later from another thread
    /// via `host_send_result` (mirrors what iOS/Android hosts do).
    extern "C" fn fake_async_cb(method: *const c_char, _args: *const c_char, request_id: u64) {
        let m = unsafe { CStr::from_ptr(method).to_string_lossy().into_owned() };
        std::thread::spawn(move || {
            let resp = match m.as_str() {
                "get_battery" => r#"{"level":77.0,"charging":false,"source":"battery"}"#,
                "get_clipboard" => r#"{"ok":true,"text":"async-hello"}"#,
                "take_photo" => r#"{"ok":false,"error":"async no camera"}"#,
                _ => r#"{"ok":true}"#,
            };
            let c = CString::new(resp).unwrap();
            host_send_result(request_id, c.as_ptr());
        });
    }

    #[test]
    fn async_battery_roundtrip() {
        let p = CallbackDevicePlugin::with_async(fake_async_cb);
        let b = p.get_battery().unwrap();
        assert_eq!(b.level, 77.0);
        assert!(!b.charging);
    }

    #[test]
    fn async_clipboard_and_error() {
        let p = CallbackDevicePlugin::with_async(fake_async_cb);
        assert_eq!(p.get_clipboard().unwrap(), "async-hello");
        let e = p.take_photo("/x.jpg").unwrap_err();
        assert!(e.contains("async no camera"));
    }

    #[test]
    fn async_response_unknown_id_is_noop() {
        // Must not panic when the host responds for an id nobody waits on.
        let c = CString::new(r#"{"ok":true}"#).unwrap();
        host_send_result(999_999, c.as_ptr());
    }
}
