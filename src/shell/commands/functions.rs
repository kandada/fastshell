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
                    // `unset NAME[IDX]` removes one element.
                    if name.ends_with(']') {
                        if let Some(open) = name.find('[') {
                            let base = &name[..open];
                            let idx = &name[open + 1..name.len() - 1];
                            if let Some(m) = self.assoc.get_mut(base) {
                                m.remove(idx);
                            } else if let Some(arr) = self.arrays.get_mut(base) {
                                if let Ok(i) = idx.parse::<usize>() {
                                    if i < arr.len() {
                                        arr[i] = String::new();
                                    }
                                }
                            }
                            i += 1;
                            continue;
                        }
                    }
                    // `unset` of a nonexistent name is a silent no-op in bash
                    // (also drops an indexed array with that name).
                    self.arrays.remove(name);
                    self.vars.remove(name);
                    self.exported.remove(name);
                    self.functions.remove(name);
                }
            }
            i += 1;
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_declare(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            let mut out = String::new();
            let mut keys: Vec<&String> = self.vars.keys().collect();
            keys.sort();
            for k in keys {
                out.push_str(&format!("declare -- {}='{}'\n", k, self.vars[k]));
            }
            return CommandOutput::success(out);
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
                // `declare -p NAME...` prints specific vars; `declare -p` all.
                if args.len() > 1 {
                    let mut out = String::new();
                    for name in &args[1..] {
                        if name.starts_with('-') {
                            continue;
                        }
                        if let Some(v) = self.vars.get(*name) {
                            let attr = if self.readonly.contains(*name) {
                                " -r"
                            } else if self.integer.contains(*name) {
                                " -i"
                            } else {
                                ""
                            };
                            out.push_str(&format!("declare{} {}=\"{}\"\n", attr, name, v));
                        }
                    }
                    return CommandOutput::success(out);
                }
                let mut out = String::new();
                let mut keys: Vec<&String> = self.vars.keys().collect();
                keys.sort();
                for k in keys {
                    let v = &self.vars[k];
                    out.push_str(&format!("declare -- {}='{}'\n", k, v));
                }
                CommandOutput::success(out)
            }
            "-A" => {
                // `declare -A m [m2 ...]` — mark names as associative arrays.
                for a in &args[1..] {
                    if a.starts_with('-') {
                        continue;
                    }
                    let name = a.split('=').next().unwrap_or(a);
                    if !name.is_empty() {
                        self.assoc.entry(name.to_string()).or_default();
                    }
                }
                CommandOutput::success(String::new())
            }
            _ => {
                // Parse attribute flags (`-i -r -x -g -a -A`, combinable).
                let mut fi = false;
                let mut fr = false;
                let mut fx = false;
                let mut fnref = false;
                let mut i = 0;
                while i < args.len() && args[i].starts_with('-') && args[i].len() > 1 {
                    for ch in args[i].trim_start_matches('-').chars() {
                        match ch {
                            'i' => fi = true,
                            'r' => fr = true,
                            'x' => fx = true,
                            'n' => fnref = true,
                            _ => {}
                        }
                    }
                    i += 1;
                }
                for a in &args[i..] {
                    let (name, value) = match a.split_once('=') {
                        Some((n, v)) => (n, Some(v)),
                        None => (*a, None),
                    };
                    if fr {
                        self.readonly.insert(name.to_string());
                    }
                    if fi {
                        self.integer.insert(name.to_string());
                    }
                    if fx {
                        self.exported.insert(name.to_string());
                    }
                    if fnref {
                        if let Some(v) = value {
                            self.namerefs.insert(name.to_string(), v.to_string());
                        } else {
                            self.namerefs
                                .entry(name.to_string())
                                .or_insert_with(String::new);
                        }
                        continue;
                    }
                    if let Some(v) = value {
                        if self.readonly.contains(name) && self.vars.contains_key(name) {
                            return CommandOutput::error(
                                format!("{}: readonly variable\n", name),
                                1,
                            );
                        }
                        self.vars.insert(name.to_string(), v.to_string());
                    } else {
                        self.vars
                            .entry(name.to_string())
                            .or_insert_with(String::new);
                    }
                }
                CommandOutput::success(String::new())
            }
        }
    }

    /// `exec [cmd args]`: with only redirections (stripped by the executor) it
    /// is a no-op; with a command it runs it. fastshell does not model process
    /// replacement.
    pub fn cmd_exec(&mut self, args: &[&str]) -> CommandOutput {
        match args.split_first() {
            None => CommandOutput::success(String::new()),
            Some((cmd, rest)) => self.execute(cmd, rest, None),
        }
    }
}
