use base64::Engine as _;
use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_base64(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut decode = false;
        let mut wrap = 76usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-d" | "--decode" => decode = true,
                "-w" => {
                    if i + 1 < args.len() {
                        wrap = args[i + 1].parse().unwrap_or(76);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-w") && arg.len() > 2 => {
                    wrap = arg[2..].parse().unwrap_or(76);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
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
            let cleaned: String = String::from_utf8_lossy(&input_data)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
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
                    .map(|chunk| std::str::from_utf8(chunk).unwrap())
                    .collect::<Vec<&str>>()
                    .join("\n");
                CommandOutput::success(wrapped + "\n")
            }
        }
    }
}
