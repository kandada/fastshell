// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_read(&mut self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut prompt: Option<String> = None;
        let mut _silent = false;
        let mut _timeout: Option<u32> = None;
        let mut _delim: Option<char> = None;
        let mut array_target: Option<String> = None;
        let mut i = 0;

        while i < args.len() {
            match args[i] {
                "-a" => {
                    i += 1;
                    if i < args.len() {
                        array_target = Some(args[i].to_string());
                    } else {
                        return CommandOutput::error(
                            "read: -a requires an array name\n".to_string(),
                            1,
                        );
                    }
                }
                "-p" => {
                    i += 1;
                    if i < args.len() {
                        prompt = Some(args[i].to_string());
                    } else {
                        return CommandOutput::error(
                            "read: -p requires an argument\n".to_string(),
                            1,
                        );
                    }
                }
                "-s" => {
                    _silent = true;
                }
                "-r" => {
                    // Raw mode: our reader never interprets backslashes, so
                    // this is effectively the default behaviour.
                }
                "-t" => {
                    i += 1;
                    if i < args.len() {
                        _timeout = args[i].parse().ok();
                    } else {
                        return CommandOutput::error(
                            "read: -t requires an argument\n".to_string(),
                            1,
                        );
                    }
                }
                "-d" => {
                    i += 1;
                    if i < args.len() {
                        _delim = args[i].chars().next();
                    } else {
                        return CommandOutput::error(
                            "read: -d requires an argument\n".to_string(),
                            1,
                        );
                    }
                }
                _ => break,
            }
            i += 1;
        }

        let var_args = &args[i..];
        if var_args.is_empty() && array_target.is_none() {
            return CommandOutput::error(
                "read: usage: read [-p prompt] [-s] [-t timeout] [-d delim] name...\n".to_string(),
                1,
            );
        }

        let mut stderr_out = String::new();
        if let Some(p) = prompt {
            stderr_out.push_str(&p);
        }

        // `read` consumes exactly ONE line (bash semantics); the remainder is
        // left for the next `read`. Using the whole stdin made `read x < file`
        // join every line into one value ("L1 L2").
        let input = stdin.unwrap_or("");
        let had_input = !input.is_empty();
        let line = input.split('\n').next().unwrap_or("");
        let line = line.trim_end_matches('\r');

        // Split on the shell's IFS (default: whitespace). `IFS=, read a b`
        // splits on commas.
        let ifs = self
            .vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());
        let default_ifs = ifs == " \t\n" || ifs == " \t\n\r" || ifs == " \t";
        let parts: Vec<&str> = if default_ifs || ifs.is_empty() {
            line.split_whitespace().collect()
        } else {
            let seps: Vec<char> = ifs.chars().collect();
            line.split(|c| seps.contains(&c))
                .filter(|s| !s.is_empty())
                .collect()
        };
        let n_vars = var_args.len();

        if let Some(name) = array_target {
            self.arrays
                .insert(name, parts.iter().map(|s| s.to_string()).collect());
            return CommandOutput {
                stdout: String::new(),
                stderr: stderr_out,
                exit_code: if had_input { 0 } else { 1 },
            };
        }

        for j in 0..n_vars {
            let val = if j == n_vars - 1 && parts.len() > j {
                parts[j..].join(" ")
            } else if j < parts.len() {
                parts[j].to_string()
            } else {
                String::new()
            };
            self.vars.insert(var_args[j].to_string(), val);
        }

        CommandOutput {
            stdout: String::new(),
            stderr: stderr_out,
            // bash: `read` returns 1 at EOF (no input) so `while read` ends.
            exit_code: if had_input { 0 } else { 1 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_read_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        Shell::new(setup_vfs())
    }

    #[test]
    fn test_read_single_var() {
        let mut shell = mk_shell();
        let out = shell.cmd_read(&["foo"], Some("hello"));
        assert_eq!(out.exit_code, 0);
        assert_eq!(shell.vars.get("foo").map(|s| s.as_str()), Some("hello"));
    }

    #[test]
    fn test_read_multi_var() {
        let mut shell = mk_shell();
        let out = shell.cmd_read(&["a", "b"], Some("hello world"));
        assert_eq!(out.exit_code, 0);
        assert_eq!(shell.vars.get("a").map(|s| s.as_str()), Some("hello"));
        assert_eq!(shell.vars.get("b").map(|s| s.as_str()), Some("world"));
    }

    #[test]
    fn test_read_last_var_gets_rest() {
        let mut shell = mk_shell();
        let out = shell.cmd_read(&["a", "b"], Some("one two three four"));
        assert_eq!(out.exit_code, 0);
        assert_eq!(shell.vars.get("a").map(|s| s.as_str()), Some("one"));
        assert_eq!(
            shell.vars.get("b").map(|s| s.as_str()),
            Some("two three four")
        );
    }

    #[test]
    fn test_read_more_vars_than_parts() {
        let mut shell = mk_shell();
        let out = shell.cmd_read(&["a", "b", "c"], Some("x"));
        assert_eq!(out.exit_code, 0);
        assert_eq!(shell.vars.get("a").map(|s| s.as_str()), Some("x"));
        assert_eq!(shell.vars.get("b").map(|s| s.as_str()), Some(""));
        assert_eq!(shell.vars.get("c").map(|s| s.as_str()), Some(""));
    }

    #[test]
    fn test_read_no_args() {
        let mut shell = mk_shell();
        let out = shell.cmd_read(&[], None);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_read_no_stdin() {
        let mut shell = mk_shell();
        let out = shell.cmd_read(&["foo"], None);
        // bash returns 1 at EOF (no input line).
        assert_ne!(out.exit_code, 0);
        assert_eq!(shell.vars.get("foo").map(|s| s.as_str()), Some(""));
    }
}
