// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_unset(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("unset: missing argument\n".to_string(), 1);
        }
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-f" => {
                    i += 1;
                    if i < args.len() {
                        let name = args[i];
                        if self.functions.remove(name).is_none() {
                            return CommandOutput::error(
                                format!("unset: {}: not a function\n", name),
                                1,
                            );
                        }
                    } else {
                        return CommandOutput::error(
                            "unset: -f requires a function name\n".to_string(),
                            1,
                        );
                    }
                }
                "-v" => {
                    i += 1;
                    if i < args.len() {
                        let name = args[i];
                        self.vars.remove(name);
                        self.exported.remove(name);
                    } else {
                        return CommandOutput::error(
                            "unset: -v requires a variable name\n".to_string(),
                            1,
                        );
                    }
                }
                name => {
                    if self.vars.remove(name).is_some() {
                        self.exported.remove(name);
                    } else if self.functions.remove(name).is_some() {
                    } else {
                        return CommandOutput::error(
                            format!("unset: {}: not found\n", name),
                            1,
                        );
                    }
                }
            }
            i += 1;
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_declare(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::success(String::new());
        }
        match args[0] {
            "-f" => {
                let mut out = String::new();
                let mut keys: Vec<&String> = self.functions.keys().collect();
                keys.sort();
                for k in keys {
                    let v = &self.functions[k];
                    out.push_str(&format!("{}() {{\n    {}\n}}\n", k, v));
                }
                CommandOutput::success(out)
            }
            "-p" => {
                let mut out = String::new();
                let mut keys: Vec<&String> = self.vars.keys().collect();
                keys.sort();
                for k in keys {
                    let v = &self.vars[k];
                    out.push_str(&format!("declare -- {}='{}'\n", k, v));
                }
                CommandOutput::success(out)
            }
            _ => CommandOutput::error(
                format!("declare: unknown option {}\n", args[0]),
                1,
            ),
        }
    }
}
