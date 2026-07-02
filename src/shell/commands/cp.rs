// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_cp(&self, args: &[&str]) -> CommandOutput {
        let mut recursive = false;
        let mut operands = Vec::new();

        for arg in args {
            match *arg {
                "-r" | "-R" => recursive = true,
                _ if arg.starts_with('-') => {}
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

            if let Err(e) = self.vfs.copy(src, &dest_path, &self.cwd) {
                return CommandOutput::error(format!("cp: {}\n", e), 1);
            }
        }

        CommandOutput::success(String::new())
    }
}
