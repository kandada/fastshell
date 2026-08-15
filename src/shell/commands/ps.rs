// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const PS_HELP_TEXT: &str = "\
Usage: ps [OPTION]...
Report a snapshot of the current processes.

  -e, -A       select all processes
  -f           full-format listing (accepted)
  -o FORMAT    user-defined output format (pid,ppid,rss,pcpu,comm)
  -p PID[,PID]...  select by PID
  -u UID       select by effective user ID
  -C CMD       select by command name
  aux          show all processes (BSD style)
  -ef          show all processes (System V style)
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_ps(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(PS_HELP_TEXT.to_string());
        }
        let mut format: Option<String> = None;
        let mut pids: Vec<u32> = Vec::new();
        let mut user_filter: Option<u32> = None;
        let mut cmd_filter: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if a == "aux" || a == "ax" {
                // BSD all-processes style — nothing else needed.
            } else if a.starts_with("--") {
                match a {
                    "--forest" | "--no-headers" | "--sort" | "--pid" | "--user" | "--format"
                    | "--help" => {}
                    _ => crate::warn!("ps: warning: unsupported option '{}'", a),
                }
            } else if a.starts_with('-') && a.len() > 1 {
                let chars: Vec<char> = a.chars().skip(1).collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'e' | 'A' | 'a' | 'x' | 'f' | 'l' | 'j' | 'w' => {}
                        'o' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            format = Some(val);
                            j = chars.len();
                            continue;
                        }
                        'p' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            for pid_str in val.split(',') {
                                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                                    pids.push(pid);
                                }
                            }
                            j = chars.len();
                            continue;
                        }
                        'u' | 'U' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            user_filter = val.parse::<u32>().ok();
                            j = chars.len();
                            continue;
                        }
                        'C' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            cmd_filter = Some(val);
                            j = chars.len();
                            continue;
                        }
                        _ => crate::warn!("ps: warning: unsupported option '-{}'", chars[j]),
                    }
                    j += 1;
                }
            }
            i += 1;
        }

        let processes = crate::shell::list_processes();
        match processes {
            Ok(procs) => {
                let filtered: Vec<&crate::shell::ProcInfo> = procs
                    .iter()
                    .filter(|p| {
                        let pid_ok = pids.is_empty() || pids.contains(&p.pid);
                        let user_ok = match user_filter {
                            Some(uid) => p.uid == uid,
                            None => true,
                        };
                        let cmd_ok = match &cmd_filter {
                            Some(c) => p.comm == *c,
                            None => true,
                        };
                        pid_ok && user_ok && cmd_ok
                    })
                    .collect();

                if let Some(ref fmt) = format {
                    let fields: Vec<&str> = fmt.split(',').map(|s| s.trim()).collect();
                    let mut output = String::new();
                    for p in &filtered {
                        let mut parts = Vec::new();
                        for field in &fields {
                            parts.push(match *field {
                                "pid" => format!("{}", p.pid),
                                "ppid" => format!("{}", p.ppid),
                                "rss" => format!("{}", p.rss),
                                "pcpu" => format!("{:.1}", p.cpu_pct),
                                "comm" => p.comm.clone(),
                                "state" => "?".to_string(),
                                _ => String::new(),
                            });
                        }
                        output.push_str(&parts.join(" "));
                        output.push('\n');
                    }
                    return CommandOutput::success(output);
                }

                let mut output = String::new();
                output.push_str(&format!(
                    "{:>8} {:>8} {:>8} {:>8} {}\n",
                    "PID", "PPID", "%CPU", "RSS", "COMMAND"
                ));
                for p in &filtered {
                    output.push_str(&format!(
                        "{:>8} {:>8} {:>7.1} {:>8} {}\n",
                        p.pid,
                        p.ppid,
                        p.cpu_pct,
                        crate::shell::human_size(p.rss * 1024),
                        p.comm
                    ));
                }
                CommandOutput::success(output)
            }
            Err(e) => CommandOutput::error(format!("ps: {}\n", e), 1),
        }
    }

    /// `top` / `htop` — in this non-interactive sandbox, print a single
    /// process snapshot (equivalent to `top -b -n 1`), sorted by CPU.
    pub fn cmd_top(&self, args: &[&str]) -> CommandOutput {
        let mut count = 10usize;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-b" | "-p" => {}
                "-n" => {
                    if i + 1 < args.len() {
                        // iterations are meaningless for a snapshot; ignore the value
                        let _ = args[i + 1].parse::<u64>().unwrap_or(1);
                        i += 1;
                    }
                }
                "-d" => {
                    if i + 1 < args.len() {
                        i += 1;
                    }
                }
                a if a.starts_with('-') => {
                    crate::warn!("top: warning: unsupported option '{}'", a);
                }
                _ => {
                    if let Ok(n) = args[i].parse::<usize>() {
                        count = n;
                    }
                }
            }
            i += 1;
        }

        match crate::shell::list_processes() {
            Ok(mut procs) => {
                procs.sort_by(|a, b| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal));
                let mut output = String::new();
                output.push_str(&format!(
                    "PID       %CPU   RSS      COMMAND\n"
                ));
                for p in procs.iter().take(count) {
                    output.push_str(&format!(
                        "{:<10} {:>5.1} {:>7}  {}\n",
                        p.pid,
                        p.cpu_pct,
                        crate::shell::human_size(p.rss * 1024),
                        p.comm
                    ));
                }
                CommandOutput::success(output)
            }
            Err(e) => CommandOutput::error(format!("top: {}\n", e), 1),
        }
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
        let dir = std::env::temp_dir().join(format!("fastshell_ps_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_ps_help() {
        let mut s = mk_shell();
        let out = s.execute("ps", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_ps_help_long() {
        let mut s = mk_shell();
        let out = s.execute("ps", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_ps_default() {
        let mut s = mk_shell();
        let out = s.execute("ps", &[], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("PID"));
    }

    #[test]
    fn test_ps_aux() {
        let mut s = mk_shell();
        let out = s.execute("ps", &["aux"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("PID"));
    }

    #[test]
    fn test_ps_ef() {
        let mut s = mk_shell();
        let out = s.execute("ps", &["-ef"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("PID"));
    }

    #[test]
    fn test_top_snapshot() {
        let mut s = mk_shell();
        let out = s.execute("top", &["-b", "-n", "1"], None);
        assert_eq!(out.exit_code, 0, "top should return a snapshot: {}", out.stderr);
        assert!(out.stdout.contains("PID"), "top output should have a header: {}", out.stdout);
    }
}
