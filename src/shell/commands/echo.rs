// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_echo(&self, args: &[&str]) -> CommandOutput {
        let mut no_newline = false;
        let mut start = 0;

        while start < args.len() {
            if args[start] == "-n" {
                no_newline = true;
                start += 1;
            } else {
                break;
            }
        }

        let output = args[start..].join(" ");
        if no_newline {
            CommandOutput::success(output)
        } else {
            CommandOutput::success(output + "\n")
        }
    }
}
