// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_export(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            // export -p: list exported variables
            let mut out = String::new();
            for (k, v) in &self.vars {
                if self.exported.contains(k) {
                    out.push_str(&format!("export {}='{}'\n", k, v));
                }
            }
            return CommandOutput::success(out);
        }

        if args[0] == "-p" {
            let mut out = String::new();
            for (k, v) in &self.vars {
                if self.exported.contains(k) {
                    out.push_str(&format!("export {}='{}'\n", k, v));
                }
            }
            return CommandOutput::success(out);
        }

        if args[0] == "-n" {
            // export -n KEY: remove export attribute
            for &name in &args[1..] {
                self.exported.remove(name);
            }
            return CommandOutput::success(String::new());
        }

        // export KEY (no =) — mark existing var as exported
        // export KEY=val — handled by consume_assignments, but called here too
        for &arg in args {
            if let Some(eq) = arg.find('=') {
                let name = &arg[..eq];
                let value = &arg[eq + 1..];
                self.vars.insert(name.to_string(), value.to_string());
                self.exported.insert(name.to_string());
            } else {
                // Mark existing variable as exported
                if self.vars.contains_key(arg) {
                    self.exported.insert(arg.to_string());
                } else {
                    // Variable not set yet, mark as exported with empty value
                    self.exported.insert(arg.to_string());
                }
            }
        }
        CommandOutput::success(String::new())
    }
}
