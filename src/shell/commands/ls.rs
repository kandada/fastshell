use crate::shell::{Shell, CommandOutput};
use std::path::Path;
use std::time::UNIX_EPOCH;

impl Shell {
    pub fn cmd_ls(&self, args: &[&str]) -> CommandOutput {
        let mut show_all = false;
        let mut long_format = false;
        let mut path = ".";
        let mut human_readable = false;

        for arg in args {
            if arg.starts_with('-') {
                for ch in arg.chars().skip(1) {
                    match ch {
                        'a' => show_all = true,
                        'l' => long_format = true,
                        'h' => human_readable = true,
                        _ => {}
                    }
                }
            } else {
                path = arg;
            }
        }

        let target = match self.vfs.resolve(path, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(e.to_string(), 1),
        };

        if target.is_file() {
            return match self.format_ls_entry(&target, long_format, human_readable) {
                Some(s) => CommandOutput::success(s),
                None => CommandOutput::error(format!("Cannot access {}", target.display()), 1),
            };
        }

        let entries = match self.vfs.list_dir(path, &self.cwd) {
            Ok(e) => e,
            Err(e) => return CommandOutput::error(e.to_string(), 1),
        };

        let mut output = String::new();

        if show_all {
            if long_format {
                for implicit in &[".", ".."] {
                    let full_path = target.join(implicit);
                    if let Some(line) = self.format_ls_entry(&full_path, true, human_readable) {
                        output.push_str(&line);
                    }
                }
            } else {
                output.push_str(".\n..\n");
            }
        }

        for entry in &entries {
            if !show_all && entry.name.starts_with('.') {
                continue;
            }
            if long_format {
                let full_path = target.join(&entry.name);
                if let Some(line) = self.format_ls_entry(&full_path, true, human_readable) {
                    output.push_str(&line);
                }
            } else {
                output.push_str(&entry.name);
                output.push('\n');
            }
        }

        CommandOutput::success(output)
    }

    fn format_ls_entry(&self, path: &Path, long_format: bool, human_readable: bool) -> Option<String> {
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
