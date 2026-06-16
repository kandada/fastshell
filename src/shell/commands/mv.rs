use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_mv(&self, args: &[&str]) -> CommandOutput {
        let operands: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();

        if operands.len() < 2 {
            return CommandOutput::error("mv: missing file operand\n".to_string(), 1);
        }

        let dest = operands.last().unwrap();
        let sources = &operands[..operands.len() - 1];

        for src in sources {
            let src_path = match self.vfs.resolve(src, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("mv: {}: {}\n", src, e), 1),
            };

            let dest_path = if self.vfs.is_dir(dest, &self.cwd) {
                let fname = src_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| src.to_string());
                format!("{}/{}", dest.trim_end_matches('/'), fname)
            } else {
                dest.to_string()
            };

            if let Err(e) = self.vfs.rename(src, &dest_path, &self.cwd) {
                return CommandOutput::error(format!("mv: {}\n", e), 1);
            }
        }

        CommandOutput::success(String::new())
    }
}
