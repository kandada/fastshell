// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::io::{Read, Write};

impl Shell {
    pub fn cmd_zip(&self, args: &[&str]) -> CommandOutput {
        let mut zip_name = String::new();
        let mut files = Vec::new();

        for arg in args {
            if arg.starts_with('-') {
                continue;
            }
            if zip_name.is_empty() {
                zip_name = arg.to_string();
            } else {
                files.push(arg.to_string());
            }
        }

        if zip_name.is_empty() || files.is_empty() {
            return CommandOutput::error(
                "zip: usage: zip archive.zip file1 [file2 ...]\n".to_string(),
                1,
            );
        }

        let resolved_zip = match self.vfs.resolve(&zip_name, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("zip: {}: {}\n", zip_name, e), 1),
        };

        let zip_file = match std::fs::File::create(&resolved_zip) {
            Ok(f) => f,
            Err(e) => return CommandOutput::error(format!("zip: create error: {}\n", e), 1),
        };

        let mut zip_writer = zip::ZipWriter::new(zip_file);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for file in &files {
            match self.vfs.read(file, &self.cwd) {
                Ok(data) => {
                    if let Err(e) = zip_writer.start_file(file.to_string(), options) {
                        return CommandOutput::error(format!("zip: {}: {}\n", file, e), 1);
                    }
                    if let Err(e) = zip_writer.write_all(&data) {
                        return CommandOutput::error(
                            format!("zip: {}: write error: {}\n", file, e),
                            1,
                        );
                    }
                }
                Err(e) => {
                    return CommandOutput::error(format!("zip: {}: {}\n", file, e), 1);
                }
            }
        }

        if let Err(e) = zip_writer.finish() {
            return CommandOutput::error(format!("zip: finish error: {}\n", e), 1);
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_unzip(&self, args: &[&str]) -> CommandOutput {
        let mut zip_name = String::new();

        for arg in args {
            if !arg.starts_with('-') {
                zip_name = arg.to_string();
                break;
            }
        }

        if zip_name.is_empty() {
            return CommandOutput::error("unzip: missing operand\n".to_string(), 1);
        }

        let data = match self.vfs.read(&zip_name, &self.cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("unzip: {}: {}\n", zip_name, e), 1),
        };

        let cursor = std::io::Cursor::new(data);
        let mut archive = match zip::ZipArchive::new(cursor) {
            Ok(a) => a,
            Err(e) => return CommandOutput::error(format!("unzip: read error: {}\n", e), 1),
        };

        let mut output = String::new();
        for i in 0..archive.len() {
            let mut entry = match archive.by_index(i) {
                Ok(e) => e,
                Err(e) => {
                    output.push_str(&format!("unzip: entry error: {}\n", e));
                    continue;
                }
            };

            let name = entry.name().to_string();
            if entry.is_dir() {
                if let Err(e) = self.vfs.create_dir(&name, &self.cwd) {
                    output.push_str(&format!("unzip: {}: {}\n", name, e));
                }
                continue;
            }

            let mut content = Vec::new();
            if let Err(e) = entry.read_to_end(&mut content) {
                output.push_str(&format!("unzip: {}: read error: {}\n", name, e));
                continue;
            }

            match self.vfs.write_bytes(&name, &self.cwd, &content) {
                Ok(_) => {}
                Err(e) => output.push_str(&format!("unzip: {}: {}\n", name, e)),
            }
        }

        if output.is_empty() {
            CommandOutput::success(format!("Archive: {}\n", zip_name))
        } else {
            CommandOutput {
                stdout: format!("Archive: {}\n", zip_name),
                stderr: output,
                exit_code: 0,
            }
        }
    }
}
