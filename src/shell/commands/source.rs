// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_source(&self, args: &[&str]) -> CommandOutput {
        let file = match args.first() {
            Some(f) => *f,
            None => return CommandOutput::error("source: missing file operand\n".to_string(), 1),
        };
        let _resolved = match self.vfs.resolve(file, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("source: {}: {}\n", file, e), 1),
        };
        let content = match self.vfs.read_to_string(file, &self.cwd) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("source: {}: {}\n", file, e), 1),
        };
        CommandOutput::success(content)
    }
}
