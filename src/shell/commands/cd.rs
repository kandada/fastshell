// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_cd(&mut self, args: &[&str]) -> CommandOutput {
        let target = if args.is_empty() {
            "/".to_string()
        } else if args[0] == "-" {
            if self.prev_dir.is_empty() {
                return CommandOutput::error("cd: OLDPWD not set\n".to_string(), 1);
            }
            self.prev_dir.clone()
        } else {
            args[0].to_string()
        };

        let resolved = match self.vfs.resolve(&target, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(e.to_string(), 1),
        };

        if !resolved.is_dir() {
            return CommandOutput::error(format!("cd: {}: Not a directory", target), 1);
        }

        let new_cwd = self.vfs.to_vpath(&resolved);
        if new_cwd != self.cwd {
            self.prev_dir = self.cwd.clone();
            self.cwd = new_cwd;
        }

        if args.first() == Some(&"-") {
            CommandOutput::success(self.cwd.clone() + "\n")
        } else {
            CommandOutput::success(String::new())
        }
    }
}
