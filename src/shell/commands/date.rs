// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_date(&self, args: &[&str]) -> CommandOutput {
        let mut use_utc = false;
        let mut date_str: Option<String> = None;
        let mut format = None;
        let mut i = 0;

        while i < args.len() {
            match args[i] {
                "-u" => use_utc = true,
                "-d" => {
                    if i + 1 < args.len() {
                        date_str = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                arg if arg.starts_with('+') => {
                    format = Some(arg[1..].to_string());
                }
                _ => {}
            }
            i += 1;
        }

        let secs = if let Some(ref ds) = date_str {
            match parse_iso8601(ds) {
                Some(s) => s,
                None => return CommandOutput::error(format!("date: invalid date '{}'\n", ds), 1),
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

        let output = match format {
            Some(ref fmt) => format_date(secs, use_utc, fmt),
            None => {
                if use_utc {
                    format!("{}\n", crate::shell::format_unix_time(secs))
                } else {
                    format!("{}\n", crate::shell::format_unix_time(secs))
                }
            }
        };

        CommandOutput::success(output)
    }
}

fn parse_iso8601(s: &str) -> Option<u64> {
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

    let tz_offset = if use_utc { "+0000" } else { timezone_offset() };

    let mut result = fmt.to_string();
    result = result.replace("%Y", &format!("{:04}", year));
    result = result.replace("%m", &format!("{:02}", month));
    result = result.replace("%d", &format!("{:02}", day));
    result = result.replace("%H", &format!("{:02}", hours));
    result = result.replace("%M", &format!("{:02}", minutes));
    result = result.replace("%S", &format!("{:02}", seconds));
    result = result.replace("%s", &format!("{}", secs));
    result = result.replace("%z", tz_offset);
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

fn timezone_offset() -> &'static str {
    "+0000"
}
