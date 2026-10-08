// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::io::{Read, Write};

impl Shell {
    pub fn cmd_zip(&self, args: &[&str]) -> CommandOutput {
        let mut zip_name = String::new();
        let mut paths: Vec<String> = Vec::new();
        let mut recursive = false;
        let mut quiet = false;
        let mut junk_paths = false;
        let mut no_dir_entries = false;
        let mut excludes: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if a == "--" {
                i += 1;
                while i < args.len() {
                    if zip_name.is_empty() {
                        zip_name = args[i].to_string();
                    } else {
                        paths.push(args[i].to_string());
                    }
                    i += 1;
                }
                break;
            }
            if a == "-x" {
                i += 1;
                while i < args.len() && !args[i].starts_with('-') {
                    excludes.push(args[i].to_string());
                    i += 1;
                }
                continue;
            }
            if a.starts_with('-') && a.len() > 1 {
                for c in a[1..].chars() {
                    match c {
                        'r' | 'R' => recursive = true,
                        'q' => quiet = true,
                        'j' => junk_paths = true,
                        'D' => no_dir_entries = true,
                        // Accepted-but-ignored flags (levels, attrs, symlink mode…).
                        '0'..='9'
                        | 'X'
                        | 'y'
                        | 'o'
                        | 'S'
                        | 'm'
                        | 'e'
                        | 'z'
                        | 'g'
                        | 'n'
                        | 'C'
                        | 'u'
                        | 'f'
                        | 'F'
                        | 'L'
                        | 'U'
                        | 'k' => {}
                        _ => {}
                    }
                }
                i += 1;
                continue;
            }
            if zip_name.is_empty() {
                zip_name = a.to_string();
            } else {
                paths.push(a.to_string());
            }
            i += 1;
        }

