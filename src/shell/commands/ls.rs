// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::path::Path;
use std::time::UNIX_EPOCH;

const LS_HELP_TEXT: &str = "\
Usage: ls [OPTION]... [FILE]...
List information about the FILEs (the current directory by default).

  -l  use a long listing format
  -a  do not ignore entries starting with .
  -h  with -l, print sizes in human readable format
  -R  list subdirectories recursively
  -t  sort by modification time, newest first
  -S  sort by file size, largest first
  -r  reverse order while sorting
  -1  list one file per line
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_ls(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(LS_HELP_TEXT.to_string());
        }
        let mut show_all = false;
        let mut long_format = false;
        let mut human_readable = false;
        let mut recursive = false;
        let mut sort_time = false;
        let mut sort_size = false;
        let mut reverse = false;
        let mut single_column = false;
        let mut paths: Vec<&str> = Vec::new();

        for arg in args {
            if arg.starts_with('-') {
                for ch in arg.chars().skip(1) {
                    match ch {
                        'a' => show_all = true,
                        'l' => long_format = true,
                        'h' => human_readable = true,
                        'R' => recursive = true,
                        't' => sort_time = true,
                        'S' => sort_size = true,
                        'r' => reverse = true,
                        '1' => single_column = true,
                        _ => eprintln!("ls: warning: unsupported option '-{}'", ch),
                    }
                }
            } else {
                paths.push(arg);
            }
        }

        if paths.is_empty() {
            paths.push(".");
        }

        let mut output = String::new();

        // Like real ls: file operands are listed plainly first; directory
        // operands get "name:" headers only when there are multiple operands.
        let mut file_paths: Vec<&str> = Vec::new();
        let mut dir_paths: Vec<&str> = Vec::new();
        for path in &paths {
            match self.vfs.resolve(path, &self.cwd) {
                Ok(p) if p.is_file() => file_paths.push(path),
                Ok(_) => dir_paths.push(path),
                Err(e) => return CommandOutput::error(e.to_string(), 1),
            }
        }
        let multi = paths.len() > 1;
        let use_long = long_format && !single_column;

        for path in &file_paths {
            if let Ok(target) = self.vfs.resolve(path, &self.cwd) {
                if use_long {
                    if let Some(s) = self.format_ls_entry(&target, true, human_readable) {
                        output.push_str(&s);
                    }
                } else {
                    output.push_str(path);
                    output.push('\n');
                }
            }
        }

        for (pi, path) in dir_paths.iter().enumerate() {
            if multi {
                if pi > 0 || !file_paths.is_empty() {
                    output.push('\n');
                }
                output.push_str(&format!("{}:\n", path));
            }

            let target = match self.vfs.resolve(path, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(e.to_string(), 1),
            };

            self.ls_list_dir(
                path, &target, show_all, use_long, human_readable,
                recursive, sort_time, sort_size, reverse, &mut output,
            );
        }

        CommandOutput::success(output)
    }

    fn ls_list_dir(
        &self,
        vpath: &str,
        real_path: &Path,
        show_all: bool,
        long_format: bool,
        human_readable: bool,
        recursive: bool,
        sort_time: bool,
        sort_size: bool,
        reverse: bool,
        output: &mut String,
    ) {
        let mut entries = match self.vfs.list_dir(vpath, &self.cwd) {
            Ok(e) => e,
            Err(e) => {
                output.push_str(&format!("ls: {}: {}\n", vpath, e));
                return;
            }
        };

        // Sort entries
        if sort_time {
            entries.sort_by_key(|e| {
                let full = real_path.join(&e.name);
                full.metadata().ok().and_then(|m| m.modified().ok())
                    .map(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()).unwrap_or(0))
                    .unwrap_or(0)
            });
        } else if sort_size {
            entries.sort_by_key(|e| e.size);
        } else {
            entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }

        if reverse {
            entries.reverse();
        }

        let mut dirs: Vec<String> = Vec::new();

        if show_all && !long_format {
            output.push_str(".\n..\n");
        } else if show_all && long_format {
            for implicit in &[".", ".."] {
                let full = real_path.join(implicit);
                if let Some(line) = self.format_ls_entry(&full, true, human_readable) {
                    output.push_str(&line);
                }
            }
        }

        for entry in &entries {
            if !show_all && entry.name.starts_with('.') {
                continue;
            }
            if long_format {
                let full = real_path.join(&entry.name);
                if let Some(line) = self.format_ls_entry(&full, true, human_readable) {
                    output.push_str(&line);
                }
            } else {
                output.push_str(&entry.name);
                output.push('\n');
            }

            if entry.is_dir && recursive {
                let sub_vpath = if vpath.ends_with('/') {
                    format!("{}{}", vpath, entry.name)
                } else {
                    format!("{}/{}", vpath, entry.name)
                };
                dirs.push(sub_vpath);
            }
        }

        // Recurse into subdirectories
        for sub in &dirs {
            output.push_str(&format!("\n{}:\n", sub));
            if let Ok(sub_real) = self.vfs.resolve(sub, &self.cwd) {
                self.ls_list_dir(
                    sub, &sub_real, show_all, long_format, human_readable,
                    recursive, sort_time, sort_size, reverse, output,
                );
            }
        }
    }

    fn format_ls_entry(
        &self,
        path: &Path,
        long_format: bool,
        human_readable: bool,
    ) -> Option<String> {
        let metadata = match path.symlink_metadata() {
            Ok(m) => m,
            Err(_) => return None,
        };

        let name = path.file_name()?.to_string_lossy().to_string();
        if !long_format {
            return Some(format!("{}\n", name));
        }

        let file_type = if metadata.is_dir() {
            'd'
        } else if metadata.is_symlink() {
            'l'
        } else {
            '-'
        };

        let mode = crate::shell::mode_string(&metadata);
        let size = if human_readable {
            crate::shell::human_size(metadata.len())
        } else {
            metadata.len().to_string()
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| {
                let d = t.duration_since(UNIX_EPOCH).ok()?;
                Some(crate::shell::format_unix_time(d.as_secs()))
            })
            .unwrap_or_else(|| "?".to_string());

        Some(format!(
            "{}{} {:>8} {} {}\n",
            file_type, mode, size, modified, name
        ))
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
        let dir = std::env::temp_dir().join(format!("fastshell_ls_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_ls_help() {
        let mut s = mk_shell();
        let out = s.execute("ls", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_ls_help_long() {
        let mut s = mk_shell();
        let out = s.execute("ls", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_ls_1_flag() {
        let mut s = mk_shell();
        s.vfs.write("a.txt", "", "hello").unwrap();
        s.vfs.write("b.txt", "", "world").unwrap();
        let out = s.execute("ls", &["-1"], None);
        assert_eq!(out.exit_code, 0);
        let lines: Vec<&str> = out.stdout.trim().lines().collect();
        assert_eq!(lines.len(), 2);
    }
}