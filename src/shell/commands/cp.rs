// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const CP_HELP_TEXT: &str = "\
Usage: cp [OPTION]... SOURCE DEST
Copy SOURCE to DEST, or multiple SOURCE(s) to DIRECTORY.

  -r, -R  copy directories recursively
  -f       if an existing destination file cannot be opened, remove it
  -v       explain what is being done
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_cp(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(CP_HELP_TEXT.to_string());
        }
        let mut recursive = false;
        let mut force = false;
        let mut verbose = false;
        let mut operands = Vec::new();

        for arg in args {
            match *arg {
                "-r" | "-R" => recursive = true,
                "-f" => force = true,
                "-v" => verbose = true,
                _ if arg.starts_with('-') => {
                    eprintln!("cp: warning: unsupported option '{}'", arg);
                }
                _ => operands.push(arg.to_string()),
            }
        }

        if operands.len() < 2 {
            return CommandOutput::error("cp: missing file operand\n".to_string(), 1);
        }

        let dest = operands.pop().unwrap();
        let sources = operands;

        for src in &sources {
            let src_path = match self.vfs.resolve(src, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("cp: {}: {}\n", src, e), 1),
            };

            if src_path.is_dir() && !recursive {
                return CommandOutput::error(
                    format!("cp: {} is a directory (not copied)\n", src),
                    1,
                );
            }

            let dest_path = if self.vfs.is_dir(&dest, &self.cwd) {
                let fname = src_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| src.clone());
                format!("{}/{}", dest.trim_end_matches('/'), fname)
            } else {
                dest.clone()
            };

            if verbose {
                eprintln!("{} -> {}", src, &dest_path);
            }
            if let Err(e) = self.vfs.copy(src, &dest_path, &self.cwd) {
                if !force {
                    return CommandOutput::error(format!("cp: {}\n", e), 1);
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
        let dir = std::env::temp_dir().join(format!("fastshell_cp_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_cp_help() {
        let mut s = mk_shell();
        let out = s.execute("cp", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_cp_help_long() {
        let mut s = mk_shell();
        let out = s.execute("cp", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
