// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_ssh(&self, args: &[&str]) -> CommandOutput {
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
                _ => {}
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
