// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const KILL_HELP_TEXT: &str = "\
Usage: kill [OPTION]... PID...
Send a signal to a process.

  -s SIG    specify signal name (e.g. TERM, KILL)
  -NUM      specify signal number (e.g. -9)
  -SIGNAME  specify signal by name (e.g. -SIGTERM)
  -l        list signal names
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_kill(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(KILL_HELP_TEXT.to_string());
        }
        if args.contains(&"-l") {
            let signals = " 1) SIGHUP 2) SIGINT 3) SIGQUIT 4) SIGILL 5) SIGTRAP 6) SIGABRT 7) SIGBUS 8) SIGFPE 9) SIGKILL 10) SIGUSR1 11) SIGSEGV 12) SIGUSR2 13) SIGPIPE 14) SIGALRM 15) SIGTERM\n";
            return CommandOutput::success(signals.to_string());
        }
        if args.is_empty() {
            return CommandOutput::error(
                "kill: usage: kill [-signal|-s signal] pid...\n".to_string(),
                1,
            );
        }

        let mut signal: i32 = 15;
        let mut pid_start = 0usize;

        if args[0].starts_with('-') {
            let sig_str = &args[0][1..];
            if sig_str.eq_ignore_ascii_case("s") {
                if args.len() < 2 {
                    return CommandOutput::error(
                        "kill: -s requires a signal name\n".to_string(),
                        1,
                    );
                }
                signal = match crate::shell::parse_signal(args[1]) {
                    Some(s) => s,
                    None => {
                        return CommandOutput::error(
                            format!("kill: invalid signal: {}\n", args[1]),
                            1,
                        );
                    }
                };
                pid_start = 2;
            } else {
                signal = match crate::shell::parse_signal(sig_str) {
                    Some(s) => s,
                    None => {
                        if let Ok(n) = sig_str.parse::<i32>() {
                            n
                        } else {
                            return CommandOutput::error(
                                format!("kill: invalid signal: {}\n", sig_str),
                                1,
                            );
                        }
                    }
                };
                pid_start = 1;
            }
        }

        if pid_start >= args.len() {
            return CommandOutput::error("kill: missing pid argument\n".to_string(), 1);
        }

        for pid_str in &args[pid_start..] {
            let pid: libc::pid_t = match pid_str.parse() {
                Ok(p) => p,
                Err(_) => {
                    return CommandOutput::error(format!("kill: {}: invalid pid\n", pid_str), 1);
                }
            };

            #[cfg(unix)]
            {
                let ret = unsafe { libc::kill(pid, signal) };
                if ret != 0 {
                    let err = std::io::Error::last_os_error();
                    return CommandOutput::error(format!("kill: {}: {}\n", pid, err), 1);
                }
            }
            #[cfg(not(unix))]
            {
                return CommandOutput::error(
                    "kill: not supported on this platform\n".to_string(),
                    1,
                );
            }
        }

        CommandOutput::success(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fastshell_kill_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_kill_help() {
        let mut s = mk_shell();
        let out = s.execute("kill", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_kill_help_long() {
        let mut s = mk_shell();
        let out = s.execute("kill", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_kill_list_signals() {
        let mut s = mk_shell();
        let out = s.execute("kill", &["-l"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("SIG"));
    }
}
