// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const RM_HELP_TEXT: &str = "\
Usage: rm [OPTION]... FILE...
Remove (unlink) the FILE(s).

  -r, -R  remove directories and their contents recursively
  -f      ignore nonexistent files and arguments, never prompt
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_rm(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(RM_HELP_TEXT.to_string());
        }
        let mut recursive = false;
        let mut force = false;
        let mut targets = Vec::new();

        for arg in args {
            match *arg {
                "-r" | "-R" => recursive = true,
                "-f" => force = true,
                "-rf" | "-fr" => {
                    recursive = true;
                    force = true;
                }
                _ if arg.starts_with('-') => {
                    eprintln!("rm: warning: unsupported option '{}'", arg);
                }
                _ => targets.push(arg.to_string()),
            }
        }

        if targets.is_empty() {
            return CommandOutput::error("rm: missing operand\n".to_string(), 1);
        }

        for target in &targets {
            let resolved = match self.vfs.resolve(target, &self.cwd) {
                Ok(p) => p,
                Err(e) => {
                    if force {
                        continue;
                    }
                    return CommandOutput::error(format!("rm: {}: {}\n", target, e), 1);
                }
            };

            if !resolved.exists() {
                if force {
                    continue;
                }
                return CommandOutput::error(
                    format!("rm: {}: No such file or directory\n", target),
                    1,
                );
            }

            let result = if resolved.is_dir() {
                if recursive {
                    self.vfs.remove_dir_all(target, &self.cwd)
                } else {
                    Err(crate::vfs::VfsError::NotADirectory(
                        self.vfs.to_vpath(&resolved),
                    ))
                }
            } else {
                self.vfs.remove_file(target, &self.cwd)
            };

            if let Err(e) = result {
                if !force {
                    return CommandOutput::error(format!("rm: {}: {}\n", target, e), 1);
                }
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
        let dir = std::env::temp_dir().join(format!("fastshell_rm_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_rm_help() {
        let mut s = mk_shell();
        let out = s.execute("rm", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_rm_help_long() {
        let mut s = mk_shell();
        let out = s.execute("rm", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
