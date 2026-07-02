// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use md5::Md5;
use sha2::{Digest, Sha256, Sha512};

impl Shell {
    pub fn cmd_sha256sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_sha::<Sha256>(self, args, stdin, "sha256sum")
    }

    pub fn cmd_sha512sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_sha::<Sha512>(self, args, stdin, "sha512sum")
    }

    pub fn cmd_md5sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_md5(self, args, stdin)
    }
}

fn cmd_hashsum_sha<H: Digest>(
    shell: &Shell,
    args: &[&str],
    stdin: Option<&str>,
    name: &str,
) -> CommandOutput {
    let mut check = false;
    let mut files = Vec::new();

    for arg in args {
        match *arg {
            "-c" | "--check" => check = true,
            arg if !arg.starts_with('-') => files.push(arg.to_string()),
            _ => {}
        }
    }

    if check {
        let input = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => {
                    return CommandOutput::error(format!("{}: missing checksum file\n", name), 1)
                }
            }
        } else {
            match shell.vfs.read_to_string(&files[0], &shell.cwd) {
                Ok(c) => c,
                Err(e) => {
                    return CommandOutput::error(format!("{}: {}: {}\n", name, files[0], e), 1)
                }
            }
        };
        return verify_checksums_sha::<H>(shell, &input, name);
    }

    if files.is_empty() {
        let input = match stdin {
            Some(s) => s,
            None => return CommandOutput::error(format!("{}: missing operand\n", name), 1),
        };
        let hash = hex::encode(H::digest(input.as_bytes()));
        return CommandOutput::success(format!("{}  -\n", hash));
    }

    let mut output = String::new();
    for file in &files {
        match shell.vfs.read(file, &shell.cwd) {
            Ok(data) => {
                let hash = hex::encode(H::digest(&data));
                output.push_str(&format!("{}  {}\n", hash, file));
            }
            Err(e) => {
                output.push_str(&format!("{}: {}: {}\n", name, file, e));
            }
        }
    }
    CommandOutput::success(output)
}

fn cmd_hashsum_md5(shell: &Shell, args: &[&str], stdin: Option<&str>) -> CommandOutput {
    let mut check = false;
    let mut files = Vec::new();

    for arg in args {
        match *arg {
            "-c" | "--check" => check = true,
            arg if !arg.starts_with('-') => files.push(arg.to_string()),
            _ => {}
        }
    }

    if check {
        let input = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => {
                    return CommandOutput::error("md5sum: missing checksum file\n".to_string(), 1)
                }
            }
        } else {
            match shell.vfs.read_to_string(&files[0], &shell.cwd) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("md5sum: {}: {}\n", files[0], e), 1),
            }
        };
        return verify_checksums_md5(shell, &input);
    }

    if files.is_empty() {
        let input = match stdin {
            Some(s) => s,
            None => return CommandOutput::error("md5sum: missing operand\n".to_string(), 1),
        };
        let hash = hex::encode(Md5::digest(input.as_bytes()));
        return CommandOutput::success(format!("{}  -\n", hash));
    }

    let mut output = String::new();
    for file in &files {
        match shell.vfs.read(file, &shell.cwd) {
            Ok(data) => {
                let hash = hex::encode(Md5::digest(&data));
                output.push_str(&format!("{}  {}\n", hash, file));
            }
            Err(e) => {
                output.push_str(&format!("md5sum: {}: {}\n", file, e));
            }
        }
    }
    CommandOutput::success(output)
}

fn verify_checksums_sha<H: Digest>(shell: &Shell, input: &str, name: &str) -> CommandOutput {
    let mut output = String::new();
    let mut fail = 0usize;

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() < 2 {
            output.push_str(&format!("{}: invalid line: {}\n", name, line));
            fail += 1;
            continue;
        }
        let expected_hash = parts[0];
        let file_path = parts[parts.len() - 1];
        if file_path == "-" || file_path.is_empty() {
            fail += 1;
            continue;
        }
        match shell.vfs.read(file_path, &shell.cwd) {
            Ok(data) => {
                let actual = hex::encode(H::digest(&data));
                if actual == expected_hash {
                    output.push_str(&format!("{}: OK\n", file_path));
                } else {
                    output.push_str(&format!("{}: FAILED\n", file_path));
                    fail += 1;
                }
            }
            Err(e) => {
                output.push_str(&format!("{}: {}: {}\n", name, file_path, e));
                fail += 1;
            }
        }
    }

    let exit_code = if fail > 0 { 1 } else { 0 };
    CommandOutput {
        stdout: output,
        stderr: String::new(),
        exit_code,
    }
}

fn verify_checksums_md5(shell: &Shell, input: &str) -> CommandOutput {
    let mut output = String::new();
    let mut fail = 0usize;

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() < 2 {
            output.push_str(&format!("md5sum: invalid line: {}\n", line));
            fail += 1;
            continue;
        }
        let expected_hash = parts[0];
        let file_path = parts[parts.len() - 1];
        if file_path == "-" || file_path.is_empty() {
            fail += 1;
            continue;
        }
        match shell.vfs.read(file_path, &shell.cwd) {
            Ok(data) => {
                let actual = hex::encode(Md5::digest(&data));
                if actual == expected_hash {
                    output.push_str(&format!("{}: OK\n", file_path));
                } else {
                    output.push_str(&format!("{}: FAILED\n", file_path));
                    fail += 1;
                }
            }
            Err(e) => {
                output.push_str(&format!("md5sum: {}: {}\n", file_path, e));
                fail += 1;
            }
        }
    }

    let exit_code = if fail > 0 { 1 } else { 0 };
    CommandOutput {
        stdout: output,
        stderr: String::new(),
        exit_code,
    }
}

mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join("")
    }
}
