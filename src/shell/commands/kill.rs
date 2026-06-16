use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_kill(&self, args: &[&str]) -> CommandOutput {
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
                    return CommandOutput::error("kill: -s requires a signal name\n".to_string(), 1);
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
                    return CommandOutput::error(
                        format!("kill: {}: invalid pid\n", pid_str),
                        1,
                    );
                }
            };

            #[cfg(unix)]
            {
                let ret = unsafe { libc::kill(pid, signal) };
                if ret != 0 {
                    let err = std::io::Error::last_os_error();
                    return CommandOutput::error(
                        format!("kill: {}: {}\n", pid, err),
                        1,
                    );
                }
            }
            #[cfg(not(unix))]
            {
                return CommandOutput::error("kill: not supported on this platform\n".to_string(), 1);
            }
        }

        CommandOutput::success(String::new())
    }
}
