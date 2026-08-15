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
    /// The list of built-in fastshell commands, shared by `which` and
    /// `command -v`.
    pub fn known_builtin_commands() -> &'static [&'static str] {
        &[
            "ls", "cd", "pwd", "mkdir", "rm", "cp", "mv", "cat", "find", "grep", "rg", "tree",
            "echo", "touch", "chmod", "ps", "kill", "curl", "wget", "gzip", "gunzip", "tar",
            "ping", "ssh", "git", "head", "tail", "wc", "diff", "sed", "sort", "uniq", "tee",
            "xargs", "which", "command", "cut", "awk", "tr", "sleep", "date", "true", "false",
            "test", "base64", "sha256sum", "sha512sum", "md5sum", "du", "df", "stat", "jq",
            "env", "printenv", "printf", "basename", "dirname", "realpath", "file",
            "pdftotext", "pip-install", "pip", "doctotext", "epubtext", "column", "seq",
            "zip", "unzip", "shuf", "uuidgen", "rev", "split", "comm", "xxd", "expr", "uname",
            "hostname", "whoami", "id", "pgrep", "pkill", "paste", "timeout", "ln", "readlink",
            "rmdir", "mktemp", "tac", "nl", "truncate", "cmp", "strings", "fold", "expand",
            "unexpand", "yes", "sha1sum", "sum", "pidof", "nproc", "tty", "clear", "sync",
            "nice", "chown", "chgrp", "groups", "dd", "od", "uptime", "free", "nslookup",
            "bzip2", "bunzip2", "xz", "unxz", "zcat", "dos2unix", "unix2dos", "cal", "logger",
            "dmesg", "pstree", "killall", "watch", "logname", "who", "reset", "hexdump",
            "sha3sum", "tsort", "renice", "nohup", "chroot", "mkfifo", "install", "shred",
            "fallocate", "telnet", "traceroute", "ifconfig", "netstat", "nc", "patch", "mknod",
            "mount", "umount", "whois", "hostid", "bc", "iostat", "vmstat", "lsblk", "lsof",
            "dig", "rsync", "hdparm", "smartctl", "blkid", "lsusb", "ss", "ip", "ethtool",
            "service", "showmount", "sqlite3", "python", "python3", "export", "unset",
            "declare", "set", "source", "read", "eval", "exit", "history", "alias", "unalias",
        ]
    }

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
                    crate::warn!("which: warning: unsupported option '{}'", a);
                    query_start = i + 1;
                }
                _ => break,
            }
        }
        let queries = &args[query_start..];
        if queries.is_empty() {
            return CommandOutput::error("which: missing operand\n".to_string(), 1);
        }

        let known_cmds = Self::known_builtin_commands();

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

    /// `command -v foo` — the standard way shells locate a command. Prints the
    /// command name (builtin) or filesystem path (external), and returns
    /// exit 0 when found, 1 when not.
    pub fn cmd_command(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(
                "Usage: command [-v|-V] COMMAND [ARG...]\n  -v  show command name/path\n  -V  show verbose description\n".to_string(),
            );
        }
        let mut verbose = false;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-v" | "--version-check" => {}
                "-V" => verbose = true,
                a if a.starts_with('-') => {
                    crate::warn!("command: warning: unsupported option '{}'", a);
                }
                _ => break,
            }
            i += 1;
        }
        if i >= args.len() {
            return CommandOutput::error("command: missing operand\n".to_string(), 1);
        }
        let queries = &args[i..];
        let known_cmds = Self::known_builtin_commands();

        let mut output = String::new();
        let mut any_not_found = false;
        for arg in queries {
            if known_cmds.contains(arg) {
                if verbose {
                    output.push_str(&format!("{} is a fastshell builtin\n", arg));
                } else {
                    output.push_str(arg);
                    output.push('\n');
                }
            } else {
                let result = std::process::Command::new("which")
                    .arg(arg)
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                if result.is_empty() {
                    any_not_found = true;
                } else {
                    output.push_str(&result);
                    output.push('\n');
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

    #[test]
    fn test_command_v_builtin() {
        let mut s = mk_shell();
        let out = s.execute("command", &["-v", "ls"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("ls"), "command -v ls should print ls: {}", out.stdout);
    }

    #[test]
    fn test_command_v_unknown_exits_nonzero() {
        let mut s = mk_shell();
        let out = s.execute("command", &["-v", "nonexistent_cmd_xyz123"], None);
        assert_eq!(out.exit_code, 1);
        assert!(out.stdout.is_empty(), "command -v unknown should print nothing");
    }
}
