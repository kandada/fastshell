// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_alias(&mut self, args: &[&str]) -> CommandOutput {
        let raw = args.join(" ");
        if raw.is_empty() {
            let mut keys: Vec<&String> = self.aliases.keys().collect();
            keys.sort();
            let mut out = String::new();
            for k in keys {
                let v = &self.aliases[k];
                out.push_str(&format!("alias {}='{}'\n", k, v));
            }
            return CommandOutput::success(out);
        }

        if let Some(eq) = raw.find('=') {
            let name = raw[..eq].trim().to_string();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return CommandOutput::error("alias: invalid name\n".to_string(), 1);
            }
            let value = raw[eq + 1..].trim();
            let value = if value.starts_with('\'') && value.ends_with('\'') {
                value[1..value.len() - 1].to_string()
            } else if value.starts_with('"') && value.ends_with('"') {
                value[1..value.len() - 1].to_string()
            } else {
                value.to_string()
            };
            self.aliases.insert(name, value);
            CommandOutput::success(String::new())
        } else {
            let name = raw.trim();
            match self.aliases.get(name) {
                Some(v) => CommandOutput::success(format!("alias {}='{}'\n", name, v)),
                None => CommandOutput::error(format!("alias: {}: not found\n", name), 1),
            }
        }
    }

    pub fn cmd_unalias(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("unalias: missing argument\n".to_string(), 1);
        }
        if args[0] == "-a" {
            self.aliases.clear();
            return CommandOutput::success(String::new());
        }
        for name in args {
            if self.aliases.remove(*name).is_none() {
                return CommandOutput::error(
                    format!("unalias: {}: not found\n", name),
                    1,
                );
            }
        }
        CommandOutput::success(String::new())
    }

    /// Resolve alias for a command. Returns (expanded_cmd, expanded_args, was_resolved).
    /// Handles recursive alias expansion with cycle detection.
    pub fn resolve_alias(&self, command: &str, args: &[&str]) -> Option<(String, Vec<String>)> {
        let alias_key = command.to_string();
        let value = self.aliases.get(&alias_key)?;
        let expanded = value.clone();

        // Tokenize the alias value
        let parts: Vec<&str> = expanded.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let new_cmd = parts[0].to_string();
        let mut new_args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
        for a in args {
            new_args.push(a.to_string());
        }

        // Recursive alias resolution (with depth limit)
        let mut resolved_cmd = new_cmd;
        let mut resolved_args = new_args;
        let mut seen = std::collections::HashSet::new();
        seen.insert(alias_key);
        const MAX_DEPTH: usize = 10;

        for _ in 0..MAX_DEPTH {
            if let Some(v) = self.aliases.get(&resolved_cmd) {
                if seen.contains(&resolved_cmd) {
                    break; // cycle detected, stop expanding
                }
                seen.insert(resolved_cmd.clone());
                let p: Vec<&str> = v.split_whitespace().collect();
                if p.is_empty() {
                    break;
                }
                resolved_cmd = p[0].to_string();
                let mut prefix_args: Vec<String> = p[1..].iter().map(|s| s.to_string()).collect();
                prefix_args.extend(resolved_args);
                resolved_args = prefix_args;

                // Trailing space on alias value triggers further alias expansion on next word
            } else {
                break;
            }
        }

        // Trailing-space-trigger: if alias value ends with space, expand first arg as alias too
        if value.ends_with(' ') && !resolved_args.is_empty() {
            let first_arg = &resolved_args[0];
            if let Some(v2) = self.aliases.get(first_arg) {
                let p2: Vec<&str> = v2.split_whitespace().collect();
                if !p2.is_empty() && !seen.contains(first_arg) {
                    let mut new_args: Vec<String> = p2.iter().map(|s| s.to_string()).collect();
                    new_args.extend(resolved_args[1..].iter().cloned());
                    resolved_args = new_args;
                }
            }
        }

        Some((resolved_cmd, resolved_args))
    }
}
