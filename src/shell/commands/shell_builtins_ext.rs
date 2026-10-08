// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Shell builtins filling backbone gaps: `pushd`/`popd`/`dirs`, `umask`,
//! `ulimit`, `builtin`, `hash`, `shopt`, `readonly`, `trap`.

use crate::shell::{CommandOutput, Shell};

const SHOPT_NAMES: &[&str] = &[
    "nullglob",
    "failglob",
    "nocaseglob",
    "dotglob",
    "extglob",
    "globstar",
    "noglob",
    "expand_aliases",
];

impl Shell {
    fn print_dirs(&self) -> String {
        let mut parts = vec![self.cwd.clone()];
        for d in self.dir_stack.iter().rev() {
            parts.push(d.clone());
        }
        parts.join(" ")
    }

    /// `pushd [DIR]` — push cwd and cd to DIR (or swap the top two entries).
    pub fn cmd_pushd(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            if self.dir_stack.is_empty() {
                return CommandOutput::error("pushd: no other directory\n".to_string(), 1);
            }
            let target = self.dir_stack.pop().unwrap();
            let cur = self.cwd.clone();
            let out = self.cmd_cd(&[target.as_str()]);
            if out.exit_code != 0 {
                self.dir_stack.push(cur);
                return out;
            }
            self.dir_stack.push(cur);
            return CommandOutput::success(self.print_dirs() + "\n");
        }
        let target = args[0].to_string();
        let cur = self.cwd.clone();
        let out = self.cmd_cd(&[target.as_str()]);
        if out.exit_code != 0 {
            return out;
        }
        self.dir_stack.push(cur);
        CommandOutput::success(self.print_dirs() + "\n")
    }

    /// `popd` — cd to the most recently pushed directory.
    pub fn cmd_popd(&mut self, _args: &[&str]) -> CommandOutput {
        match self.dir_stack.pop() {
            Some(target) => {
                let out = self.cmd_cd(&[target.as_str()]);
                if out.exit_code != 0 {
                    return out;
                }
                CommandOutput::success(self.print_dirs() + "\n")
            }
            None => CommandOutput::error("popd: directory stack empty\n".to_string(), 1),
        }
    }

    /// `dirs` — print the directory stack (cwd first).
    pub fn cmd_dirs(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success(self.print_dirs() + "\n")
    }

    /// `umask [MASK]` — get/set the file-creation mask.
    pub fn cmd_umask(&mut self, args: &[&str]) -> CommandOutput {
        if let Some(a) = args.first() {
            let a = a.trim_start_matches('0');
            let val = u32::from_str_radix(if a.is_empty() { "0" } else { a }, 8);
            match val {
                Ok(v) => {
                    self.umask = v & 0o777;
                    CommandOutput::success(String::new())
                }
                Err(_) => CommandOutput::error(format!("umask: invalid mask '{}'\n", args[0]), 1),
            }
        } else {
            CommandOutput::success(format!("{:04o}\n", self.umask))
        }
    }

    /// `ulimit [-n] [-a]` — report resource limits (fixed, sandbox-appropriate).
    pub fn cmd_ulimit(&self, args: &[&str]) -> CommandOutput {
        let show_all = args.iter().any(|a| *a == "-a");
        if show_all {
            let out = "core file size          (blocks, -c) 0\n\
                       data seg size           (kbytes, -d) unlimited\n\
                       open files                      (-n) 1024\n\
                       max user processes              (-u) 512\n\
                       virtual memory          (kbytes, -v) unlimited\n";
            return CommandOutput::success(out.to_string());
        }
        // `-n` and the default both report open files.
        CommandOutput::success("1024\n".to_string())
    }

    /// `builtin CMD [ARGS...]` — run CMD as a shell builtin (bypassing functions).
    pub fn cmd_builtin(&mut self, args: &[&str]) -> CommandOutput {
        match args.split_first() {
            None => CommandOutput::success(String::new()),
            Some((cmd, rest)) => self.execute(cmd, rest, None),
        }
    }

    /// `hash [...]` — no-op cache control (fastshell resolves commands each time).
    pub fn cmd_hash(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success(String::new())
    }

    /// `shopt [-s|-u|-q] [NAME...]` — toggle shell options.
    pub fn cmd_shopt(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            let mut out = String::new();
            for name in SHOPT_NAMES {
                let on = *self.shopt.get(*name).unwrap_or(&false);
                out.push_str(&format!("{}\t{}\n", name, if on { "on" } else { "off" }));
            }
            return CommandOutput::success(out);
        }
        let mode = args[0];
        if mode == "-s" || mode == "-u" {
            let val = mode == "-s";
            if args.len() == 1 {
                // apply to all? bash only with names; be lenient.
                return CommandOutput::success(String::new());
            }
            for name in &args[1..] {
                if !SHOPT_NAMES.contains(name) {
                    return CommandOutput::error(
                        format!("shopt: {}: invalid shell option name\n", name),
                        1,
                    );
                }
                self.shopt.insert((*name).to_string(), val);
            }
            return CommandOutput::success(String::new());
        }
        if mode == "-q" {
            let all_on = args[1..]
                .iter()
                .all(|n| *self.shopt.get(*n).unwrap_or(&false));
            return CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: if all_on { 0 } else { 1 },
            };
        }
        // Unknown: print current values.
        CommandOutput::success(String::new())
    }

    /// `readonly [-p] [NAME[=VALUE]...]` — mark variables read-only.
    pub fn cmd_readonly(&mut self, args: &[&str]) -> CommandOutput {
        let names: Vec<&str> = args
            .iter()
            .copied()
            .filter(|a| !a.starts_with('-'))
            .collect();
        if names.is_empty() {
            let mut out = String::new();
            let mut keys: Vec<&String> = self.readonly.iter().collect();
            keys.sort();
            for k in keys {
                let v = self.vars.get(k).cloned().unwrap_or_default();
                out.push_str(&format!("declare -r {}=\"{}\"\n", k, v));
            }
            return CommandOutput::success(out);
        }
        for a in names {
            let (name, val) = match a.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (a, None),
            };
            if let Some(v) = val {
                self.vars.insert(name.to_string(), v);
            }
            self.readonly.insert(name.to_string());
        }
        CommandOutput::success(String::new())
    }

    /// `jobs` — list recorded background jobs.
    pub fn cmd_jobs(&self, _args: &[&str]) -> CommandOutput {
        let mut out = String::new();
        for (i, j) in self.jobs.iter().enumerate() {
            out.push_str(&format!("[{}] {} {}\n", i + 1, j.status, j.cmd));
        }
        CommandOutput::success(out)
    }

    /// `disown [%n]` — forget background jobs (they run synchronously here).
    pub fn cmd_disown(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            self.jobs.clear();
        } else {
            for a in args {
                if let Some(key) = a.strip_prefix('%') {
                    if let Ok(n) = key.parse::<usize>() {
                        if n >= 1 && n <= self.jobs.len() {
                            self.jobs.remove(n - 1);
                        }
                    }
                }
            }
        }
        CommandOutput::success(String::new())
    }

    /// `fg [%n]` / `bg [%n]` — no real job control (jobs run synchronously).
    pub fn cmd_fg(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::error("fg: no job control\n".to_string(), 1)
    }

    pub fn cmd_bg(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::error("bg: no job control\n".to_string(), 1)
    }

    /// `wait [PID|%n]` — background jobs run synchronously, so this is a no-op.
    pub fn cmd_wait(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success(String::new())
    }

    /// `trap ['CMD'] SIGNAL...` / `trap - SIGNAL` / `trap` (list).
    pub fn cmd_trap(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            let mut out = String::new();
            let mut keys: Vec<&String> = self.traps.keys().collect();
            keys.sort();
            for k in keys {
                out.push_str(&format!("trap -- '{}' {}\n", self.traps[k], k));
            }
            return CommandOutput::success(out);
        }
        if args[0] == "-l" {
            return CommandOutput::success(
                "HUP INT QUIT ILL TRAP ABRT BUS FPE KILL USR1 SEGV USR2 PIPE ALRM TERM CHLD CONT STOP TSTP TTIN TTOU URG XCPU XFSZ VTALRM PROF WINCH IO PWR SYS EXIT ERR\n".to_string(),
            );
        }
        let normalize = |s: &str| -> String {
            let up = s.to_ascii_uppercase();
            up.strip_prefix("SIG").unwrap_or(&up).to_string()
        };
        // Clear form: `trap - SIG...` or `trap '' SIG...`
        if args[0] == "-" || args[0].is_empty() {
            for s in &args[1..] {
                self.traps.remove(&normalize(s));
            }
            return CommandOutput::success(String::new());
        }
        let handler = args[0].to_string();
        if args.len() < 2 {
            // `trap 'CMD'` with no signal is a usage error in bash.
            return CommandOutput::error(
                "trap: usage: trap [-lp] [[arg] signal_spec ...]\n".to_string(),
                2,
            );
        }
        for s in &args[1..] {
            self.traps.insert(normalize(s), handler.clone());
        }
        CommandOutput::success(String::new())
    }
}
