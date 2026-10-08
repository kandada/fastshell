// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TarCompression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

fn compression_from_ext(name: &str) -> TarCompression {
    if name.ends_with(".gz") || name.ends_with(".tgz") {
        TarCompression::Gzip
    } else if name.ends_with(".bz2") || name.ends_with(".tbz") || name.ends_with(".tbz2") {
        TarCompression::Bzip2
    } else if name.ends_with(".xz") || name.ends_with(".txz") {
        TarCompression::Xz
    } else if name.ends_with(".zst") || name.ends_with(".tzst") {
        TarCompression::Zstd
    } else {
        TarCompression::None
    }
}

/// Detect compression from the archive's magic bytes (GNU tar auto-detects).
fn sniff_compression(data: &[u8]) -> TarCompression {
    if data.starts_with(&[0x1f, 0x8b]) {
        TarCompression::Gzip
    } else if data.starts_with(b"BZh") {
        TarCompression::Bzip2
    } else if data.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        TarCompression::Xz
    } else if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        TarCompression::Zstd
    } else {
        TarCompression::None
    }
}

/// Wrap a byte reader with the matching decompressor for `c`.
fn decompress_reader<'a>(c: TarCompression, data: &'a [u8]) -> Box<dyn Read + 'a> {
    match c {
        TarCompression::Gzip => Box::new(flate2::read::GzDecoder::new(data)),
        TarCompression::Bzip2 => Box::new(bzip2::read::BzDecoder::new(data)),
        TarCompression::Xz => Box::new(liblzma::read::XzDecoder::new(data)),
        TarCompression::Zstd => match zstd::stream::read::Decoder::new(data) {
            Ok(d) => Box::new(d),
            Err(_) => Box::new(data),
        },
        TarCompression::None => Box::new(data),
    }
}

const TAR_HELP_TEXT: &str = "\
tar: tape archiver
Usage: tar [OPTIONS] [FILE...]
Options:
  -c, --create     Create a new archive
  -x, --extract    Extract files from archive
  -t, --list       List archive contents
  -z, --gzip       Filter archive through gzip
  -f, --file FILE  Use archive file
  -C, --directory  Change to directory
  -v               Verbose (accepted, ignored)
  -h, --help       Show this help message
";

impl Shell {
    pub fn cmd_tar(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(TAR_HELP_TEXT.to_string());
        }
        let mut create = false;
        let mut extract = false;
        let mut list = false;
        let mut compression = TarCompression::None;
        let mut auto_compress = false;
        let mut file: Option<String> = None;
        let mut directory: Option<String> = None;
        let mut operands = Vec::new();
        let mut excludes: Vec<String> = Vec::new();
        let mut append = false;
        let mut verbose = false;
        let mut to_stdout = false;
        let mut strip_components: usize = 0;

