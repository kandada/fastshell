// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_rm(&self, args: &[&str]) -> CommandOutput {
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
                _ if arg.starts_with('-') => {}
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
