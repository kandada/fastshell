// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const SLEEP_HELP_TEXT: &str = "\
Usage: sleep NUMBER[SUFFIX]...
Pause for NUMBER seconds. SUFFIX may be:

  s  seconds (default)
  m  minutes
  h  hours
  d  days
  Fractional values are accepted.
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_sleep(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(SLEEP_HELP_TEXT.to_string());
        }
        if args.is_empty() {
            return CommandOutput::error("sleep: missing operand\n".to_string(), 1);
        }

        let mut total_secs: f64 = 0.0;

        for arg in args {
            if arg.starts_with('-') {
                eprintln!("sleep: warning: unsupported option '{}'", arg);
                continue;
            }
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

#[cfg(test)]
mod tests {
    use super::Shell;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fastshell_sleep_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_sleep_help() {
        let mut s = mk_shell();
        let out = s.execute("sleep", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_sleep_help_long() {
        let mut s = mk_shell();
        let out = s.execute("sleep", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