        let mut i = 0;
        // Traditional (dash-less) option cluster as the first arg, e.g.
        // `tar cf a.tar f` — bash accepts this legacy form.
        let first_is_cluster = args
            .first()
            .map(|a| {
                !a.starts_with('-')
                    && a.len() > 1
                    && a.chars().all(|c| "cxtvzjJfaCr".contains(c))
                    && a.chars().any(|c| matches!(c, 'c' | 'x' | 't' | 'r'))
            })
            .unwrap_or(false);
        while i < args.len() {
            let arg = args[i];
            let as_short = (arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--"))
                || (i == 0 && first_is_cluster);
            if as_short {
                for ch in arg.trim_start_matches('-').chars() {
                    match ch {
                        'c' => create = true,
                        'x' => extract = true,
                        't' => list = true,
                        'r' => append = true,
                        'z' => compression = TarCompression::Gzip,
                        'j' => compression = TarCompression::Bzip2,
                        'J' => compression = TarCompression::Xz,
                        'a' => auto_compress = true,
                        'f' => {
                            if i + 1 < args.len() {
                                i += 1;
                                file = Some(args[i].to_string());
                            }
                        }
                        'C' => {
                            if i + 1 < args.len() {
                                i += 1;
                                directory = Some(args[i].to_string());
                            }
                        }
                        'v' => verbose = true,
                        'O' => to_stdout = true,
                        _ => crate::warn!("tar: warning: unsupported option '-{}'", ch),
                    }
                }
            } else if arg.starts_with("--") {
                if arg == "--create" {
                    create = true;
                } else if arg == "--extract" {
                    extract = true;
                } else if arg == "--list" {
                    list = true;
                } else if arg == "--append" {
                    append = true;
                } else if arg == "--verbose" {
                    verbose = true;
                } else if arg == "--to-stdout" {
                    to_stdout = true;
                } else if arg == "--gzip" {
                    compression = TarCompression::Gzip;
                } else if arg == "--bzip2" {
                    compression = TarCompression::Bzip2;
                } else if arg == "--xz" {
                    compression = TarCompression::Xz;
                } else if arg == "--auto-compress" {
                    auto_compress = true;
                } else if arg == "--file" {
                    if i + 1 < args.len() {
                        i += 1;
                        file = Some(args[i].to_string());
                    }
                } else if arg == "--directory" {
                    if i + 1 < args.len() {
                        i += 1;
                        directory = Some(args[i].to_string());
                    }
                } else if let Some(d) = arg.strip_prefix("--directory=") {
                    directory = Some(d.to_string());
                } else if arg == "--strip-components" {
                    if i + 1 < args.len() {
                        i += 1;
                        strip_components = args[i].parse().unwrap_or(0);
                    }
                } else if let Some(n) = arg.strip_prefix("--strip-components=") {
                    strip_components = n.parse().unwrap_or(0);
                } else if arg == "--exclude" {
                    if i + 1 < args.len() {
                        i += 1;
                        excludes.push(args[i].to_string());
                    }
                } else if let Some(pat) = arg.strip_prefix("--exclude=") {
                    excludes.push(pat.to_string());
                }
            } else {
                operands.push(arg.to_string());
            }
            i += 1;
        }

        if !create && !extract && !list && !append {
            return CommandOutput::error(
                "tar: you must specify one of -c, -x, -t\n".to_string(),
                1,
            );
        }

        let archive = match file {
            Some(ref f) => f.clone(),
            None => return CommandOutput::error("tar: no archive specified (-f)\n".to_string(), 1),
        };

        if auto_compress {
            compression = compression_from_ext(&archive);
        } else if !create && compression == TarCompression::None {
            // Auto-detect for extract/list (like GNU tar): extension then magic.
            let by_ext = compression_from_ext(&archive);
            compression = if by_ext != TarCompression::None {
                by_ext
            } else if let Ok(data) = self.vfs.read(&archive, &self.cwd) {
                sniff_compression(&data)
            } else {
                TarCompression::None
            };
        }

        // `-C dir` changes the *source base* (create) / *destination* (extract);
        // the archive path itself is always relative to the shell's cwd (GNU tar).
        // A relative `-C` dir must be resolved against the shell cwd (the VFS
        // otherwise treats it as sandbox-root-relative, e.g. `-C arc` → `/arc`).
        let archive_cwd = self.cwd.clone();
        let dir = directory
            .as_deref()
            .map(|d| absolute_dir(&self.cwd, d))
            .unwrap_or_else(|| self.cwd.clone());

        if create {
            let mut out = self.tar_create(
                &archive,
                &operands,
                &dir,
                &archive_cwd,
                compression,
                &excludes,
            );
            // `-v`: list the archive contents after creating (like `tar -cv`).
            if verbose && out.exit_code == 0 {
                let listing = self.tar_list(&archive, &archive_cwd, compression);
                if listing.exit_code == 0 {
                    out.stdout = listing.stdout;
                }
            }
            out
        } else if append {
            self.tar_append(&archive, &operands, &dir, &archive_cwd, &excludes)
        } else if extract {
            let mut out = self.tar_extract(
                &archive,
                &archive_cwd,
                &dir,
                compression,
                strip_components,
                to_stdout,
                &operands,
            );
            if verbose && out.exit_code == 0 {
                let listing = self.tar_list(&archive, &archive_cwd, compression);
                if listing.exit_code == 0 {
                    out.stdout = listing.stdout;
                }
            }
            out
        } else if list {
            self.tar_list(&archive, &archive_cwd, compression)
        } else {
            CommandOutput::error("tar: unknown mode\n".to_string(), 1)
        }
    }

