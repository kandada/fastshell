use crate::shell::{Shell, CommandOutput};
use std::fs;

impl Shell {
    pub fn cmd_chmod(&self, args: &[&str]) -> CommandOutput {
        if args.len() < 2 {
            return CommandOutput::error("chmod: missing operand\n".to_string(), 1);
        }

        let mode_str = args[0];
        let mode = match u32::from_str_radix(mode_str, 8) {
            Ok(m) => m,
            Err(_) => {
                return CommandOutput::error(
                    format!("chmod: {}: invalid mode\n", mode_str),
                    1,
                );
            }
        };

        for path in &args[1..] {
            let target = match self.vfs.resolve(path, &self.cwd) {
                Ok(p) => p,
                Err(e) => {
                    return CommandOutput::error(format!("chmod: {}: {}\n", path, e), 1);
                }
            };

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = target.metadata().map(|m| m.permissions()).unwrap_or_else(|_| {
                    std::fs::Permissions::from_mode(0o644)
                });
                perms.set_mode(mode);
                if let Err(e) = fs::set_permissions(&target, perms) {
                    return CommandOutput::error(format!("chmod: {}: {}\n", path, e), 1);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = target;
                return CommandOutput::error(
                    "chmod: not supported on this platform\n".to_string(),
                    1,
                );
            }
        }

        CommandOutput::success(String::new())
    }
}
