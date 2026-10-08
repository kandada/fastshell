// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::io::Read;

impl Shell {
    pub fn cmd_gunzip(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut to_stdout = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-c" | "--stdout" => to_stdout = true,
                _ if arg.starts_with('-') => {}
                _ => files.push(arg.to_string()),
            }
        }

        if files.is_empty() {
            // `... | gunzip` — decompress the byte-accurate stdin (or the
            // pipeline's text stdin for non-binary producers).
            if let Some(bytes) = self
                .take_binary_in()
                .or_else(|| stdin.map(|s| s.as_bytes().to_vec()))
            {
                let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
                let mut out = Vec::new();
                if decoder.read_to_end(&mut out).is_ok() {
                    if !out.is_empty() {
                        self.set_binary_out(out.clone());
                    }
                    return CommandOutput::success(String::from_utf8_lossy(&out).to_string());
                }
            }
            return CommandOutput::error("gunzip: missing file operand\n".to_string(), 1);
        }

        let mut output_bytes = Vec::new();

        for file in &files {
            let compressed = match self.vfs.read(file, &self.cwd) {
                Ok(b) => b,
                Err(e) => {
                    return CommandOutput::error(format!("gunzip: {}: {}\n", file, e), 1);
                }
            };

            let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
            let mut decompressed = Vec::new();
            if let Err(e) = decoder.read_to_end(&mut decompressed) {
                return CommandOutput::error(format!("gunzip: {}: {}\n", file, e), 1);
            }

            if to_stdout {
                output_bytes.extend_from_slice(&decompressed);
            } else {
                let out_name = file
                    .strip_suffix(".gz")
                    .or_else(|| file.strip_suffix(".GZ"))
                    .unwrap_or(file);
                let out_name = if out_name.is_empty() {
                    "uncompressed"
                } else {
                    out_name
                };

                if let Err(e) = self.vfs.write_bytes(out_name, &self.cwd, &decompressed) {
                    return CommandOutput::error(format!("gunzip: {}: {}\n", out_name, e), 1);
                }

                let _ = self.vfs.remove_file(file, &self.cwd);
            }
        }

        if to_stdout {
            if !output_bytes.is_empty() {
                self.set_binary_out(output_bytes.clone());
            }
            CommandOutput::success(String::from_utf8_lossy(&output_bytes).to_string())
        } else {
            CommandOutput::success(String::new())
        }
    }
}
