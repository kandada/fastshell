use crate::shell::{Shell, CommandOutput};
use std::io::Write;

impl Shell {
    pub fn cmd_gzip(&self, args: &[&str]) -> CommandOutput {
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

            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            if let Err(e) = encoder.write_all(&input_bytes) {
                return CommandOutput::error(format!("gzip: {}: {}\n", file, e), 1);
            }
            let compressed = match encoder.finish() {
                Ok(c) => c,
                Err(e) => {
                    return CommandOutput::error(format!("gzip: {}: {}\n", file, e), 1);
                }
            };

            if to_stdout {
                output_bytes.extend_from_slice(&compressed);
            } else {
                let out_name = format!("{}.gz", file);
                if let Err(e) = self.vfs.write_bytes(&out_name, &self.cwd, &compressed) {
                    return CommandOutput::error(format!("gzip: {}: {}\n", out_name, e), 1);
                }
                let _ = self.vfs.remove_file(file, &self.cwd);
            }
        }

        if to_stdout {
            CommandOutput::success(String::from_utf8_lossy(&output_bytes).to_string())
        } else {
            CommandOutput::success(String::new())
        }
    }
}
