// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const MV_HELP_TEXT: &str = "\
Usage: mv [OPTION]... SOURCE... DEST
Rename SOURCE to DEST, or move SOURCE(s) to DIRECTORY.

  -f       do not prompt before overwriting
  -n       do not overwrite an existing file
  -t DIR   move all SOURCEs into DIR
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
        let mut no_clobber = false;
        let mut target_dir: Option<String> = None;
        let mut operands: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg.starts_with("--") {
                match arg {
                    "--force" => force = true,
                    "--verbose" => verbose = true,
                    "--no-clobber" => no_clobber = true,
                    "--target-directory" => {
                        if i + 1 < args.len() {
                            target_dir = Some(args[i + 1].to_string());
                            i += 1;
                        }
                    }
                    _ => crate::warn!("mv: warning: unsupported option '{}'", arg),
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg.chars().skip(1).collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'f' => force = true,
                        'v' => verbose = true,
                        'n' => no_clobber = true,
                        'i' | 'u' => {} // no-op in the sandbox
                        't' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                target_dir = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    target_dir = Some(args[i].to_string());
                                }
                            }
                            j = chars.len();
                            continue;
                        }
                        _ => crate::warn!("mv: warning: unsupported option '-{}'", chars[j]),
                    }
                    j += 1;
                }
            } else {
                operands.push(arg.to_string());
            }
            i += 1;
        }

        if target_dir.is_none() && operands.len() < 2 {
            return CommandOutput::error("mv: missing file operand\n".to_string(), 1);
        }

        let (sources, dest) = match target_dir {
            Some(dir) => {
                if operands.is_empty() {
                    return CommandOutput::error("mv: missing file operand\n".to_string(), 1);
                }
                let srcs = operands.clone();
                (srcs, dir)
            }
            None => {
                let dest = operands.pop().unwrap();
                (operands, dest)
            }
        };

        let mut verbose_out = String::new();
        for src in &sources {
            let src_path = match self.vfs.resolve(src, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("mv: {}: {}\n", src, e), 1),
            };

            let dest_path = if self.vfs.is_dir(&dest, &self.cwd) {
                let fname = src_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| src.to_string());
                format!("{}/{}", dest.trim_end_matches('/'), fname)
            } else {
                dest.clone()
            };

            if no_clobber && self.vfs.exists(&dest_path, &self.cwd) {
                if verbose {
                    verbose_out.push_str(&format!("skipped '{}' -> '{}'\n", src, dest_path));
                }
                continue;
            }

            if verbose {
                verbose_out.push_str(&format!("'{}' -> '{}'\n", src, dest_path));
            }
            if let Err(e) = self.vfs.rename(src, &dest_path, &self.cwd) {
                if !force {
                    return CommandOutput::error(format!("mv: {}\n", e), 1);
                }
            }
        }

        CommandOutput::success(verbose_out)
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
