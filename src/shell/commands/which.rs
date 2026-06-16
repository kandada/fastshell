use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_which(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("which: missing operand\n".to_string(), 1);
        }

        let known_cmds: &[&str] = &[
            "ls", "cd", "pwd", "mkdir", "rm", "cp", "mv", "cat", "find", "grep",
            "echo", "touch", "chmod", "ps", "kill", "curl", "wget", "gzip",
            "gunzip", "tar", "ping", "ssh", "git", "head", "tail", "wc", "diff",
            "sed", "sort", "uniq", "tee", "xargs", "which",
            "cut", "awk", "tr", "sleep", "date", "true", "false", "test",
            "base64", "sha256sum", "sha512sum", "md5sum", "du", "df", "stat",
            "jq", "env", "printenv", "printf", "basename", "dirname", "realpath",
            "file", "column", "seq", "zip", "unzip",
            "shuf", "uuidgen", "rev", "split", "comm", "xxd", "expr",
            "uname", "hostname", "whoami", "id", "pgrep", "pkill", "paste", "timeout",
        ];

        let mut output = String::new();
        for arg in args {
            if arg.starts_with('-') { continue; }
            if known_cmds.contains(arg) {
                output.push_str(&format!("{}: built-in fastshell command\n", arg));
            } else {
                let result = std::process::Command::new("which")
                    .arg(arg)
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_else(|| format!("{} not found\n", arg));
                output.push_str(&result);
            }
        }

        CommandOutput::success(output)
    }
}
