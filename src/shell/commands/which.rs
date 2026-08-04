// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const WHICH_HELP_TEXT: &str = "\
Usage: which COMMAND...
Locate a command and display its path.

  -a        print all matching executables in PATH
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_which(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(WHICH_HELP_TEXT.to_string());
        }
        if args.is_empty() {
            return CommandOutput::error("which: missing operand\n".to_string(), 1);
        }

        let mut _all_matches = false;
        let mut query_start = 0;
        for (i, arg) in args.iter().enumerate() {
            match *arg {
                "-a" => {
                    _all_matches = true;
                    query_start = i + 1;
                }
                a if a.starts_with('-') => {
                    eprintln!("which: warning: unsupported option '{}'", a);
                    query_start = i + 1;
                }
                _ => break,
            }
        }
        let queries = &args[query_start..];
        if queries.is_empty() {
            return CommandOutput::error("which: missing operand\n".to_string(), 1);
        }

        let known_cmds: &[&str] = &[
            "ls",
            "cd",
            "pwd",
            "mkdir",
            "rm",
            "cp",
            "mv",
            "cat",
            "find",
            "grep",
            "rg",
            "tree",
            "echo",
            "touch",
            "chmod",
            "ps",
            "kill",
            "curl",
            "wget",
            "gzip",
            "gunzip",
            "tar",
            "ping",
            "ssh",
            "git",
            "head",
            "tail",
            "wc",
            "diff",
            "sed",
            "sort",
            "uniq",
            "tee",
            "xargs",
            "which",
            "cut",
            "awk",
            "tr",
            "sleep",
            "date",
            "true",
            "false",
            "test",
            "base64",
            "sha256sum",
            "sha512sum",
            "md5sum",
            "du",
            "df",
            "stat",
            "jq",
            "env",
            "printenv",
            "printf",
            "basename",
            "dirname",
            "realpath",
            "file",
            "column",
            "seq",
            "zip",
            "unzip",
            "shuf",
            "uuidgen",
            "rev",
            "split",
            "comm",
            "xxd",
            "expr",
            "uname",
            "hostname",
            "whoami",
            "id",
            "pgrep",
            "pkill",
            "paste",
            "timeout",
        ];

        let mut output = String::new();
        let mut any_not_found = false;
        for arg in queries {
            if known_cmds.contains(arg) {
                output.push_str(&format!("{}: built-in fastshell command\n", arg));
            } else {
                let result = std::process::Command::new("which")
                    .arg(arg)
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                if result.is_empty() {
                    output.push_str(&format!("{} not found\n", arg));
                    any_not_found = true;
                } else {
                    output.push_str(&result);
                }
            }
        }

        if any_not_found {
            CommandOutput::error(output, 1)
        } else {
            CommandOutput::success(output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fastshell_which_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_which_help() {
        let mut s = mk_shell();
        let out = s.execute("which", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_which_help_long() {
        let mut s = mk_shell();
        let out = s.execute("which", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_which_unknown_exits_nonzero() {
        let mut s = mk_shell();
        let out = s.execute("which", &["nonexistent_cmd_xyz123"], None);
        assert_eq!(out.exit_code, 1);
    }
}
