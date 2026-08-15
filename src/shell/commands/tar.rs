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
}

fn compression_from_ext(name: &str) -> TarCompression {
    if name.ends_with(".gz") || name.ends_with(".tgz") {
        TarCompression::Gzip
    } else if name.ends_with(".bz2") || name.ends_with(".tbz") || name.ends_with(".tbz2") {
        TarCompression::Bzip2
    } else if name.ends_with(".xz") || name.ends_with(".txz") {
        TarCompression::Xz
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

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                for ch in arg.chars().skip(1) {
                    match ch {
                        'c' => create = true,
                        'x' => extract = true,
                        't' => list = true,
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
                        'v' => {}
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
                }
            } else {
                operands.push(arg.to_string());
            }
            i += 1;
        }

        if !create && !extract && !list {
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
        }

        let cwd = directory.unwrap_or_else(|| self.cwd.clone());

        if create {
            self.tar_create(&archive, &operands, &cwd, compression)
        } else if extract {
            self.tar_extract(&archive, &cwd, compression)
        } else if list {
            self.tar_list(&archive, &cwd, compression)
        } else {
            CommandOutput::error("tar: unknown mode\n".to_string(), 1)
        }
    }

    fn tar_create(&self, archive: &str, files: &[String], cwd: &str, compression: TarCompression) -> CommandOutput {
        let entries = if files.is_empty() {
            match self.vfs.list_dir(".", cwd) {
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
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, cwd) {
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
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, cwd) {
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
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, cwd) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                let xz = builder.into_inner().map_err(|e| format!("tar: {}\n", e));
                if let Err(e) = xz {
                    return CommandOutput::error(e, 1);
                }
                let _ = xz.unwrap().finish();
            }
            TarCompression::None => {
                let mut builder = tar::Builder::new(&mut buf);
                if let Err(e) = self.tar_append_entries(&mut builder, &entries, cwd) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                builder
                    .into_inner()
                    .map_err(|e| CommandOutput::error(format!("tar: {}\n", e), 1))
                    .unwrap();
            }
        }

        if let Err(e) = self.vfs.write_bytes(archive, cwd, &buf) {
            return CommandOutput::error(format!("tar: {}: {}\n", archive, e), 1);
        }

        CommandOutput::success(String::new())
    }

    fn tar_append_entries<W: std::io::Write>(
        &self,
        builder: &mut tar::Builder<W>,
        entries: &[String],
        cwd: &str,
    ) -> Result<(), String> {
        for entry in entries {
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
                    if sub_resolved.is_dir() {
                        self.tar_append_entries(builder, &[sub_path.clone()], cwd)?;
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

    fn tar_extract(&self, archive: &str, cwd: &str, compression: TarCompression) -> CommandOutput {
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

        for entry_result in entries {
            let mut entry = match entry_result {
                Ok(e) => e,
                Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
            };
            let path = match entry.path() {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
            };
            let path_str = path.to_string_lossy().to_string();

            if entry.header().entry_type() == tar::EntryType::Directory {
                let _ = self.vfs.create_dir_all(&path_str, cwd);
            } else {
                let mut file_data = Vec::new();
                if let Err(e) = entry.read_to_end(&mut file_data) {
                    return CommandOutput::error(format!("tar: {}\n", e), 1);
                }
                if let Err(e) = self.vfs.write_bytes(&path_str, cwd, &file_data) {
                    return CommandOutput::error(format!("tar: {}: {}\n", path_str, e), 1);
                }
            }
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
        let dir = std::env::temp_dir().join(format!("fastshell_test_{}_{}", std::process::id(), uuid::Uuid::new_v4()));
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
