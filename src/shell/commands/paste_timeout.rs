use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_paste(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut delimiter = '\t';
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-d" => {
                    if i + 1 < args.len() {
                        delimiter = args[i + 1].chars().next().unwrap_or('\t');
                        i += 1;
                    }
                }
                arg if arg.starts_with("-d") && arg.len() > 2 => {
                    delimiter = arg[2..].chars().next().unwrap_or('\t');
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
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
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(content) => {
                        columns.push(content.lines().map(|l| l.to_string()).collect());
                    }
                    Err(e) => return CommandOutput::error(format!("paste: {}: {}\n", file, e), 1),
                }
            }
        }

        let max_rows = columns.iter().map(|c| c.len()).max().unwrap_or(0);
        let mut output = String::new();
        for row in 0..max_rows {
            let parts: Vec<&str> = columns.iter()
                .map(|col| col.get(row).map(|s| s.as_str()).unwrap_or(""))
                .collect();
            output.push_str(&parts.join(&delimiter.to_string()));
            output.push('\n');
        }

        CommandOutput::success(output)
    }

    pub fn cmd_timeout(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("timeout: missing duration\n".to_string(), 1);
        }

        let duration = match parse_timeout_duration(args[0]) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("timeout: {}\n", e), 1),
        };

        if args.len() < 2 {
            return CommandOutput::error("timeout: missing command\n".to_string(), 1);
        }

        let cmd = args[1];
        let cmd_args: Vec<&str> = args[2..].to_vec();
        let vfs_root = self.vfs.root().to_path_buf();
        let cwd = if self.cwd == "/" {
            vfs_root.clone()
        } else {
            vfs_root.join(self.cwd.trim_start_matches('/'))
        };

        let child = match std::process::Command::new(cmd)
            .args(&cmd_args)
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
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

    let secs: f64 = num_str.parse().map_err(|_| format!("invalid duration '{}'", s))?;
    Ok(std::time::Duration::from_secs_f64(secs * multiplier))
}
