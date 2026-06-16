use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_ln(&self, args: &[&str]) -> CommandOutput {
        let mut symbolic = false;
        let mut force = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-s" | "--symbolic" => symbolic = true,
                "-f" | "--force" => force = true,
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        if files.len() < 2 {
            return CommandOutput::error("ln: missing file operand\n".to_string(), 1);
        }

        let target = &files[files.len() - 1];
        let target_path = match self.vfs.resolve(target, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("ln: {}: {}\n", target, e), 1),
        };

        let is_target_dir = target_path.is_dir();

        for source in &files[..files.len() - 1] {
            let source_path = match self.vfs.resolve(source, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("ln: {}: {}\n", source, e), 1),
            };

            let link_path = if is_target_dir {
                let name = source_path.file_name().unwrap_or_default();
                target_path.join(name)
            } else {
                target_path.clone()
            };

            if force && link_path.exists() {
                let _ = std::fs::remove_file(&link_path);
            }

            let result = if symbolic {
                std::os::unix::fs::symlink(&source_path, &link_path)
            } else {
                std::fs::hard_link(&source_path, &link_path)
            };

            if let Err(e) = result {
                return CommandOutput::error(format!("ln: {}: {}\n", source, e), 1);
            }
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_readlink(&self, args: &[&str]) -> CommandOutput {
        let mut canonicalize = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-f" | "--canonicalize" => canonicalize = true,
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        if files.is_empty() {
            return CommandOutput::error("readlink: missing operand\n".to_string(), 1);
        }

        let mut output = String::new();
        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(p) => p,
                Err(e) => {
                    output.push_str(&format!("readlink: {}: {}\n", file, e));
                    continue;
                }
            };

            if canonicalize {
                match std::fs::canonicalize(&resolved) {
                    Ok(p) => output.push_str(&format!("{}\n", p.display())),
                    Err(e) => output.push_str(&format!("readlink: {}: {}\n", file, e)),
                }
            } else {
                match std::fs::read_link(&resolved) {
                    Ok(p) => output.push_str(&format!("{}\n", p.display())),
                    Err(e) => output.push_str(&format!("readlink: {}: {}\n", file, e)),
                }
            }
        }

        CommandOutput::success(output)
    }

    pub fn cmd_rmdir(&self, args: &[&str]) -> CommandOutput {
        let mut parents = false;
        let mut dirs = Vec::new();

        for arg in args {
            match *arg {
                "-p" | "--parents" => parents = true,
                arg if !arg.starts_with('-') => dirs.push(arg.to_string()),
                _ => {}
            }
        }

        if dirs.is_empty() {
            return CommandOutput::error("rmdir: missing operand\n".to_string(), 1);
        }

        for dir in &dirs {
            let resolved = match self.vfs.resolve(dir, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("rmdir: {}: {}\n", dir, e), 1),
            };

            if parents {
                let mut path = resolved.clone();
                loop {
                    match std::fs::remove_dir(&path) {
                        Ok(_) => {}
                        Err(e) => {
                            if path == resolved {
                                return CommandOutput::error(format!("rmdir: {}: {}\n", dir, e), 1);
                            }
                            break;
                        }
                    }
                    if let Some(parent) = path.parent() {
                        if parent.as_os_str().is_empty() { break; }
                        path = parent.to_path_buf();
                    } else {
                        break;
                    }
                }
            } else {
                if let Err(e) = std::fs::remove_dir(&resolved) {
                    return CommandOutput::error(format!("rmdir: {}: {}\n", dir, e), 1);
                }
            }
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_mktemp(&self, args: &[&str]) -> CommandOutput {
        let mut directory = false;
        let mut template = String::new();

        for arg in args {
            match *arg {
                "-d" | "--directory" => directory = true,
                arg if !arg.starts_with('-') && template.is_empty() => template = arg.to_string(),
                _ => {}
            }
        }

        if template.is_empty() {
            template = "/tmp/tmp.XXXXXXXXXX".to_string();
        }

        let resolved = match self.vfs.resolve(&template, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("mktemp: {}: {}\n", template, e), 1),
        };

        let _parent = resolved.parent().unwrap_or(&resolved);
        let mut rng = simple_rng();

        for _ in 0..100 {
            let mut name = resolved.clone();
            let stem = name.file_stem().unwrap_or_default().to_string_lossy();
            let suffix = format!("{:06x}", rng.next());
            let new_name = format!("{}{}", stem, suffix);
            name.set_file_name(&new_name);

            if !name.exists() {
                let result = if directory {
                    std::fs::create_dir(&name)
                } else {
                    std::fs::File::create(&name).map(|_| {})
                };
                match result {
                    Ok(_) => return CommandOutput::success(format!("{}\n", name.display())),
                    Err(e) => return CommandOutput::error(format!("mktemp: {}\n", e), 1),
                }
            }
        }

        CommandOutput::error("mktemp: failed to create temporary file\n".to_string(), 1)
    }

    pub fn cmd_tac(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut files = Vec::new();
        let mut separator = "\n".to_string();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-s" | "--separator" => {
                    if i + 1 < args.len() {
                        separator = args[i + 1].to_string();
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let content = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("tac: missing operand\n".to_string(), 1),
            }
        } else {
            let mut all = String::new();
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(c) => {
                        if files.len() > 1 && !all.is_empty() {
                            all.push_str(&separator);
                        }
                        all.push_str(&c);
                    }
                    Err(e) => return CommandOutput::error(format!("tac: {}: {}\n", file, e), 1),
                }
            }
            all
        };

        let parts: Vec<&str> = content.split(&separator).collect();
        let mut output = String::new();
        for part in parts.iter().rev() {
            output.push_str(part);
            output.push_str(&separator);
        }

        CommandOutput::success(output)
    }

    pub fn cmd_nl(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut files = Vec::new();
        let mut body_numbering = true;

        for arg in args {
            match *arg {
                "-b" => {
                    // body numbering style (default 't' = non-empty only)
                    body_numbering = true;
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        let content = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("nl: missing operand\n".to_string(), 1),
            }
        } else {
            let mut all = String::new();
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(c) => all.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("nl: {}: {}\n", file, e), 1),
                }
            }
            all
        };

        let mut output = String::new();
        let mut line_num: usize = 1;
        for line in content.lines() {
            if body_numbering && line.trim().is_empty() {
                output.push_str("      \t\n");
            } else {
                output.push_str(&format!("{:>6}\t{}\n", line_num, line));
                line_num += 1;
            }
        }

        CommandOutput::success(output)
    }
}

struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.state
    }
}

fn simple_rng() -> SimpleRng {
    use std::hash::{BuildHasher, Hash, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    std::time::SystemTime::now().hash(&mut h);
    SimpleRng { state: h.finish() }
}
