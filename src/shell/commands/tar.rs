use crate::shell::{Shell, CommandOutput};
use std::io::Read;

impl Shell {
    pub fn cmd_tar(&self, args: &[&str]) -> CommandOutput {
        let mut create = false;
        let mut extract = false;
        let mut list = false;
        let mut gzip = false;
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
                        'z' => gzip = true,
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
                        _ => {}
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
                    gzip = true;
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

        let cwd = directory.unwrap_or_else(|| self.cwd.clone());

        if create {
            self.tar_create(&archive, &operands, &cwd, gzip)
        } else if extract {
            self.tar_extract(&archive, &cwd, gzip)
        } else if list {
            self.tar_list(&archive, &cwd, gzip)
        } else {
            CommandOutput::error("tar: unknown mode\n".to_string(), 1)
        }
    }

    fn tar_create(
        &self,
        archive: &str,
        files: &[String],
        cwd: &str,
        gzip: bool,
    ) -> CommandOutput {
        let entries = if files.is_empty() {
            match self.vfs.list_dir(".", cwd) {
                Ok(e) => e.iter().map(|e| e.name.clone()).collect(),
                Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
            }
        } else {
            files.to_vec()
        };

        let mut buf = Vec::new();

        if gzip {
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
        } else {
            let mut builder = tar::Builder::new(&mut buf);
            if let Err(e) = self.tar_append_entries(&mut builder, &entries, cwd) {
                return CommandOutput::error(format!("tar: {}\n", e), 1);
            }
            builder
                .into_inner()
                .map_err(|e| CommandOutput::error(format!("tar: {}\n", e), 1))
                .unwrap();
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
            let resolved = self
                .vfs
                .resolve(entry, cwd)
                .map_err(|e| e.to_string())?;

            if resolved.is_dir() {
                let sub_entries = self
                    .vfs
                    .list_dir(entry, cwd)
                    .map_err(|e| e.to_string())?;
                let sub_names: Vec<String> =
                    sub_entries.iter().map(|e| e.name.clone()).collect();
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
                        let data = self
                            .vfs
                            .read(&sub_path, cwd)
                            .map_err(|e| e.to_string())?;
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

    fn tar_extract(&self, archive: &str, cwd: &str, gzip: bool) -> CommandOutput {
        let data = match self.vfs.read(archive, cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("tar: {}: {}\n", archive, e), 1),
        };

        let reader: Box<dyn std::io::Read> = if gzip {
            Box::new(flate2::read::GzDecoder::new(&data[..]))
        } else {
            Box::new(&data[..])
        };

        let mut archive_reader = match tar::Archive::new(reader).entries() {
            Ok(_) => tar::Archive::new(if gzip {
                Box::new(flate2::read::GzDecoder::new(&data[..])) as Box<dyn std::io::Read>
            } else {
                Box::new(&data[..])
            }),
            Err(e) => return CommandOutput::error(format!("tar: {}\n", e), 1),
        };

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

    fn tar_list(&self, archive: &str, cwd: &str, gzip: bool) -> CommandOutput {
        let data = match self.vfs.read(archive, cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("tar: {}: {}\n", archive, e), 1),
        };

        let reader: Box<dyn std::io::Read> = if gzip {
            Box::new(flate2::read::GzDecoder::new(&data[..]))
        } else {
            Box::new(&data[..])
        };

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
