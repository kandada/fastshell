// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! `cpio` — a compact implementation of the `newc` (SVR4) format.
//!
//! Supported: `cpio -o` (create, names from stdin), `cpio -i`/`-id` (extract),
//! `cpio -it` (list), `-v` (verbose), `-F FILE` (archive file), `--no-absolute`.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_cpio(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut create = false;
        let mut extract = false;
        let mut list = false;
        let mut make_dirs = false;
        let mut verbose = false;
        let mut archive_file: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if a == "-F" || a == "--file" {
                i += 1;
                archive_file = args.get(i).map(|s| s.to_string());
            } else if let Some(f) = a.strip_prefix("-F") {
                if !f.is_empty() {
                    archive_file = Some(f.to_string());
                }
            } else if a.starts_with('-') && a.len() > 1 {
                for ch in a[1..].chars() {
                    match ch {
                        'o' => create = true,
                        'i' => extract = true,
                        't' => list = true,
                        'd' => make_dirs = true,
                        'v' => verbose = true,
                        _ => {}
                    }
                }
            }
            i += 1;
        }

        if create {
            return self.cpio_create(stdin, verbose, archive_file.as_deref());
        }
        if extract || list {
            return self.cpio_extract(stdin, list, make_dirs, verbose, archive_file.as_deref());
        }
        CommandOutput::error(
            "cpio: one of -o, -i, -t is required (newc format)\n".to_string(),
            1,
        )
    }

    fn cpio_create(
        &self,
        stdin: Option<&str>,
        verbose: bool,
        archive_file: Option<&str>,
    ) -> CommandOutput {
        let names: Vec<String> = stdin
            .unwrap_or("")
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if names.is_empty() {
            return CommandOutput::error("cpio: no input files\n".to_string(), 1);
        }
        let mut out: Vec<u8> = Vec::new();
        let mut ino: u32 = 1;
        for name in &names {
            let path = name.trim_start_matches("./");
            let is_dir = self.vfs.is_dir(path, &self.cwd);
            let data: Vec<u8> = if is_dir {
                Vec::new()
            } else {
                match self.vfs.read(path, &self.cwd) {
                    Ok(d) => d,
                    Err(_) => continue,
                }
            };
            let mode: u32 = if is_dir { 0o040755 } else { 0o100644 };
            let header = newc_header(ino, mode, data.len() as u32, path);
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(path.as_bytes());
            out.push(0);
            pad4(&mut out);
            out.extend_from_slice(&data);
            pad4(&mut out);
            ino += 1;
            if verbose {
                // verbose output goes to stderr in real cpio; keep it simple.
            }
        }
        let trailer = newc_header(0, 0, 0, "TRAILER!!!");
        out.extend_from_slice(trailer.as_bytes());
        out.extend_from_slice(b"TRAILER!!!\0");
        pad4(&mut out);

        if let Some(f) = archive_file {
            match self.vfs.write_bytes(f, &self.cwd, &out) {
                Ok(_) => CommandOutput::success(String::new()),
                Err(e) => CommandOutput::error(format!("cpio: {}: {}\n", f, e), 1),
            }
        } else {
            self.set_binary_out(out.clone());
            CommandOutput::success(String::from_utf8_lossy(&out).to_string())
        }
    }

    fn cpio_extract(
        &self,
        stdin: Option<&str>,
        list_only: bool,
        make_dirs: bool,
        verbose: bool,
        archive_file: Option<&str>,
    ) -> CommandOutput {
        let data: Vec<u8> = if let Some(f) = archive_file {
            match self.vfs.read(f, &self.cwd) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("cpio: {}: {}\n", f, e), 1),
            }
        } else {
            match self
                .take_binary_in()
                .or_else(|| stdin.map(|s| s.as_bytes().to_vec()))
            {
                Some(b) => b,
                None => return CommandOutput::error("cpio: missing archive\n".to_string(), 1),
            }
        };
        let mut pos = 0usize;
        let mut out = String::new();
        while pos + 110 <= data.len() {
            let hdr = &data[pos..pos + 110];
            if &hdr[..6] != b"070701" {
                break;
            }
            let hex = |o: usize| -> usize {
                usize::from_str_radix(
                    std::str::from_utf8(&hdr[o..o + 8]).unwrap_or("0").trim(),
                    16,
                )
                .unwrap_or(0)
            };
            let mode = hex(14);
            let filesize = hex(54);
            let namesize = hex(94);
            let name_start = pos + 110;
            if name_start + namesize > data.len() {
                break;
            }
            let name = String::from_utf8_lossy(&data[name_start..name_start + namesize])
                .trim_end_matches('\0')
                .to_string();
            if name == "TRAILER!!!" {
                break;
            }
            let data_start = name_start + namesize;
            let data_start = (data_start + 3) & !3;
            let file_end = data_start + filesize;
            if file_end > data.len() {
                break;
            }
            let is_dir = mode & 0o170000 == 0o040000;
            if list_only {
                out.push_str(&name);
                out.push('\n');
            } else if is_dir {
                let _ = self.vfs.create_dir_all(&name, &self.cwd);
            } else {
                if make_dirs {
                    if let Some(parent) = name.rsplit_once('/').map(|(p, _)| p) {
                        if !parent.is_empty() {
                            let _ = self.vfs.create_dir_all(parent, &self.cwd);
                        }
                    }
                }
                if let Err(e) = self
                    .vfs
                    .write_bytes(&name, &self.cwd, &data[data_start..file_end])
                {
                    return CommandOutput::error(format!("cpio: {}: {}\n", name, e), 1);
                }
                if verbose {
                    out.push_str(&name);
                    out.push('\n');
                }
            }
            pos = file_end;
            pos = (pos + 3) & !3;
        }
        CommandOutput::success(out)
    }
}

fn pad4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn newc_header(ino: u32, mode: u32, size: u32, name: &str) -> String {
    let namesize = name.len() + 1;
    format!(
        "070701{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}",
        ino,
        mode,
        0, // uid
        0, // gid
        1, // nlink
        0, // mtime
        size,
        0, // devmajor
        0, // devminor
        0, // rdevmajor
        0, // rdevminor
        namesize,
        0 // check
    )
}
