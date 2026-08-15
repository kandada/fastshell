// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const DATE_HELP_TEXT: &str = "\
Usage: date [OPTION]... [+FORMAT]
Display the current time in the given FORMAT, or set the system date.

  -u        print or set Coordinated Universal Time (UTC)
  -d STRING display time described by STRING (ISO 8601)
  +FORMAT   output date/time according to FORMAT (e.g. +%Y-%m-%d)
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_date(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(DATE_HELP_TEXT.to_string());
        }
        let mut use_utc = false;
        let mut date_str: Option<String> = None;
        let mut format = None;
        let mut rfc_email = false;
        let mut iso8601 = false;
        let mut iso_suffix = String::new();
        let mut i = 0;

        while i < args.len() {
            match args[i] {
                "-u" | "--utc" | "--universal" => use_utc = true,
                "-d" | "--date" => {
                    if i + 1 < args.len() {
                        date_str = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-R" | "--rfc-email" | "--rfc-2822" => rfc_email = true,
                "-I" | "--iso-8601" => {
                    iso8601 = true;
                    iso_suffix = "date".to_string();
                }
                a if a.starts_with("-I") && a.len() > 2 => {
                    iso8601 = true;
                    iso_suffix = a[2..].to_string();
                }
                a if a.starts_with("--iso-8601=") => {
                    iso8601 = true;
                    iso_suffix = a[11..].to_string();
                }
                a if a.starts_with("--date=") => {
                    date_str = Some(a[7..].to_string());
                }
                arg if arg.starts_with('+') => {
                    format = Some(arg[1..].to_string());
                }
                _ => {
                    crate::warn!("date: warning: unsupported option '{}'", args[i]);
                }
            }
            i += 1;
        }

        let secs = if let Some(ref ds) = date_str {
            match parse_iso8601(ds) {
                Some(s) => s,
                None => match parse_relative_time(ds) {
                    Some(s) => s,
                    None => return CommandOutput::error(format!("date: invalid date '{}'\n", ds), 1),
                },
            }
        } else {
            let dur = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let s = dur.as_secs();
            if use_utc {
                s
            } else {
                s
            }
        };

        let output = if rfc_email {
            format!("{}\n", format_rfc2822(secs))
        } else if iso8601 {
            format!("{}\n", format_iso8601(secs, &iso_suffix))
        } else {
            match format {
                Some(ref fmt) => format_date(secs, use_utc, fmt),
                None => format!("{}\n", crate::shell::format_unix_time(secs)),
            }
        };

        CommandOutput::success(output)
    }
}

fn format_rfc2822(secs: u64) -> String {
    let days_since_epoch = (secs / 86400) as i32;
    let tod = secs % 86400;
    let (year, month, day) = crate::shell::civil_from_days(days_since_epoch);
    let (hour, minute, second) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let weekday = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        [((days_since_epoch as i64 + 4) % 7) as usize];
    let month_names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        weekday, day, month_names[(month - 1).max(0) as usize], year, hour, minute, second
    )
}

fn format_iso8601(secs: u64, suffix: &str) -> String {
    let days_since_epoch = (secs / 86400) as i32;
    let tod = secs % 86400;
    let (year, month, day) = crate::shell::civil_from_days(days_since_epoch);
    let (hour, minute, second) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    match suffix {
        "seconds" => format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00", year, month, day, hour, minute, second),
        "minutes" => format!("{:04}-{:02}-{:02}T{:02}:{:02}+00:00", year, month, day, hour, minute),
        "hours" => format!("{:04}-{:02}-{:02}T{:02}:00+00:00", year, month, day, hour),
        "ns" => format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000000000+00:00", year, month, day, hour, minute, second),
        _ => format!("{:04}-{:02}-{:02}", year, month, day),
    }
}

/// Parse a GNU-date-style relative time expression: `yesterday`, `tomorrow`,
/// `-3 days`, `+2 hours`, `5 minutes ago`, etc. Returns epoch seconds.
pub(crate) fn parse_relative_time(s: &str) -> Option<u64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let s = s.trim().to_lowercase();

    let delta: Option<i64> = if s == "yesterday" {
        Some(-86400)
    } else if s == "tomorrow" {
        Some(86400)
    } else if s == "now" {
        Some(0)
    } else if let Some(rest) = s.strip_prefix('-') {
        Some(-parse_duration(rest)?)
    } else if let Some(rest) = s.strip_prefix('+') {
        Some(parse_duration(rest)?)
    } else if let Some(rest) = s.strip_suffix(" ago") {
        Some(-parse_duration(rest.trim())?)
    } else if let Some(rest) = s.strip_suffix(" hence") {
        Some(parse_duration(rest.trim())?)
    } else {
        None
    };

    delta.map(|d| (now + d).max(0) as u64)
}

/// Parse a duration like "3 days", "2 hours", "10 minutes" into seconds.
fn parse_duration(s: &str) -> Option<i64> {
    let mut parts = s.trim().split_whitespace();
    let num: i64 = parts.next()?.parse().ok()?;
    let unit = parts.next().unwrap_or("second").trim_end_matches('s');
    match unit {
        "second" | "sec" => Some(num),
        "minute" | "min" => Some(num * 60),
        "hour" | "hr" => Some(num * 3600),
        "day" => Some(num * 86400),
        "week" => Some(num * 7 * 86400),
        "month" => Some(num * 30 * 86400),
        "year" => Some(num * 365 * 86400),
        _ => None,
    }
}

