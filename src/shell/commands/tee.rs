// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const TEE_HELP_TEXT: &str = "\
Usage: tee [OPTION]... [FILE]...
Copy standard input to each FILE, and also to standard output.

  -a         append to the given FILEs, do not overwrite
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_tee(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(TEE_HELP_TEXT.to_string());
        }
        let mut append = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-a" => append = true,
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {
                    eprintln!("tee: warning: unsupported option '{}'", arg);
                }
            }
        }

        if files.is_empty() {
            return CommandOutput::error("tee: missing file operand\n".to_string(), 1);
        }

        let input = match stdin {
            Some(s) => s.to_string(),
            None => return CommandOutput::error("tee: no input\n".to_string(), 1),
        };

        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(r) => r,
                Err(e) => return CommandOutput::error(format!("tee: {}: {}\n", file, e), 1),
            };
            let result: Result<(), String> = if append {
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&resolved)
                    .map_err(|e| e.to_string())
                    .and_then(|mut f| f.write_all(input.as_bytes()).map_err(|e| e.to_string()))
            } else {
                self.vfs
                    .write(file, &self.cwd, &input)
                    .map_err(|e| e.to_string())
            };
            if let Err(e) = result {
                return CommandOutput::error(format!("tee: {}: {}\n", file, e), 1);
            }
        }

        CommandOutput::success(input)
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
        let dir = std::env::temp_dir().join(format!("fastshell_tee_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_tee_help() {
        let mut s = mk_shell();
        let out = s.execute("tee", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_tee_help_long() {
        let mut s = mk_shell();
        let out = s.execute("tee", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
