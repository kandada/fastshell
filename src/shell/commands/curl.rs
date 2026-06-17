use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_curl(&self, args: &[&str]) -> CommandOutput {
        let mut url: Option<String> = None;
        let mut output_file: Option<String> = None;
        let mut follow_redirects = false;
        let mut silent = false;
        let mut method = "GET".to_string();
        let mut data: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-o" => {
                    if i + 1 < args.len() {
                        output_file = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-O" => {
                    output_file = Some("__auto__".to_string());
                }
                "-L" => follow_redirects = true,
                "-s" => silent = true,
                "-X" => {
                    if i + 1 < args.len() {
                        method = args[i + 1].to_uppercase();
                        i += 1;
                    }
                }
                "-d" => {
                    if i + 1 < args.len() {
                        data = Some(args[i + 1].to_string());
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
            None => return CommandOutput::error("curl: no URL specified\n".to_string(), 1),
        };

        let curl_host = url.split("://").nth(1).unwrap_or(&url).split('/').next().unwrap_or(&url).split(':').next().unwrap_or(&url);
        if let Some(perm) = self.check_network_permission(curl_host) {
            return perm;
        }

        let result = crate::shell::http_request(&method, &url, data.as_deref(), follow_redirects);

        match result {
            Ok(body) => {
                if let Some(ref file) = output_file {
                    let filename = if file == "__auto__" {
                        crate::shell::extract_filename_from_url(&url)
                    } else {
                        file.clone()
                    };
                    match self.vfs.write(&filename, &self.cwd, &body) {
                        Ok(_) => {
                            if !silent {
                                return CommandOutput::success(format!(
                                    "Downloaded: {} ({} bytes)\n",
                                    filename,
                                    body.len()
                                ));
                            }
                            CommandOutput::success(String::new())
                        }
                        Err(e) => CommandOutput::error(format!("curl: {}: {}\n", filename, e), 1),
                    }
                } else {
                    CommandOutput::success(body)
                }
            }
            Err(e) => CommandOutput::error(format!("curl: {}\n", e), 1),
        }
    }
}
