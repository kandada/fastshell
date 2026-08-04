// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_eval(&self, args: &[&str]) -> CommandOutput {
        let script = args.join(" ");
        CommandOutput::success(script)
    }
}
