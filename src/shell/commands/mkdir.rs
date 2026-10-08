// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const MKDIR_HELP_TEXT: &str = "\
Usage: mkdir [OPTION]... DIRECTORY...
Create the DIRECTORY(ies), if they do not already exist.

  -p, --parents     no error if existing, make parent directories as needed
  -m, --mode=MODE   set file mode (as in chmod), not a=rwx - umask
  -v, --verbose     print a message for each created directory
  -h, --help        display this help and exit
";

impl Shell {
    pub fn cmd_mkdir(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(MKDIR_HELP_TEXT.to_string());
        }
        let mut create_parents = false;
        let mut verbose = false;
        let mut mode: Option<u32> = None;
        let mut dirs = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg.starts_with("--") {
                match arg {
                    "--parents" => create_parents = true,
                    "--verbose" => verbose = true,
                    "--mode" => {
                        if i + 1 < args.len() {
                            mode = parse_mode(args[i + 1]);
                            i += 1;
                        }
                    }
                    a if a.starts_with("--mode=") => mode = parse_mode(&a[7..]),
                    _ => crate::warn!("mkdir: warning: unsupported option '{}'", arg),
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg.chars().skip(1).collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'p' => create_parents = true,
                        'v' => verbose = true,
                        'm' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let m = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            mode = parse_mode(&m);
                            j = chars.len();
                            continue;
                        }
                        _ => crate::warn!("mkdir: warning: unsupported option '-{}'", chars[j]),
                    }
                    j += 1;
                }
            } else {
                dirs.push(arg.to_string());
            }
            i += 1;
        }

        if dirs.is_empty() {
            return CommandOutput::error("mkdir: missing operand\n".to_string(), 1);
        }

        let mut output = String::new();
        for dir in &dirs {
            let result = if create_parents {
                self.vfs.create_dir_all(dir, &self.cwd)
            } else {
                self.vfs.create_dir(dir, &self.cwd)
            };
            match result {
                Ok(_) => {
                    if let Some(m) = mode {
                        if let Ok(resolved) = self.vfs.resolve(dir, &self.cwd) {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(
                                    &resolved,
                                    std::fs::Permissions::from_mode(m),
                                );
                            }
                            #[cfg(not(unix))]
                            {
                                let _ = resolved;
                            }
                        }
                    }
                    if verbose {
                        output.push_str(&format!("mkdir: created directory '{}'\n", dir));
                    }
                }
                Err(e) => {
                    return CommandOutput::error(format!("mkdir: {}: {}\n", dir, e), 1);
                }
            }
        }

        CommandOutput::success(output)
    }
}

fn parse_mode(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim_start_matches('0'), 8).ok()
}
