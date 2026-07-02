// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{human_size, CommandOutput, Shell};

impl Shell {
    pub fn cmd_du(&self, args: &[&str]) -> CommandOutput {
        let mut summarize = false;
        let mut max_depth: Option<usize> = None;
        let mut human = false;
        let mut paths = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-s" | "--summarize" => summarize = true,
                "-h" | "--human-readable" => human = true,
                "--max-depth" => {
                    if i + 1 < args.len() {
                        max_depth = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                arg if arg.starts_with("--max-depth=") => {
                    max_depth = arg[12..].parse().ok();
                }
                arg if !arg.starts_with('-') => paths.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        if paths.is_empty() {
            paths.push(".".to_string());
        }

        let depth = if summarize { Some(0) } else { max_depth };
        let mut output = String::new();
        for path in &paths {
            let resolved = match self.vfs.resolve(path, &self.cwd) {
                Ok(r) => r,
                Err(e) => {
                    output.push_str(&format!("du: {}: {}\n", path, e));
                    continue;
                }
            };
            let size = du_walk(&resolved, depth, 0);
            if human {
                output.push_str(&format!("{}\t{}\n", human_size(size), path));
            } else {
                output.push_str(&format!("{}\t{}\n", size, path));
            }
        }

        CommandOutput::success(output)
    }

    pub fn cmd_df(&self, _args: &[&str]) -> CommandOutput {
        let vfs_root = self.vfs.root();

        let mut output = String::new();
        output.push_str("Filesystem     1K-blocks      Used Available Use% Mounted on\n");

        match fs_stats(vfs_root) {
            Some((total, avail)) => {
                let used = total.saturating_sub(avail);
                let pct = if total > 0 {
                    (used as f64 / total as f64 * 100.0) as u64
                } else {
                    0
                };
                output.push_str(&format!(
                    "fastshell     {:>10} {:>10} {:>10} {:>3}% {}\n",
                    total / 1024,
                    used / 1024,
                    avail / 1024,
                    pct,
                    vfs_root.display(),
                ));
            }
            None => {
                output.push_str("df: cannot read filesystem stats\n");
            }
        }

        CommandOutput::success(output)
    }

    pub fn cmd_stat(&self, args: &[&str]) -> CommandOutput {
        let mut format: Option<String> = None;
        let mut files: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-c" => {
                    if i + 1 < args.len() {
                        format = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "--format" => {
                    if i + 1 < args.len() {
                        format = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                arg if arg.starts_with("--format=") => {
                    format = Some(arg[9..].to_string());
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        if files.is_empty() {
            return CommandOutput::error("stat: missing operand\n".to_string(), 1);
        }

        if let Some(ref fmt) = format {
            let mut output = String::new();
            for file in &files {
                let resolved = match self.vfs.resolve(file, &self.cwd) {
                    Ok(r) => r,
                    Err(e) => {
                        output.push_str(&format!("stat: {}: {}\n", file, e));
                        continue;
                    }
                };

                match std::fs::symlink_metadata(&resolved) {
                    Ok(meta) => {
                        output.push_str(&format_stat(&resolved, &meta, fmt));
                    }
                    Err(e) => {
                        output.push_str(&format!("stat: {}: {}\n", file, e));
                    }
                }
            }
            return CommandOutput::success(output);
        }

        let mut output = String::new();
        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(r) => r,
                Err(e) => {
                    output.push_str(&format!("stat: {}: {}\n", file, e));
                    continue;
                }
            };

            match std::fs::symlink_metadata(&resolved) {
                Ok(meta) => {
                    let ftype = if meta.is_dir() {
                        "directory"
                    } else if meta.is_symlink() {
                        "symbolic link"
                    } else {
                        "regular file"
                    };

                    output.push_str(&format!("  File: {}\n", resolved.display()));
                    output.push_str(&format!(
                        "  Size: {}\tBlocks: {}\tType: {}\n",
                        meta.len(),
                        meta.len() / 512 + if meta.len() % 512 != 0 { 1 } else { 0 },
                        ftype,
                    ));

                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::{MetadataExt, PermissionsExt};
                        output.push_str(&format!(
                            "Access: ({:04o})\tUid: {}\tGid: {}\n",
                            meta.permissions().mode() & 0o7777,
                            meta.uid(),
                            meta.gid(),
                        ));
                    }
                    #[cfg(not(unix))]
                    {
                        output.push_str(&format!(
                            "Access: ({})\n",
                            if meta.permissions().readonly() {
                                "read-only"
                            } else {
                                "read-write"
                            },
                        ));
                    }
                }
                Err(e) => {
                    output.push_str(&format!("stat: {}: {}\n", file, e));
                }
            }
        }

        CommandOutput::success(output)
    }
}

fn format_stat(path: &std::path::Path, meta: &std::fs::Metadata, fmt: &str) -> String {
    let ftype_str = if meta.is_dir() {
        "directory"
    } else if meta.is_symlink() {
        "symbolic link"
    } else if meta.is_file() {
        "regular file"
    } else {
        "unknown"
    };

    let (uid, gid, mode) = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            (meta.uid(), meta.gid(), meta.permissions().mode() & 0o7777)
        }
        #[cfg(not(unix))]
        {
            (0, 0, 0)
        }
    };

    let perms_human = {
        #[cfg(unix)]
        {
            crate::shell::mode_string(meta)
        }
        #[cfg(not(unix))]
        {
            if meta.permissions().readonly() {
                "r--r--r--".to_string()
            } else {
                "rw-r--r--".to_string()
            }
        }
    };

    let nlink = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            meta.nlink()
        }
        #[cfg(not(unix))]
        {
            1
        }
    };

    let (atime_secs, mtime_secs, ctime_secs) = {
        let to_secs = |t: std::io::Result<std::time::SystemTime>| -> u64 {
            t.ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0)
        };
        (
            to_secs(meta.accessed()),
            to_secs(meta.modified()),
            to_secs(meta.created()),
        )
    };

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let mut result = fmt.to_string();
    result = result.replace("%n", &filename);
    result = result.replace("%s", &format!("{}", meta.len()));
    result = result.replace("%a", &format!("{:04o}", mode));
    result = result.replace("%A", &perms_human);
    result = result.replace("%F", ftype_str);
    result = result.replace("%h", &format!("{}", nlink));
    result = result.replace("%u", &format!("{}", uid));
    result = result.replace("%g", &format!("{}", gid));
    result = result.replace("%x", &crate::shell::format_unix_time(atime_secs));
    result = result.replace("%y", &crate::shell::format_unix_time(mtime_secs));
    result = result.replace("%z", &crate::shell::format_unix_time(ctime_secs));

    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn du_walk(path: &std::path::Path, max_depth: Option<usize>, current_depth: usize) -> u64 {
    if let Some(d) = max_depth {
        if current_depth > d {
            return if path.is_file() {
                path.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
        }
    }

    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }

    let mut total: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let epath = entry.path();
            if epath.is_dir() {
                total += du_walk(&epath, max_depth, current_depth + 1);
            } else {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

fn fs_stats(path: &std::path::Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        let path_cstr = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::statvfs(path_cstr.as_ptr(), &mut stat) };
        if ret == 0 {
            let bs = stat.f_frsize as u64;
            let total = stat.f_blocks as u64 * bs;
            let avail = stat.f_bavail as u64 * bs;
            Some((total, avail))
        } else {
            None
        }
    }
    #[cfg(not(unix))]
    {
        path.metadata()
            .ok()
            .map(|_| (1024 * 1024 * 1024, 512 * 1024 * 1024))
    }
}
