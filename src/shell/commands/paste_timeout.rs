// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_paste(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut delimiter = '\t';
        let mut serial = false;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg.starts_with("--") {
                match arg {
                    "--serial" => serial = true,
                    "--delimiters" => {
                        if i + 1 < args.len() {
                            delimiter = args[i + 1].chars().next().unwrap_or('\t');
                            i += 1;
                        }
                    }
                    _ => {}
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg.chars().skip(1).collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        's' => serial = true,
                        'd' => {
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
                            delimiter = val.chars().next().unwrap_or('\t');
                            j = chars.len();
                            continue;
                        }
                        _ => {}
                    }
                    j += 1;
                }
            } else {
                files.push(arg.to_string());
            }
            i += 1;
        }

        let mut columns: Vec<Vec<String>> = Vec::new();

        if files.is_empty() {
            match stdin {
                Some(s) => {
                    columns.push(s.lines().map(|l| l.to_string()).collect());
                }
                None => return CommandOutput::error("paste: missing input\n".to_string(), 1),
            }
        } else {
            for file in &files {
                // `-` means read from stdin (GNU conv: one `-` per file slot).
                if file == "-" {
                    match stdin {
                        Some(s) => columns.push(s.lines().map(|l| l.to_string()).collect()),
                        None => columns.push(Vec::new()),
                    }
                    continue;
                }
                match self.read_text_lossy(file) {
                    Ok(content) => {
                        columns.push(content.lines().map(|l| l.to_string()).collect());
                    }
                    Err(e) => return CommandOutput::error(format!("paste: {}: {}\n", file, e), 1),
                }
            }
        }

        let mut output = String::new();
        if serial {
            for col in &columns {
                output.push_str(&col.join(&delimiter.to_string()));
                output.push('\n');
            }
        } else {
            let max_rows = columns.iter().map(|c| c.len()).max().unwrap_or(0);
            for row in 0..max_rows {
                let parts: Vec<&str> = columns
                    .iter()
                    .map(|col| col.get(row).map(|s| s.as_str()).unwrap_or(""))
                    .collect();
                output.push_str(&parts.join(&delimiter.to_string()));
                output.push('\n');
            }
        }

        CommandOutput::success(output)
    }

    pub fn cmd_timeout(&mut self, args: &[&str]) -> CommandOutput {
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-s" | "--signal" | "-k" | "--kill-after" => {
                    if i + 1 < args.len() {
                        i += 1; // consume signal/duration value (not enforced in sandbox)
                    }
                }
                "--preserve-status" | "--foreground" => {}
                a if a.starts_with('-') && a.len() > 1 => {
                    return CommandOutput::error(
                        format!("timeout: unrecognized option '{}'\n", a),
                        125,
                    );
                }
                _ => break,
            }
            i += 1;
        }

        if i >= args.len() {
            return CommandOutput::error("timeout: missing duration\n".to_string(), 1);
        }

        let duration = match parse_timeout_duration(args[i]) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("timeout: {}\n", e), 1),
        };
        i += 1;

        if i >= args.len() {
            return CommandOutput::error("timeout: missing command\n".to_string(), 1);
        }

        let cmd = args[i];
        let cmd_args: Vec<&str> = args[i + 1..].to_vec();
        let vfs_root = self.vfs.root().to_path_buf();
        let cwd = if self.cwd == "/" {
            vfs_root.clone()
        } else {
            vfs_root.join(self.cwd.trim_start_matches('/'))
        };

        // Only spawn when external execution is allowed; otherwise fall straight
        // to the in-process path (avoids Android seccomp → SIGSYS).
        let spawn_result = if self.allow_subprocess {
            std::process::Command::new(cmd)
                .args(&cmd_args)
                .current_dir(&cwd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "external commands are disabled",
            ))
        };
        let child = match spawn_result {
            Ok(c) => c,
            // Not a real binary → a fastshell builtin (`sleep`, `wget`, `battery`,
            // …). Run it in-process, but arm a watchdog that sets the shell
            // cancel flag after `duration` so long-running builtins actually
            // abort (they poll `cancel`). Restore the previous flag afterwards.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                use std::sync::atomic::{AtomicBool, Ordering};
                use std::sync::Arc;
                let cancel = self.cancel.clone();
                let prev = cancel.load(Ordering::SeqCst);
                cancel.store(false, Ordering::SeqCst);
                let done = Arc::new(AtomicBool::new(false));
                let wd_cancel = cancel.clone();
                let wd_done = done.clone();
                std::thread::spawn(move || {
                    let deadline = std::time::Instant::now() + duration;
                    while std::time::Instant::now() < deadline {
                        if wd_done.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    if !wd_done.load(Ordering::SeqCst) {
                        wd_cancel.store(true, Ordering::SeqCst);
                    }
                });
                let out = self.execute(cmd, &cmd_args, None);
                done.store(true, Ordering::SeqCst);
                cancel.store(prev, Ordering::SeqCst);
                if out.exit_code == 143 && out.stderr.contains("cancelled") {
                    return CommandOutput::error("timeout: command timed out\n".to_string(), 124);
                }
                return out;
            }
            Err(e) => return CommandOutput::error(format!("timeout: failed to spawn: {}\n", e), 1),
        };

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = child.wait_with_output();
            let _ = tx.send(result);
        });

        match rx.recv_timeout(duration) {
            Ok(output_result) => match output_result {
                Ok(out) => CommandOutput {
                    stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                    exit_code: out.status.code().unwrap_or(-1),
                },
                Err(e) => CommandOutput::error(format!("timeout: {}\n", e), 1),
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                CommandOutput::error("timeout: command timed out\n".to_string(), 124)
            }
            Err(_) => CommandOutput::error("timeout: internal error\n".to_string(), 1),
        }
    }
}

fn parse_timeout_duration(s: &str) -> Result<std::time::Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("invalid duration".to_string());
    }

    let (num_str, multiplier) = if let Some(rest) = s.strip_suffix('s') {
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

    let secs: f64 = num_str
        .parse()
        .map_err(|_| format!("invalid duration '{}'", s))?;
    Ok(std::time::Duration::from_secs_f64(secs * multiplier))
}
