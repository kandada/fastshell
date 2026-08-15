// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::fs;

const TOUCH_HELP_TEXT: &str = "\
Usage: touch [OPTION]... FILE...
Update the access and modification times of each FILE to the current time.

  -a                     change only the access time
  -c, --no-create        do not create any files
  -m                     change only the modification time
  -d, --date=STRING      parse STRING and use it instead of current time
  -t STAMP               use [[CC]YY]MMDDhhmm[.ss] instead of current time
  -r, --reference=FILE   use this file's times instead of current time
  -h, --help             display this help and exit
";

impl Shell {
    pub fn cmd_touch(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(TOUCH_HELP_TEXT.to_string());
        }

        let mut no_create = false;
        let mut only_atime = false;
        let mut only_mtime = false;
        let mut reference: Option<String> = None;
        let mut date_string: Option<String> = None;
        let mut stamp: Option<String> = None;
        let mut files: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            match arg {
                "-c" | "--no-create" => no_create = true,
                "-a" => only_atime = true,
                "-m" => only_mtime = true,
                "-r" | "--reference" => {
                    if i + 1 < args.len() {
                        reference = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-d" | "--date" => {
                    if i + 1 < args.len() {
                        date_string = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-t" => {
                    if i + 1 < args.len() {
                        stamp = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                a if a.starts_with("--date=") => date_string = Some(a[7..].to_string()),
                a if a.starts_with("--reference=") => reference = Some(a[12..].to_string()),
                a if a.starts_with('-') && a.len() > 1 => {
                    crate::warn!("touch: warning: unsupported option '{}'", a);
                }
                _ => files.push(arg.to_string()),
            }
            i += 1;
        }

        if files.is_empty() {
            return CommandOutput::error("touch: missing file operand\n".to_string(), 1);
        }

        // Resolve the target timestamp.
        let target_time: Option<filetime::FileTime> = if let Some(r) = &reference {
            match self.vfs.resolve(r, &self.cwd) {
                Ok(p) => std::fs::metadata(&p)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(filetime::FileTime::from_system_time),
                Err(_) => None,
            }
        } else {
            let secs = if let Some(d) = &date_string {
                if let Some(stripped) = d.strip_prefix('@') {
                    stripped.parse::<u64>().ok()
                } else {
                    let normalized = d.replacen(' ', "T", 1);
                    crate::shell::commands::date::parse_iso8601(&normalized)
                        .or_else(|| crate::shell::commands::date::parse_relative_time(d))
                }
            } else if let Some(t) = &stamp {
                parse_touch_stamp(t)
            } else {
                None
            };
            secs.map(|s| filetime::FileTime::from_unix_time(s as i64, 0))
        };

        let now = filetime::FileTime::now();
        let ft = target_time.unwrap_or(now);

        for arg in &files {
            let target = match self.vfs.resolve(arg, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("touch: {}: {}\n", arg, e), 1),
            };

            if !target.exists() {
                if no_create {
                    continue;
                }
                match fs::File::create(&target) {
                    Ok(_) => {}
                    Err(e) => {
                        return CommandOutput::error(format!("touch: {}: {}\n", arg, e), 1);
                    }
                }
            }

            let set_atime = !only_mtime;
            let set_mtime = !only_atime;
            if set_atime {
                if let Err(e) = filetime::set_file_atime(&target, ft) {
                    return CommandOutput::error(format!("touch: {}: {}\n", arg, e), 1);
                }
            }
            if set_mtime {
                if let Err(e) = filetime::set_file_mtime(&target, ft) {
                    return CommandOutput::error(format!("touch: {}: {}\n", arg, e), 1);
                }
            }
        }

        CommandOutput::success(String::new())
    }
}

/// Parse GNU `touch -t` stamp format `[[CC]YY]MMDDhhmm[.ss]`.
fn parse_touch_stamp(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (date_part, secs) = match s.find('.') {
        Some(dot) => (&s[..dot], s[dot + 1..].parse::<u64>().unwrap_or(0)),
        None => (s, 0),
    };

    let (year, mmddhhmm) = match date_part.len() {
        12 => (date_part[0..4].parse::<i32>().ok()?, &date_part[4..]),
        10 => {
            let yy = date_part[0..2].parse::<i32>().ok()?;
            (if yy >= 69 { 1900 + yy } else { 2000 + yy }, &date_part[2..])
        }
        8 => {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs();
            let (y, _, _) = crate::shell::civil_from_days((now_secs / 86400) as i32);
            (y, date_part)
        }
        _ => return None,
    };

    let month: i32 = mmddhhmm[0..2].parse().ok()?;
    let day: i32 = mmddhhmm[2..4].parse().ok()?;
    let hour: u64 = mmddhhmm[4..6].parse().ok()?;
    let minute: u64 = mmddhhmm[6..8].parse().ok()?;

    let days = crate::shell::commands::date::parse_iso8601(&format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:00",
        year, month, day, hour, minute
    ))?;

    Some(days + secs)
}