pub(crate) fn parse_iso8601(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.len() < 10 {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    if s.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    let month: i32 = s[5..7].parse().ok()?;
    if s.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let day: i32 = s[8..10].parse().ok()?;

    let (hour, minute, second) = if s.len() >= 19 && s.as_bytes().get(10) == Some(&b'T') {
        (
            s[11..13].parse::<u64>().ok()?,
            s[14..16].parse::<u64>().ok()?,
            s[17..19].parse::<u64>().ok()?,
        )
    } else {
        (0, 0, 0)
    };

    let days_since_epoch = days_to_epoch(year, month, day);

    Some(days_since_epoch * 86400 + hour * 3600 + minute * 60 + second)
}

fn days_to_epoch(y: i32, m: i32, d: i32) -> u64 {
    let (mut y, mut m) = (y as i64, m as i64);
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let epoch: i64 = 719468;
    let days = (era * 146097) as i64 + doe - epoch;
    days as u64
}

fn format_date(secs: u64, use_utc: bool, fmt: &str) -> String {
    let days_since_epoch = (secs / 86400) as i32;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = crate::shell::civil_from_days(days_since_epoch);

    let hour_12 = if hours == 0 {
        12
    } else if hours > 12 {
        hours - 12
    } else {
        hours
    };
    let am_pm = if hours < 12 { "AM" } else { "PM" };

    let doy = compute_doy(year, month, day);

    let week_number = compute_week_number(year, month, day);

    let weekday_index = (days_since_epoch as i64 + 4) % 7;
    // 0=Sunday in the epoch calculation; adjust to 0=Sunday
    let weekday_names = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let month_names = [
        "",
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

    let tz_offset = if use_utc { "+0000".to_string() } else { timezone_offset() };

    let mut result = fmt.to_string();
    result = result.replace("%F", &format!("{:04}-{:02}-{:02}", year, month, day));
    result = result.replace("%T", &format!("{:02}:{:02}:{:02}", hours, minutes, seconds));
    result = result.replace("%e", &format!("{:2}", day));
    result = result.replace("%Y", &format!("{:04}", year));
    result = result.replace("%m", &format!("{:02}", month));
    result = result.replace("%d", &format!("{:02}", day));
    result = result.replace("%H", &format!("{:02}", hours));
    result = result.replace("%M", &format!("{:02}", minutes));
    result = result.replace("%S", &format!("{:02}", seconds));
    result = result.replace("%s", &format!("{}", secs));
    result = result.replace("%z", &tz_offset);
    result = result.replace("%A", weekday_names[weekday_index as usize]);
    result = result.replace("%B", month_names[month as usize]);
    result = result.replace("%I", &format!("{:02}", hour_12));
    result = result.replace("%p", am_pm);
    result = result.replace("%j", &format!("{:03}", doy));
    result = result.replace("%U", &format!("{:02}", week_number));
    result.push('\n');
    result
}

fn compute_doy(year: i32, month: i32, day: i32) -> u64 {
    let month_days = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut doy = month_days[month as usize - 1] + day as u64;
    if month > 2 && is_leap(year) {
        doy += 1;
    }
    doy
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn compute_week_number(year: i32, month: i32, day: i32) -> u64 {
    let doy = compute_doy(year, month, day) as i64;
    let jan1_dow = (epoch_days(year, 1, 1) as i64 + 4) % 7;
    let week = (doy + jan1_dow - 1) / 7;
    if week < 0 {
        0
    } else {
        week as u64
    }
}

fn epoch_days(y: i32, m: i32, d: i32) -> u64 {
    days_to_epoch(y, m, d)
}

fn timezone_offset() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let offset_secs = tm.tm_gmtoff;
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs_secs = offset_secs.abs();
    let hours = abs_secs / 3600;
    let minutes = (abs_secs % 3600) / 60;
    format!("{}{:02}{:02}", sign, hours, minutes)
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fastshell_date_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_date_help() {
        let mut s = mk_shell();
        let out = s.execute("date", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_date_help_long() {
        let mut s = mk_shell();
        let out = s.execute("date", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_date_full_date_format() {
        let mut s = mk_shell();
        let out = s.execute("date", &["+%F"], None);
        // %F = %Y-%m-%d
        let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
        assert!(re.is_match(out.stdout.trim()), "%F should be YYYY-MM-DD, got: {}", out.stdout);
    }

    #[test]
    fn test_date_full_time_format() {
        let mut s = mk_shell();
        let out = s.execute("date", &["+%T"], None);
        // %T = %H:%M:%S
        let re = regex::Regex::new(r"^\d{2}:\d{2}:\d{2}$").unwrap();
        assert!(re.is_match(out.stdout.trim()), "%T should be HH:MM:SS, got: {}", out.stdout);
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(super::parse_duration("3 days"), Some(3 * 86400));
        assert_eq!(super::parse_duration("2 hours"), Some(2 * 3600));
        assert_eq!(super::parse_duration("10 minutes"), Some(10 * 60));
        assert_eq!(super::parse_duration("1 week"), Some(7 * 86400));
    }

    #[test]
    fn test_parse_relative_time_yesterday() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let yesterday = super::parse_relative_time("yesterday").unwrap();
        assert!(
            (yesterday as i64 - (now as i64 - 86400)).abs() < 5,
            "yesterday should be ~now-86400"
        );
    }

    #[test]
    fn test_parse_relative_time_ago() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let v = super::parse_relative_time("5 minutes ago").unwrap();
        assert!((v as i64 - (now as i64 - 300)).abs() < 5, "5 minutes ago should be ~now-300");
    }

    #[test]
    fn test_date_d_yesterday() {
        let mut s = mk_shell();
        let out = s.execute("date", &["-d", "yesterday", "+%s"], None);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let val: i64 = out.stdout.trim().parse().unwrap();
        assert!((val - (now as i64 - 86400)).abs() < 5, "date -d yesterday +%s should be ~now-86400");
    }
}
