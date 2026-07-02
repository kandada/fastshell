// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_wget(&self, args: &[&str]) -> CommandOutput {
        let mut url: Option<String> = None;
        let mut output_file: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-O" => {
                    if i + 1 < args.len() {
                        output_file = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => url = Some(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let url = match url {
            Some(u) => u,
            None => return CommandOutput::error("wget: missing URL\n".to_string(), 1),
        };

        let wget_host = url
            .split("://")
            .nth(1)
            .unwrap_or(&url)
            .split('/')
            .next()
            .unwrap_or(&url)
            .split(':')
            .next()
            .unwrap_or(&url);
        if let Some(perm) = self.check_network_permission(wget_host) {
            return perm;
        }

        let result = crate::shell::http_request("GET", &url, None, true);
        match result {
            Ok(body) => {
                let filename = match output_file {
                    Some(ref f) => f.clone(),
                    None => crate::shell::extract_filename_from_url(&url),
                };
                match self.vfs.write(&filename, &self.cwd, &body) {
                    Ok(_) => CommandOutput::success(format!(
                        "{} saved [{} bytes]\n",
                        filename,
                        body.len()
                    )),
                    Err(e) => CommandOutput::error(format!("wget: {}: {}\n", filename, e), 1),
                }
            }
            Err(e) => CommandOutput::error(format!("wget: {}\n", e), 1),
        }
    }
}
