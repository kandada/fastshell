// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_sleep(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("sleep: missing operand\n".to_string(), 1);
        }

        let mut total_secs: f64 = 0.0;

        for arg in args {
            let secs = match parse_duration(arg) {
                Ok(s) => s,
                Err(e) => return CommandOutput::error(format!("sleep: {}\n", e), 1),
            };
            total_secs += secs;
        }

        std::thread::sleep(std::time::Duration::from_secs_f64(total_secs));
        CommandOutput::success(String::new())
    }
}

fn parse_duration(s: &str) -> Result<f64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("invalid time interval ''".to_string());
    }

    let (num_str, suffix) = if let Some(rest) = s.strip_suffix('s') {
        (rest, 1.0)
    } else if let Some(rest) = s.strip_suffix('m') {
        (rest, 60.0)
    } else if let Some(rest) = s.strip_suffix('h') {
        (rest, 3600.0)
    } else if let Some(rest) = s.strip_suffix('d') {
        (rest, 86400.0)
    } else {
        (s, 1.0)
    };

    let num: f64 = num_str
        .parse()
        .map_err(|_| format!("invalid time interval '{}'", s))?;
    Ok(num * suffix)
}
