// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::sdk::plugin::DevicePlugin;
use crate::shell::{CommandOutput, Shell};

fn plugin<T>(
    shell: &Shell,
    f: impl FnOnce(&Box<dyn DevicePlugin>) -> Result<T, String>,
) -> Result<T, CommandOutput> {
    let guard = match shell.plugin.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    match guard.as_ref() {
        Some(p) => f(p).map_err(|e| CommandOutput::error(e, 1)),
        None => Err(CommandOutput::not_supported("device plugin")),
    }
}

impl Shell {
    // ── camera ──
    pub fn cmd_camera(&self, args: &[&str]) -> CommandOutput {
        let path = args
            .first()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "/photo.jpg".to_string());
        if let Some(perm) = self.check_device_permission("camera", "photo") {
            return perm;
        }
        match plugin(self, |p| p.take_photo(&path)) {
            Ok(()) => CommandOutput::success(format!("Photo saved to {}\n", path)),
            Err(e) => e,
        }
    }

    // ── screencapture ──
    pub fn cmd_screencapture(&self, args: &[&str]) -> CommandOutput {
        let path = args
            .first()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "/screenshot.png".to_string());
        if let Some(perm) = self.check_device_permission("screen", "capture") {
            return perm;
        }
        match plugin(self, |p| p.take_screenshot(&path)) {
            Ok(()) => CommandOutput::success(format!("Screenshot saved to {}\n", path)),
            Err(e) => e,
        }
    }

    // ── photolib ──
    pub fn cmd_photolib(&self, args: &[&str]) -> CommandOutput {
        let mut media_type = "image";
        let mut count = 1u32;
        let mut output_dir = "/".to_string();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--video" => media_type = "video",
                "-n" | "--count" => {
                    if i + 1 < args.len() {
                        count = args[i + 1].parse().unwrap_or(1);
                        i += 1;
                    }
                }
                "-o" | "--output" => {
                    if i + 1 < args.len() {
                        output_dir = args[i + 1].to_string();
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => {
                    output_dir = arg.to_string();
                }
                _ => {}
            }
            i += 1;
        }
        if let Some(perm) = self.check_device_permission("photolib", "read") {
            return perm;
        }
        match plugin(self, |p| {
            if media_type == "video" {
                p.pick_video(&output_dir).map(|p| {
                    serde_json::to_string_pretty(&serde_json::json!({"video": p})).unwrap_or(p)
                })
            } else {
                let files = p.pick_photo(&output_dir, count)?;
                let json = serde_json::json!({"photos": files});
                serde_json::to_string_pretty(&json).map_err(|e| e.to_string())
            }
        }) {
            Ok(out) => CommandOutput::success(format!("{}\n", out)),
            Err(e) => e,
        }
    }

    // ── record ──
    pub fn cmd_record(&self, args: &[&str]) -> CommandOutput {
        let mut path = "/recording.wav".to_string();
        let mut duration: u32 = 10;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-d" | "--duration" => {
                    if i + 1 < args.len() {
                        duration = args[i + 1].parse().unwrap_or(10);
                        i += 1;
                    }
                }
                "-o" | "--output" => {
                    if i + 1 < args.len() {
                        path = args[i + 1].to_string();
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => {
                    path = arg.to_string();
                }
                _ => {}
            }
            i += 1;
        }
        if let Some(perm) = self.check_device_permission("microphone", "record") {
            return perm;
        }
        match plugin(self, |p| p.record_audio(&path, duration)) {
            Ok(()) => CommandOutput::success(format!("Recorded {}s to {}\n", duration, path)),
            Err(e) => e,
        }
    }

    // ── play ──
    pub fn cmd_play(&self, args: &[&str]) -> CommandOutput {
        let path = match args.first() {
            Some(p) => p.to_string(),
            None => return CommandOutput::error("play: missing file path\n".to_string(), 1),
        };
        match plugin(self, |p| p.play_audio(&path)) {
            Ok(()) => CommandOutput::success(String::new()),
            Err(e) => e,
        }
    }

    // ── say ──
    pub fn cmd_say(&self, args: &[&str]) -> CommandOutput {
        let text = args.join(" ");
        if text.is_empty() {
            return CommandOutput::error("say: missing text\n".to_string(), 1);
        }
        match plugin(self, |p| p.text_to_speech(&text)) {
            Ok(()) => CommandOutput::success(String::new()),
            Err(e) => e,
        }
    }

    // ── speech ──
    pub fn cmd_speech(&self, args: &[&str]) -> CommandOutput {
        let path = match args.first() {
            Some(p) => p.to_string(),
            None => {
                return CommandOutput::error("speech: missing audio file path\n".to_string(), 1)
            }
        };
        if let Some(perm) = self.check_device_permission("microphone", "speech") {
            return perm;
        }
        match plugin(self, |p| p.speech_to_text(&path)) {
            Ok(text) => CommandOutput::success(format!("{}\n", text)),
            Err(e) => e,
        }
    }

    // ── contacts ──
    pub fn cmd_contacts(&self, args: &[&str]) -> CommandOutput {
        let mut query = String::new();
        let mut limit = 50u32;
        let mut contact_id: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "get" => {
                    if i + 1 < args.len() {
                        contact_id = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "search" => {
                    if i + 1 < args.len() {
                        query = args[i + 1].to_string();
                        i += 1;
                    }
                }
                "-n" | "--limit" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().unwrap_or(50);
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if let Some(perm) = self.check_device_permission("contacts", "read") {
            return perm;
        }
        match plugin(self, |p| {
            if let Some(cid) = contact_id {
                let c = p.get_contact(&cid)?;
                serde_json::to_string_pretty(&c).map_err(|e| e.to_string())
            } else {
                let list = p.list_contacts(&query, limit)?;
                serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
            }
        }) {
            Ok(json) => CommandOutput::success(format!("{}\n", json)),
            Err(e) => e,
        }
    }

    // ── location ──
    pub fn cmd_location(&self, _args: &[&str]) -> CommandOutput {
        if let Some(perm) = self.check_device_permission("location", "gps") {
            return perm;
        }
        match plugin(self, |p| {
            let loc = p.get_location()?;
            serde_json::to_string_pretty(&loc).map_err(|e| e.to_string())
        }) {
            Ok(json) => CommandOutput::success(format!("{}\n", json)),
            Err(e) => e,
        }
    }

    // ── clipboard (read) ──
    pub fn cmd_clipboard(&self, args: &[&str]) -> CommandOutput {
        if args.first() == Some(&"set") {
            let text = args[1..].join(" ");
            if text.is_empty() {
                return CommandOutput::error("clipboard set: missing text\n".to_string(), 1);
            }
            return match plugin(self, |p| p.set_clipboard(&text)) {
                Ok(()) => CommandOutput::success(String::new()),
                Err(e) => e,
            };
        }
        match plugin(self, |p| p.get_clipboard()) {
            Ok(text) => CommandOutput::success(format!("{}\n", text)),
            Err(e) => e,
        }
    }

    // ── pbcopy (write clipboard) ──
    pub fn cmd_pbcopy(&self, args: &[&str]) -> CommandOutput {
        let text = args.join(" ");
        if text.is_empty() {
            return CommandOutput::error("pbcopy: missing text\n".to_string(), 1);
        }
        match plugin(self, |p| p.set_clipboard(&text)) {
            Ok(()) => CommandOutput::success(String::new()),
            Err(e) => e,
        }
    }

    // ── sensor ──
    pub fn cmd_sensor(&self, args: &[&str]) -> CommandOutput {
        let sub = args.first().map(|s| *s).unwrap_or("list");
        match sub {
            "orientation" => match plugin(self, |p| {
                let o = p.get_orientation()?;
                serde_json::to_string_pretty(&o).map_err(|e| e.to_string())
            }) {
                Ok(json) => CommandOutput::success(format!("{}\n", json)),
                Err(e) => e,
            },
            "motion" => match plugin(self, |p| {
                let m = p.get_motion()?;
                serde_json::to_string_pretty(&m).map_err(|e| e.to_string())
            }) {
                Ok(json) => CommandOutput::success(format!("{}\n", json)),
                Err(e) => e,
            },
            "light" => match plugin(self, |p| p.get_ambient_light()) {
                Ok(v) => CommandOutput::success(format!("{} lux\n", v)),
                Err(e) => e,
            },
            "proximity" => match plugin(self, |p| p.get_proximity()) {
                Ok(v) => CommandOutput::success(format!("{}\n", if v { "near" } else { "far" })),
                Err(e) => e,
            },
            "list" | _ => match plugin(self, |p| {
                let sensors = p.list_sensors()?;
                serde_json::to_string_pretty(&sensors).map_err(|e| e.to_string())
            }) {
                Ok(json) => CommandOutput::success(format!("{}\n", json)),
                Err(e) => e,
            },
        }
    }

    // ── notify ──
    pub fn cmd_notify(&self, args: &[&str]) -> CommandOutput {
        let (title, body) = if let Some(pos) = args.iter().position(|&a| a == "--") {
            let title = args[..pos].join(" ");
            let body = args[pos + 1..].join(" ");
            (title, body)
        } else if args.len() >= 2 {
            (args[0].to_string(), args[1..].join(" "))
        } else if args.len() == 1 {
            ("fastshell".to_string(), args[0].to_string())
        } else {
            return CommandOutput::error("notify-send: missing message\n".to_string(), 1);
        };
        let sound = args.contains(&"--sound");
        match plugin(self, |p| p.send_notification(&title, &body, sound)) {
            Ok(()) => CommandOutput::success(String::new()),
            Err(e) => e,
        }
    }

    // ── share ──
    pub fn cmd_share(&self, args: &[&str]) -> CommandOutput {
        let mut path: Option<String> = None;
        let mut text: Option<String> = None;
        let mut mime = "application/octet-stream".to_string();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--text" => {
                    if i + 1 < args.len() {
                        text = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "--mime" => {
                    if i + 1 < args.len() {
                        mime = args[i + 1].to_string();
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => {
                    path = Some(arg.to_string());
                }
                _ => {}
            }
            i += 1;
        }
        match (path, text) {
            (Some(p), _) => match plugin(self, |p2| p2.share_file(&p, &mime)) {
                Ok(()) => CommandOutput::success(String::new()),
                Err(e) => e,
            },
            (_, Some(t)) => match plugin(self, |p2| p2.share_text(&t)) {
                Ok(()) => CommandOutput::success(String::new()),
                Err(e) => e,
            },
            _ => CommandOutput::error("share: missing file or --text\n".to_string(), 1),
        }
    }

    // ── open ──
    pub fn cmd_open_url(&self, args: &[&str]) -> CommandOutput {
        let url = match args.first() {
            Some(u) => u.to_string(),
            None => return CommandOutput::error("open: missing URL\n".to_string(), 1),
        };
        match plugin(self, |p| p.open_url(&url)) {
            Ok(()) => CommandOutput::success(String::new()),
            Err(e) => e,
        }
    }

    // ── auth ──
    pub fn cmd_auth(&self, args: &[&str]) -> CommandOutput {
        let reason = if args.first() == Some(&"bio") {
            args.get(1)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Authenticate".to_string())
        } else {
            args.join(" ")
        };
        if reason.is_empty() {
            return CommandOutput::error("auth: missing reason\n".to_string(), 1);
        }
        match plugin(self, |p| p.authenticate_biometric(&reason)) {
            Ok(true) => CommandOutput::success("authenticated\n".to_string()),
            Ok(false) => CommandOutput::error("authentication failed\n".to_string(), 1),
            Err(e) => e,
        }
    }

    // ── battery ──
    pub fn cmd_battery(&self, _args: &[&str]) -> CommandOutput {
        match plugin(self, |p| {
            let b = p.get_battery()?;
            serde_json::to_string_pretty(&b).map_err(|e| e.to_string())
        }) {
            Ok(json) => CommandOutput::success(format!("{}\n", json)),
            Err(e) => e,
        }
    }

    // ── vibrate ──
    pub fn cmd_vibrate(&self, args: &[&str]) -> CommandOutput {
        let ms: u32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(200);
        match plugin(self, |p| p.vibrate(ms)) {
            Ok(()) => CommandOutput::success(String::new()),
            Err(e) => e,
        }
    }

    // ── screen ──
    pub fn cmd_screen(&self, args: &[&str]) -> CommandOutput {
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "brightness" => {
                    if i + 1 < args.len() {
                        if let Ok(level) = args[i + 1].parse::<f64>() {
                            let level = level.clamp(0.0, 1.0);
                            return match plugin(self, |p| p.set_brightness(level)) {
                                Ok(()) => CommandOutput::success(String::new()),
                                Err(e) => e,
                            };
                        }
                    }
                    return CommandOutput::error(
                        "screen brightness: missing or invalid level (0.0-1.0)\n".to_string(),
                        1,
                    );
                }
                "on" => {
                    return match plugin(self, |p| p.keep_screen_on(true)) {
                        Ok(()) => CommandOutput::success(String::new()),
                        Err(e) => e,
                    };
                }
                "off" => {
                    return match plugin(self, |p| p.keep_screen_on(false)) {
                        Ok(()) => CommandOutput::success(String::new()),
                        Err(e) => e,
                    };
                }
                _ => {}
            }
            i += 1;
        }
        CommandOutput::error(
            "screen: usage: screen brightness <0.0-1.0> | on | off\n".to_string(),
            1,
        )
    }

    // ── device ──
    pub fn cmd_device(&self, args: &[&str]) -> CommandOutput {
        let sub = args.first().map(|s| *s);
        match sub {
            Some("info") => match plugin(self, |p| {
                let d = p.get_device_info()?;
                serde_json::to_string_pretty(&d).map_err(|e| e.to_string())
            }) {
                Ok(json) => CommandOutput::success(format!("{}\n", json)),
                Err(e) => e,
            },
            Some("network") => match plugin(self, |p| {
                let n = p.get_network_type()?;
                serde_json::to_string_pretty(&n).map_err(|e| e.to_string())
            }) {
                Ok(json) => CommandOutput::success(format!("{}\n", json)),
                Err(e) => e,
            },
            _ => CommandOutput::error(
                "device: usage: device info | device network\n".to_string(),
                1,
            ),
        }
    }
}
