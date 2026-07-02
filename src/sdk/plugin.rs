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

    // ── biometric ──
    fn authenticate_biometric(&self, reason: &str) -> Result<bool, String>;

    // ── device state ──
    fn get_battery(&self) -> Result<BatteryInfo, String>;
    fn get_network_type(&self) -> Result<NetworkType, String>;
    fn set_brightness(&self, level: f64) -> Result<(), String>;
    fn keep_screen_on(&self, on: bool) -> Result<(), String>;
    fn vibrate(&self, duration_ms: u32) -> Result<(), String>;
    fn get_device_info(&self) -> Result<DeviceInfo, String>;
}
