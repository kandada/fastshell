// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use base64::Engine as _;

impl Shell {
    pub fn cmd_base64(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut decode = false;
        let mut ignore_garbage = false;
        let mut wrap = 76usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg.starts_with("--") {
                match arg {
                    "--decode" => decode = true,
                    "--ignore-garbage" => ignore_garbage = true,
                    "--wrap" => {
                        if i + 1 < args.len() {
                            wrap = args[i + 1].parse().unwrap_or(76);
                            i += 1;
                        }
                    }
                    _ => {}
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg.chars().skip(1).collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'd' => decode = true,
                        'i' => ignore_garbage = true,
                        'w' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            wrap = val.parse().unwrap_or(76);
                            j = chars.len();
                            continue;
                        }
                        _ => {}
                    }
                    j += 1;
                }
            } else {
                files.push(arg.to_string());
            }
            i += 1;
        }

        let input_data: Vec<u8> = if files.is_empty() {
            match stdin {
                Some(s) => s.as_bytes().to_vec(),
                None => return CommandOutput::error("base64: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = Vec::new();
            for file in &files {
                match self.vfs.read(file, &self.cwd) {
                    Ok(data) => all.extend_from_slice(&data),
                    Err(e) => return CommandOutput::error(format!("base64: {}: {}\n", file, e), 1),
                }
            }
            all
        };

        if decode {
            let cleaned: String = if ignore_garbage {
                String::from_utf8_lossy(&input_data)
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
                    .collect()
            } else {
                String::from_utf8_lossy(&input_data)
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect()
            };
            match base64::engine::general_purpose::STANDARD.decode(&cleaned) {
                Ok(bytes) => CommandOutput::success(String::from_utf8_lossy(&bytes).to_string()),
                Err(e) => CommandOutput::error(format!("base64: decode error: {}\n", e), 1),
            }
        } else {
            let encoded = base64::engine::general_purpose::STANDARD.encode(&input_data);
            if wrap == 0 {
                CommandOutput::success(encoded + "\n")
            } else {
                let wrapped = encoded
                    .as_bytes()
                    .chunks(wrap)
                    .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
                    .collect::<Vec<&str>>()
                    .join("\n");
                CommandOutput::success(wrapped + "\n")
            }
        }
    }
}
