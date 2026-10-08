// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_ln(&self, args: &[&str]) -> CommandOutput {
        let mut symbolic = false;
        let mut force = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-s" | "--symbolic" => symbolic = true,
                "-f" | "--force" => force = true,
                "-sf" | "-fs" => {
                    symbolic = true;
                    force = true;
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        if files.len() < 2 {
            return CommandOutput::error("ln: missing file operand\n".to_string(), 1);
        }

        let target = &files[files.len() - 1];
        // Follow the symlink only to decide whether the destination is a dir;
        // the LINK PATH itself must NOT be resolved. Previously `-f` removed the
        // file a destination symlink pointed to (destructive self-loop bug).
        let target_path = match self.vfs.resolve(target, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("ln: {}: {}\n", target, e), 1),
        };
        let target_link = self
            .vfs
            .resolve_no_follow(target, &self.cwd)
            .unwrap_or_else(|_| target_path.clone());

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
                target_link.clone()
            };

            if force {
                // Remove the existing destination ENTRY (including a dangling
                // symlink) — never the file it points to.
                if let Ok(md) = std::fs::symlink_metadata(&link_path) {
                    let _ = if md.is_dir() && !md.file_type().is_symlink() {
                        std::fs::remove_dir(&link_path)
                    } else {
                        std::fs::remove_file(&link_path)
                    };
                }
            }

            let result = if symbolic {
                // Store the target exactly as given (relative stays relative),
                // matching `ln -s target link` (bash does not canonicalize).
                std::os::unix::fs::symlink(source, &link_path)
            } else {
                match std::fs::hard_link(&source_path, &link_path) {
                    Ok(()) => Ok(()),
                    Err(_) => {
                        // Hard links may be unsupported on some (mobile) file
                        // systems; fall back to a copy so `ln src dst` still
                        // produces a usable file.
                        std::fs::copy(&source_path, &link_path).map(|_| ())
                    }
                }
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
            // Do NOT canonicalize: `readlink` must inspect the link itself, not
            // its target. Build the host path from the sandbox root + cwd.
            let root = self.vfs.root();
            let resolved = if file.starts_with('/') {
                root.join(file.trim_start_matches('/'))
            } else {
                root.join(self.cwd.trim_start_matches('/')).join(file)
            };

            if canonicalize {
                // Follow the link chain with VFS semantics (a link target like
                // `/t.txt` is VFS-absolute, not a host path) and report the
                // result as a VFS path — never the host location.
                match self.vfs.resolve(file, &self.cwd) {
                    Ok(p) => output.push_str(&format!("{}\n", self.vfs.to_vpath(&p))),
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
                        if parent.as_os_str().is_empty() {
                            break;
                        }
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
                "-t" | "--tmpdir" => {} // default (/tmp) already
                arg if !arg.starts_with('-') && template.is_empty() => template = arg.to_string(),
                _ => {}
            }
        }
        if template.is_empty() {
            template = "tmp.XXXXXXXXXX".to_string();
        }
        // A bare name (or `-t NAME`) goes under /tmp; otherwise use as given.
        let template = if template.contains('/') {
            template
        } else {
            format!("/tmp/{template}")
        };
        let base = match template.rsplit_once('/') {
            Some((d, _)) if !d.is_empty() => d.to_string(),
            _ => "/".to_string(),
        };
        // Create the base directory through the VFS (sandbox-aware). This is the
        // step that used to fail on device when `/tmp` (or TMPDIR) was missing.
        if self.vfs.create_dir_all(&base, &self.cwd).is_err() {
            return CommandOutput::error(
                format!("mktemp: cannot create directory '{}'\n", base),
                1,
            );
        }
        let bname = template
            .rsplit('/')
            .next()
            .unwrap_or("tmp.XXXXXXXXXX")
            .to_string();
        let mut rng = simple_rng();
        for _ in 0..100 {
            let mut name = String::with_capacity(bname.len());
            for ch in bname.chars() {
                if ch == 'X' {
                    // A fresh digit per placeholder (`str::replace` reused one).
                    name.push_str(&format!("{:x}", rng.next() % 16));
                } else {
                    name.push(ch);
                }
            }
            let logical = if base == "/" {
                format!("/{name}")
            } else {
                format!("{base}/{name}")
            };
            let host = match self.vfs.resolve(&logical, &self.cwd) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if host.exists() {
                continue;
            }
            let ok = if directory {
                self.vfs.create_dir_all(&logical, &self.cwd).is_ok()
            } else {
                self.vfs.write_bytes(&logical, &self.cwd, b"").is_ok()
            };
            if ok {
                // Report the VFS path, not the host location.
                let _ = host;
                return CommandOutput::success(format!("{logical}\n"));
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
                match self.read_text_lossy(file) {
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

        // Split on the separator, ignoring a single trailing separator so a
        // trailing newline does not become a leading blank line after reversal
        // (`printf 'a\nb\n' | tac` → `b\na\n`, not `\nb\na`).
        let had_trailing = content.ends_with(&separator);
        let trimmed = if had_trailing {
            &content[..content.len() - separator.len()]
        } else {
            content.as_str()
        };
        let parts: Vec<&str> = if trimmed.is_empty() {
            Vec::new()
        } else {
            trimmed.split(&separator).collect()
        };
        let mut output = String::new();
        for (idx, part) in parts.iter().rev().enumerate() {
            if idx > 0 {
                output.push_str(&separator);
            }
            output.push_str(part);
        }
        if had_trailing && !parts.is_empty() {
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
                match self.read_text_lossy(file) {
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
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }
}

fn simple_rng() -> SimpleRng {
    use std::hash::{BuildHasher, Hash, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    std::time::SystemTime::now().hash(&mut h);
    SimpleRng { state: h.finish() }
}