        if zip_name.is_empty() || paths.is_empty() {
            return CommandOutput::error(
                "zip: usage: zip [-r] [-q] [-j] [-x PATTERN] archive.zip file1 [file2 ...]\n"
                    .to_string(),
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

        // Collect entries first: (archive_name, real_path, is_dir).
        let mut entries: Vec<(String, std::path::PathBuf, bool)> = Vec::new();
        let mut warnings = String::new();
        for p in &paths {
            let real = match self.vfs.resolve(p, &self.cwd) {
                Ok(r) => r,
                Err(e) => return CommandOutput::error(format!("zip: {}: {}\n", p, e), 1),
            };
            if !real.exists() {
                return CommandOutput::error(format!("zip: {}: No such file or directory\n", p), 1);
            }
            let base = p.trim_start_matches("./").trim_end_matches('/').to_string();
            if real.is_dir() {
                if !recursive {
                    warnings.push_str(&format!(
                        "zip warning: {} is a directory (use -r to recurse)\n",
                        p
                    ));
                    continue;
                }
                collect_dir(&real, &base, no_dir_entries, &mut entries);
            } else {
                entries.push((base, real, false));
            }
        }

        let mut out = warnings;
        for (name, real, is_dir) in entries {
            if is_dir && junk_paths {
                // `-j` flattens paths; directory entries carry no content.
                continue;
            }
            let entry_name = if junk_paths {
                real.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
            let entry_name = entry_name.trim_start_matches('/').to_string();
            if entry_name.is_empty() {
                continue;
            }
            if excludes.iter().any(|pat| wildcard_match(pat, &entry_name)) {
                continue;
            }
            if is_dir {
                let dir_name = format!("{}/", entry_name.trim_end_matches('/'));
                if let Err(e) = zip_writer.add_directory(dir_name.clone(), options) {
                    return CommandOutput::error(format!("zip: {}: {}\n", dir_name, e), 1);
                }
                if !quiet {
                    out.push_str(&format!("   adding: {}\n", dir_name));
                }
            } else {
                let data = match std::fs::read(&real) {
                    Ok(d) => d,
                    Err(e) => {
                        return CommandOutput::error(format!("zip: {}: {}\n", entry_name, e), 1)
                    }
                };
                if let Err(e) = zip_writer.start_file(entry_name.clone(), options) {
                    return CommandOutput::error(format!("zip: {}: {}\n", entry_name, e), 1);
                }
                if let Err(e) = zip_writer.write_all(&data) {
                    return CommandOutput::error(
                        format!("zip: {}: write error: {}\n", entry_name, e),
                        1,
                    );
                }
                if !quiet {
                    out.push_str(&format!("  adding: {}\n", entry_name));
                }
            }
        }

        if let Err(e) = zip_writer.finish() {
            return CommandOutput::error(format!("zip: finish error: {}\n", e), 1);
        }

        CommandOutput::success(out)
    }

    pub fn cmd_unzip(&self, args: &[&str]) -> CommandOutput {
        let mut zip_name = String::new();
        // `-d DIR` sets the extraction destination (default: cwd). Keep the raw
        // value and resolve it against the shell cwd *after* parsing so both the
        // mkdir and the file writes agree on the same absolute VFS path.
        let mut dest_raw: Option<String> = None;
        let mut list = false;
        let mut to_stdout = false;
        let mut test = false;
        let mut quiet = false;
        let mut junk_paths = false;

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            match arg {
                "-d" => {
                    if i + 1 < args.len() {
                        i += 1;
                        dest_raw = Some(args[i].to_string());
                    }
                }
                "-l" | "--list" => list = true,
                "-p" => to_stdout = true,
                "-t" | "--test" => test = true,
                "-q" | "--quiet" => quiet = true,
                "-j" | "--junk-paths" => junk_paths = true,
                "-o" | "-n" | "-f" | "-v" | "-u" | "-C" | "-x" => {}
                a if a.starts_with("-d") && a.len() > 2 => dest_raw = Some(a[2..].to_string()),
                a if a.starts_with('-') => {}
                a if zip_name.is_empty() => zip_name = a.to_string(),
                _ => {}
            }
            i += 1;
        }

        let dest = match dest_raw {
            Some(d) => absolute_dir(&self.cwd, &d),
            None => self.cwd.clone(),
        };

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
        let count = archive.len();

        // -l: list contents.
        if list {
            let mut out = format!("Archive:  {}\n", zip_name);
            out.push_str("  Length      Name\n---------  ----\n");
            let mut total: u64 = 0;
            for idx in 0..count {
                if let Ok(e) = archive.by_index(idx) {
                    total += e.size();
                    out.push_str(&format!("{:>9}  {}\n", e.size(), e.name()));
                }
            }
            out.push_str(&format!("---------  ----\n{:>9}  {} files\n", total, count));
            return CommandOutput::success(out);
        }

        // -t: test archive integrity.
        if test {
            let mut bad = String::new();
            for idx in 0..count {
                match archive.by_index(idx) {
                    Ok(mut e) => {
                        let mut buf = Vec::new();
                        if let Err(er) = e.read_to_end(&mut buf) {
                            bad.push_str(&format!("unzip: {}: {}\n", e.name(), er));
                        }
                    }
                    Err(er) => bad.push_str(&format!("unzip: entry error: {}\n", er)),
                }
            }
            return if bad.is_empty() {
                CommandOutput::success(format!(
                    "No errors detected in compressed data of {}.\n",
                    zip_name
                ))
            } else {
                CommandOutput::error(bad, 1)
            };
        }

        // -p: write file contents to stdout (binary-safe via binary_out).
        if to_stdout {
            let mut bytes: Vec<u8> = Vec::new();
            for idx in 0..count {
                if let Ok(mut e) = archive.by_index(idx) {
                    if e.is_dir() {
                        continue;
                    }
                    let _ = e.read_to_end(&mut bytes);
                }
            }
            if !bytes.is_empty() {
                self.set_binary_out(bytes.clone());
            }
            return CommandOutput::success(String::from_utf8_lossy(&bytes).to_string());
        }

        // Extraction.
        if dest != self.cwd {
            let _ = self.vfs.create_dir_all(&dest, &self.cwd);
        }
        let mut output = String::new();
        for idx in 0..count {
            let mut entry = match archive.by_index(idx) {
                Ok(e) => e,
                Err(e) => {
                    output.push_str(&format!("unzip: entry error: {}\n", e));
                    continue;
                }
            };
            let raw = entry.name().to_string();
            // `-j` strips directories; otherwise sanitize to prevent zip-slip
            // (`../` or absolute paths escaping the destination directory).
            let name: String = if junk_paths {
                raw.rsplit('/').next().unwrap_or(&raw).to_string()
            } else {
                raw.split('/')
                    .filter(|c| !c.is_empty() && *c != "." && *c != "..")
                    .collect::<Vec<_>>()
                    .join("/")
            };
            if name.is_empty() {
                continue;
            }
            if entry.is_dir() {
                if let Err(e) = self.vfs.create_dir_all(&name, &dest) {
                    output.push_str(&format!("unzip: {}: {}\n", name, e));
                }
                continue;
            }
            let mut content = Vec::new();
            if let Err(e) = entry.read_to_end(&mut content) {
                output.push_str(&format!("unzip: {}: read error: {}\n", name, e));
                continue;
            }
            if let Err(e) = self.vfs.write_bytes(&name, &dest, &content) {
                output.push_str(&format!("unzip: {}: {}\n", name, e));
            } else if !quiet {
                output.push_str(&format!("  inflating: {}\n", name));
            }
        }
        if quiet {
            return CommandOutput::success(String::new());
        }
        let mut stdout = format!("Archive: {}\n", zip_name);
        stdout.push_str(&output);
        CommandOutput::success(stdout)
    }
}

/// Recursively collect a directory tree into `entries` as
/// `(archive_name, real_path, is_dir)`. `base` is the archive-relative prefix.
fn collect_dir(
    dir: &std::path::Path,
    base: &str,
    no_dir_entries: bool,
    entries: &mut Vec<(String, std::path::PathBuf, bool)>,
) {
    if !no_dir_entries && !base.is_empty() && base != "." {
        entries.push((base.to_string(), dir.to_path_buf(), true));
    }
    let mut children: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    children.sort_by_key(|e| e.file_name());
    for child in children {
        let name = child.file_name().to_string_lossy().to_string();
        let child_base = if base.is_empty() || base == "." {
            name.clone()
        } else {
            format!("{}/{}", base.trim_end_matches('/'), name)
        };
        let ft = match child.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            collect_dir(&child.path(), &child_base, no_dir_entries, entries);
        } else if ft.is_file() {
            entries.push((child_base, child.path(), false));
        }
    }
}

/// Minimal shell-style wildcard matcher (`*`, `?`) used by `zip -x`.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Resolve a `-d DIR` destination against the shell cwd into an absolute VFS
/// path (`.`/`..` collapsed). Absolute inputs pass through unchanged.
fn absolute_dir(cwd: &str, dir: &str) -> String {
    if dir.starts_with('/') {
        return dir.to_string();
    }
    let mut parts: Vec<&str> = cwd.split('/').filter(|s| !s.is_empty()).collect();
    for seg in dir.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}
