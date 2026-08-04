// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const SSH_HELP_TEXT: &str = "\
ssh: OpenSSH remote login client
Usage: ssh [OPTIONS] [user@]host [command]
Options:
  -p PORT     Port to connect to on remote host
  -i FILE     Identity file (private key)
  -h, --help  Show this help message
";

impl Shell {
    pub fn cmd_ssh(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(SSH_HELP_TEXT.to_string());
        }
        let mut port: u16 = 22;
        let mut identity: Option<String> = None;
        let mut target: Option<String> = None;
        let mut command: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-p" => {
                    if i + 1 < args.len() {
                        port = args[i + 1].parse().unwrap_or(22);
                        i += 1;
                    }
                }
                "-i" => {
                    if i + 1 < args.len() {
                        identity = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') && target.is_none() => {
                    target = Some(arg.to_string());
                }
                arg if !arg.starts_with('-') => {
                    let rest: Vec<&str> = args[i..].iter().copied().collect();
                    command = Some(rest.join(" "));
                    break;
                }
                _ => eprintln!("ssh: warning: unsupported option '{}'", args[i]),
            }
            i += 1;
        }

        let target = match target {
            Some(t) => t,
            None => return CommandOutput::error("ssh: missing hostname\n".to_string(), 1),
        };

        let (user, host) = if let Some(at) = target.find('@') {
            (target[..at].to_string(), target[at + 1..].to_string())
        } else {
            ("root".to_string(), target.clone())
        };

        if let Some(perm) = self.check_network_permission(&host) {
            return perm;
        }

        let command = command.unwrap_or_else(|| "hostname".to_string());

        let rt = match tokio::runtime::Runtime::new() {
            Ok(r) => r,
            Err(e) => return CommandOutput::error(format!("ssh: {}\n", e), 1),
        };

        rt.block_on(async {
            crate::shell::ssh_exec_russh(&host, port, &user, &command, identity.as_deref()).await
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;

    fn mk_shell() -> Shell {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("fastshell_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_ssh_help() {
        let mut shell = mk_shell();
        let out = shell.execute("ssh", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_ssh_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("ssh", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