    fn tar_create(
        &self,
        archive: &str,
        files: &[String],
        src_cwd: &str,
        archive_cwd: &str,
        compression: TarCompression,
        excludes: &[String],
    ) -> CommandOutput {
        let entries = if files.is_empty() {
            match self.vfs.list_dir(".", src_cwd) {
                Ok(e) => e.iter().map(|e| e.name.clone()).collect(),
                Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
            }
        } else {
            files.to_vec()
        };

        let mut buf = Vec::new();

        match compression {
            TarCompression::Gzip => {
                let gz = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
                let mut builder = tar::Builder::new(gz);
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, src_cwd, excludes) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                let gz = builder.into_inner().map_err(|e| format!("tar: {}\n", e));
                if let Err(e) = gz {
                    return CommandOutput::error(e, 1);
                }
                let _ = gz.unwrap().finish();
            }
            TarCompression::Bzip2 => {
                let bz = bzip2::write::BzEncoder::new(&mut buf, bzip2::Compression::default());
                let mut builder = tar::Builder::new(bz);
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, src_cwd, excludes) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                let bz = builder.into_inner().map_err(|e| format!("tar: {}\n", e));
                if let Err(e) = bz {
                    return CommandOutput::error(e, 1);
                }
                let _ = bz.unwrap().finish();
            }
            TarCompression::Xz => {
                let xz = liblzma::write::XzEncoder::new(&mut buf, 6);
                let mut builder = tar::Builder::new(xz);
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, src_cwd, excludes) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                let xz = builder.into_inner().map_err(|e| format!("tar: {}\n", e));
                if let Err(e) = xz {
                    return CommandOutput::error(e, 1);
                }
                let _ = xz.unwrap().finish();
            }
            TarCompression::Zstd => {
                let z = match zstd::stream::write::Encoder::new(&mut buf, 3) {
                    Ok(z) => z,
                    Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
                };
                let mut builder = tar::Builder::new(z);
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, src_cwd, excludes) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                let z = builder.into_inner().map_err(|e| format!("tar: {}\n", e));
                if let Err(e) = z {
                    return CommandOutput::error(e, 1);
                }
                let _ = z.unwrap().finish();
            }
            TarCompression::None => {
                let mut builder = tar::Builder::new(&mut buf);
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, src_cwd, excludes) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                builder
                    .into_inner()
                    .map_err(|e| CommandOutput::error(format!("tar: {}\n", e), 1))
                    .unwrap();
            }
        }

        if let Err(e) = self.vfs.write_bytes(archive, archive_cwd, &buf) {
            return CommandOutput::error(format!("tar: {}: {}\n", archive, e), 1);
        }

        CommandOutput::success(String::new())
    }

    /// `tar -r` — append files to an existing (uncompressed) archive by
    /// re-emitting its entries followed by the new ones.
    fn tar_append(
        &self,
        archive: &str,
        files: &[String],
        src_cwd: &str,
        archive_cwd: &str,
        excludes: &[String],
    ) -> CommandOutput {
        if files.is_empty() {
            return CommandOutput::error("tar: no files to append\n".to_string(), 1);
        }
        let existing = match self.vfs.read(archive, archive_cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("tar: {}: {}\n", archive, e), 1),
        };
        let mut buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut buf);
            {
                let mut ar = tar::Archive::new(&existing[..]);
                let entries = match ar.entries() {
                    Ok(e) => e,
                    Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
                };
                for entry_result in entries {
                    let mut e = match entry_result {
                        Ok(e) => e,
                        Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
                    };
                    let path = match e.path() {
                        Ok(p) => p.to_path_buf(),
                        Err(_) => continue,
                    };
                    let mut data = Vec::new();
                    if std::io::Read::read_to_end(&mut e, &mut data).is_err() {
                        continue;
                    }
                    let mut header = tar::Header::new_gnu();
                    header.set_size(data.len() as u64);
                    header.set_mode(0o644);
                    if let Err(er) = builder.append_data(&mut header, &path, &data[..]) {
                        return CommandOutput::error(format!("tar: {}\n", er), 1);
                    }
                }
            }
            if let Err(e) = self.tar_append_entries(&mut builder, files, src_cwd, excludes) {
                return CommandOutput::error(format!("tar: {}\n", e), 1);
            }
            if let Err(e) = builder.finish() {
                return CommandOutput::error(format!("tar: {}\n", e), 1);
            }
        }
        if let Err(e) = self.vfs.write_bytes(archive, archive_cwd, &buf) {
            return CommandOutput::error(format!("tar: {}: {}\n", archive, e), 1);
        }
        CommandOutput::success(String::new())
    }

    fn tar_append_entries<W: std::io::Write>(
        &self,
        builder: &mut tar::Builder<W>,
        entries: &[String],
        cwd: &str,
        excludes: &[String],
    ) -> Result<(), String> {
        for entry in entries {
            if tar_excluded(entry, excludes) {
                continue;
            }
            let resolved = self.vfs.resolve(entry, cwd).map_err(|e| e.to_string())?;

            if resolved.is_dir() {
                let sub_entries = self.vfs.list_dir(entry, cwd).map_err(|e| e.to_string())?;
                let sub_names: Vec<String> = sub_entries.iter().map(|e| e.name.clone()).collect();
                let _sub_cwd = format!("{}/{}", cwd.trim_end_matches('/'), entry);

                for sub in &sub_names {
                    let sub_path = format!("{}/{}", entry, sub);
                    let sub_resolved = self
                        .vfs
                        .resolve(&sub_path, cwd)
                        .map_err(|e| e.to_string())?;
                    if tar_excluded(&sub_path, excludes) {
                        continue;
                    }
                    if sub_resolved.is_dir() {
                        self.tar_append_entries(builder, &[sub_path.clone()], cwd, excludes)?;
                    } else {
                        let data = self.vfs.read(&sub_path, cwd).map_err(|e| e.to_string())?;
                        let mut header = tar::Header::new_gnu();
                        header.set_size(data.len() as u64);
                        header.set_mode(0o644);
                        builder
                            .append_data(&mut header, &sub_path, &data[..])
                            .map_err(|e| e.to_string())?;
                    }
                }
            } else {
                let data = self.vfs.read(entry, cwd).map_err(|e| e.to_string())?;
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                builder
                    .append_data(&mut header, entry, &data[..])
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    fn tar_extract(
        &self,
        archive: &str,
        archive_cwd: &str,
        dest: &str,
        compression: TarCompression,
        strip_components: usize,
        to_stdout: bool,
        members: &[String],
    ) -> CommandOutput {
        // Ensure the destination directory exists so `tar -C DIR -x` always
        // lands the files under DIR (create it when missing).
        if !to_stdout {
            let _ = self.vfs.create_dir_all(dest, &self.cwd);
        }
        let data = match self.vfs.read(archive, archive_cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("tar: {}: {}\n", archive, e), 1),
        };

        let reader: Box<dyn std::io::Read> = decompress_reader(compression, &data);

        let mut archive_reader = tar::Archive::new(reader);
        let entries = match archive_reader.entries() {
            Ok(e) => e,
            Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
        };

        let mut stdout_buf: Vec<u8> = Vec::new();
        for entry_result in entries {
            let mut entry = match entry_result {
                Ok(e) => e,
                Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
            };
            let path = match entry.path() {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
            };
            let raw_path = path.to_string_lossy().to_string();
            // Restrict to the named members when operands are given (GNU).
            if !members.is_empty() && !tar_member_selected(&raw_path, members) {
                continue;
            }
            // `--strip-components=N`: drop the first N path components.
            let path_str = strip_leading_components(&raw_path, strip_components);
            if path_str.is_empty() && !to_stdout {
                continue;
            }

            if entry.header().entry_type() == tar::EntryType::Directory {
                if !to_stdout {
                    let _ = self.vfs.create_dir_all(&path_str, dest);
                }
            } else {
                let mut file_data = Vec::new();
                if let Err(e) = entry.read_to_end(&mut file_data) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                if to_stdout {
                    self.set_binary_out(file_data.clone());
                    stdout_buf.extend_from_slice(&file_data);
                } else if let Err(e) = self.vfs.write_bytes(&path_str, dest, &file_data) {
                    return CommandOutput::error(format!("tar: {}: {}\n", path_str, e), 1);
                }
            }
        }

        if to_stdout {
            return CommandOutput::success(String::from_utf8_lossy(&stdout_buf).to_string());
        }
        CommandOutput::success(String::new())
    }

    fn tar_list(&self, archive: &str, cwd: &str, compression: TarCompression) -> CommandOutput {
        let data = match self.vfs.read(archive, cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("tar: {}: {}\n", archive, e), 1),
        };

        let reader: Box<dyn std::io::Read> = decompress_reader(compression, &data);

        let mut archive_reader = tar::Archive::new(reader);
        let entries = match archive_reader.entries() {
            Ok(e) => e,
            Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
        };

        let mut output = String::new();

        for entry_result in entries {
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    output.push_str(&format!("tar: {}\n", e));
                    break;
                }
            };
            if let Ok(path) = entry.path() {
                output.push_str(&path.to_string_lossy());
                output.push('\n');
            }
        }

        CommandOutput::success(output)
    }
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;

    fn mk_shell() -> Shell {
        use std::fs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_tar_help() {
        let mut shell = mk_shell();
        let out = shell.execute("tar", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_tar_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("tar", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}

/// True if `path` matches any `--exclude` pattern (against the full relative
/// path or its basename, like GNU tar).
fn tar_excluded(path: &str, excludes: &[String]) -> bool {
    if excludes.is_empty() {
        return false;
    }
    let base = path.rsplit('/').next().unwrap_or(path);
    excludes
        .iter()
        .any(|pat| tar_glob(pat, path) || tar_glob(pat, base))
}

fn tar_glob(pattern: &str, text: &str) -> bool {
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

/// Resolve a `-C` directory against the shell cwd into an absolute VFS path.
/// Absolute inputs are returned unchanged; relative inputs are joined to `cwd`
/// with `.`/`..` collapsed (so `tar -C arc …` works from any cwd).
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

/// Does archive member `entry` match one of the requested `members`? A member
/// selects itself and everything under it (GNU semantics), ignoring a leading
/// `./` on either side.
fn tar_member_selected(entry: &str, members: &[String]) -> bool {
    let e = entry.trim_start_matches("./");
    members.iter().any(|m| {
        let m = m.trim_start_matches("./").trim_end_matches('/');
        !m.is_empty() && (e == m || e.starts_with(&format!("{m}/")))
    })
}

/// Drop the first `n` path components (`--strip-components=N`). Empty and `.`
/// components are ignored when counting, matching GNU tar.
fn strip_leading_components(path: &str, n: usize) -> String {
    if n == 0 {
        return path.to_string();
    }
    path.split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .skip(n)
        .collect::<Vec<_>>()
        .join("/")
}
