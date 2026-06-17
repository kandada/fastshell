use crate::shell::{Shell, CommandOutput};
use std::io::{Read, Write};

impl Shell {
    pub fn cmd_gzip(&self, args: &[&str]) -> CommandOutput {
        let mut to_stdout = false;
        let mut decompress = false;
        let mut keep = false;
        let mut level = flate2::Compression::default();
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-c" | "--stdout" => to_stdout = true,
                "-d" | "--decompress" => decompress = true,
                "-k" | "--keep" => keep = true,
                "-1" => level = flate2::Compression::new(1),
                "-2" => level = flate2::Compression::new(2),
                "-3" => level = flate2::Compression::new(3),
                "-4" => level = flate2::Compression::new(4),
                "-5" => level = flate2::Compression::new(5),
                "-6" => level = flate2::Compression::new(6),
                "-7" => level = flate2::Compression::new(7),
                "-8" => level = flate2::Compression::new(8),
                "-9" => level = flate2::Compression::new(9),
                _ if arg.starts_with('-') => {}
                _ => files.push(arg.to_string()),
            }
        }

        if files.is_empty() {
            return CommandOutput::error("gzip: missing file operand\n".to_string(), 1);
        }

        let mut output_bytes = Vec::new();

        for file in &files {
            let input_bytes = match self.vfs.read(file, &self.cwd) {
                Ok(b) => b,
                Err(e) => {
                    return CommandOutput::error(format!("gzip: {}: {}\n", file, e), 1);
                }
            };

            let result_bytes = if decompress {
                let mut decoder = flate2::read::GzDecoder::new(&input_bytes[..]);
                let mut decompressed = Vec::new();
                if let Err(e) = decoder.read_to_end(&mut decompressed) {
                    return CommandOutput::error(format!("gzip: {}: {}\n", file, e), 1);
                }
                decompressed
            } else {
                let mut encoder = flate2::write::GzEncoder::new(Vec::new(), level);
                if let Err(e) = encoder.write_all(&input_bytes) {
                    return CommandOutput::error(format!("gzip: {}: {}\n", file, e), 1);
                }
                match encoder.finish() {
                    Ok(c) => c,
                    Err(e) => {
                        return CommandOutput::error(format!("gzip: {}: {}\n", file, e), 1);
                    }
                }
            };

            if to_stdout {
                output_bytes.extend_from_slice(&result_bytes);
            } else {
                let out_name = if decompress {
                    file.strip_suffix(".gz")
                        .or_else(|| file.strip_suffix(".GZ"))
                        .unwrap_or(file)
                        .to_string()
                } else {
                    format!("{}.gz", file)
                };

                if out_name.is_empty() {
                    return CommandOutput::error(format!("gzip: {}: empty output name\n", file), 1);
                }

                if let Err(e) = self.vfs.write_bytes(&out_name, &self.cwd, &result_bytes) {
                    return CommandOutput::error(format!("gzip: {}: {}\n", out_name, e), 1);
                }

                if !keep {
                    let _ = self.vfs.remove_file(file, &self.cwd);
                }
            }
        }

        if to_stdout {
            CommandOutput::success(String::from_utf8_lossy(&output_bytes).to_string())
        } else {
            CommandOutput::success(String::new())
        }
    }
}
