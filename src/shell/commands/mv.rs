// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const MV_HELP_TEXT: &str = "\
Usage: mv SOURCE... DEST
Rename SOURCE to DEST, or move SOURCE(s) to DIRECTORY.

  -f       do not prompt before overwriting
  -v       explain what is being done
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_mv(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(MV_HELP_TEXT.to_string());
        }
        let mut force = false;
        let mut verbose = false;
        let mut operands: Vec<&str> = Vec::new();

        for arg in args {
            match *arg {
                "-f" => force = true,
                "-v" => verbose = true,
                a if a.starts_with('-') => {
                    eprintln!("mv: warning: unsupported option '{}'", a);
                }
                _ => operands.push(arg),
            }
        }

        if operands.len() < 2 {
            return CommandOutput::error("mv: missing file operand\n".to_string(), 1);
        }

        let dest = operands.last().unwrap();
        let sources = &operands[..operands.len() - 1];

        for src in sources {
            let src_path = match self.vfs.resolve(src, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("mv: {}: {}\n", src, e), 1),
            };

            let dest_path = if self.vfs.is_dir(dest, &self.cwd) {
                let fname = src_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| src.to_string());
                format!("{}/{}", dest.trim_end_matches('/'), fname)
            } else {
                dest.to_string()
            };

            if verbose {
                eprintln!("renamed {} -> {}", src, &dest_path);
            }
            if let Err(e) = self.vfs.rename(src, &dest_path, &self.cwd) {
                if !force {
                    return CommandOutput::error(format!("mv: {}\n", e), 1);
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
        let dir = std::env::temp_dir().join(format!("fastshell_mv_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_mv_help() {
        let mut s = mk_shell();
        let out = s.execute("mv", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_mv_help_long() {
        let mut s = mk_shell();
        let out = s.execute("mv", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
