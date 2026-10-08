// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use blake2::Blake2b512;
use md5::Md5;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};

impl Shell {
    pub fn cmd_sha256sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_sha::<Sha256>(self, args, stdin, "sha256sum")
    }

    pub fn cmd_sha224sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_sha::<Sha224>(self, args, stdin, "sha224sum")
    }

    pub fn cmd_sha384sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_sha::<Sha384>(self, args, stdin, "sha384sum")
    }

    pub fn cmd_sha512sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_sha::<Sha512>(self, args, stdin, "sha512sum")
    }

    /// `b2sum` — BLAKE2b-512 (GNU default).
    pub fn cmd_b2sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_sha::<Blake2b512>(self, args, stdin, "b2sum")
    }

    pub fn cmd_md5sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_md5(self, args, stdin)
    }

    /// `base32 [-d] [-w COLS] [FILE...]` — RFC 4648 base32.
    pub fn cmd_base32(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut decode = false;
        let mut wrap = 76usize;
        let mut files: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-d" | "--decode" => decode = true,
                "-w" | "--wrap" => {
                    i += 1;
                    wrap = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(76);
                }
                a if a.starts_with("-w") && a.len() > 2 => {
                    wrap = a[2..].parse().unwrap_or(76);
                }
                a if a.starts_with('-') && a.len() > 1 => {}
                a => files.push(a),
            }
            i += 1;
        }
        let input: Vec<u8> = if files.is_empty() {
            match self
                .take_binary_in()
                .or_else(|| stdin.map(|s| s.as_bytes().to_vec()))
            {
                Some(b) => b,
                None => return CommandOutput::error("base32: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = Vec::new();
            for f in &files {
                match self.vfs.read(f, &self.cwd) {
                    Ok(d) => all.extend_from_slice(&d),
                    Err(e) => return CommandOutput::error(format!("base32: {}: {}\n", f, e), 1),
                }
            }
            all
        };
        if decode {
            let cleaned: String = String::from_utf8_lossy(&input)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            match base32_decode(&cleaned) {
                Some(bytes) => {
                    if !bytes.is_empty() {
                        self.set_binary_out(bytes.clone());
                    }
                    CommandOutput::success(String::from_utf8_lossy(&bytes).to_string())
                }
                None => CommandOutput::error("base32: invalid input\n".to_string(), 1),
            }
        } else {
            let encoded = base32_encode(&input);
            if wrap == 0 {
                CommandOutput::success(encoded + "\n")
            } else {
                let wrapped = encoded
                    .as_bytes()
                    .chunks(wrap.max(1))
                    .map(|c| std::str::from_utf8(c).unwrap_or_default())
                    .collect::<Vec<&str>>()
                    .join("\n");
                CommandOutput::success(wrapped + "\n")
            }
        }
    }
}

const B32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let bits = ((buf[0] as u64) << 32)
            | ((buf[1] as u64) << 24)
            | ((buf[2] as u64) << 16)
            | ((buf[3] as u64) << 8)
            | (buf[4] as u64);
        // Number of output chars for this chunk (8 for 5 bytes, 2 per partial byte).
        let n_out = match chunk.len() {
            1 => 2,
            2 => 4,
            3 => 5,
            4 => 7,
            _ => 8,
        };
        for k in 0..8 {
            if k < n_out {
                let idx = ((bits >> (35 - k * 5)) & 0x1f) as usize;
                out.push(B32_ALPHABET[idx] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::new();
    let mut bits: u64 = 0;
    let mut nbits = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        } as u64;
        bits = (bits << 5) | v;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    Some(out)
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
            match shell.read_text_lossy(&files[0]) {
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
            match shell.read_text_lossy(&files[0]) {
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

/// POSIX `cksum` (CRC-32, non-reflected polynomial 0x04C11DB7, with the input
/// length appended before the final complement). Returns `(crc, byte_len)`.
fn posix_cksum(data: &[u8]) -> (u32, usize) {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = (i as u32) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04C1_1DB7
            } else {
                c << 1
            };
        }
        *slot = c;
    }
    let mut crc: u32 = 0;
    for &b in data {
        crc = (crc << 8) ^ table[(((crc >> 24) ^ b as u32) & 0xFF) as usize];
    }
    let mut len = data.len() as u64;
    while len != 0 {
        crc = (crc << 8) ^ table[(((crc >> 24) ^ (len & 0xFF) as u32) & 0xFF) as usize];
        len >>= 8;
    }
    (!crc, data.len())
}

impl Shell {
    /// `cksum` — POSIX checksum (`<crc> <bytes> [file]`).
    pub fn cmd_cksum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let files: Vec<String> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(|s| s.to_string())
            .collect();
        if files.is_empty() {
            let input = match self
                .take_binary_in()
                .or_else(|| stdin.map(|s| s.as_bytes().to_vec()))
            {
                Some(b) => b,
                None => return CommandOutput::error("cksum: missing operand\n".to_string(), 1),
            };
            let (crc, len) = posix_cksum(&input);
            return CommandOutput::success(format!("{} {}\n", crc, len));
        }
        let mut out = String::new();
        for f in &files {
            match self.vfs.read(f, &self.cwd) {
                Ok(data) => {
                    let (crc, len) = posix_cksum(&data);
                    out.push_str(&format!("{} {} {}\n", crc, len, f));
                }
                Err(e) => out.push_str(&format!("cksum: {}: {}\n", f, e)),
            }
        }
        CommandOutput::success(out)
    }

    /// `crc32` — zlib/CRC-32 (`<8 hex> [file]`).
    pub fn cmd_crc32(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let files: Vec<String> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(|s| s.to_string())
            .collect();
        let crc_of = |data: &[u8]| -> u32 {
            let mut c = flate2::Crc::new();
            c.update(data);
            c.sum()
        };
        if files.is_empty() {
            let input = match self
                .take_binary_in()
                .or_else(|| stdin.map(|s| s.as_bytes().to_vec()))
            {
                Some(b) => b,
                None => return CommandOutput::error("crc32: missing operand\n".to_string(), 1),
            };
            return CommandOutput::success(format!("{:08x}\n", crc_of(&input)));
        }
        let mut out = String::new();
        for f in &files {
            match self.vfs.read(f, &self.cwd) {
                Ok(data) => out.push_str(&format!("{:08x}  {}\n", crc_of(&data), f)),
                Err(e) => out.push_str(&format!("crc32: {}: {}\n", f, e)),
            }
        }
        CommandOutput::success(out)
    }
}

/// Parse a checksum-check line: `<hash>  <file>` / `<hash> *<file>` (binary),
/// tolerating any amount of whitespace between the hash and the path.
fn parse_check_line(line: &str) -> Option<(String, String)> {
    let mut it = line.splitn(2, char::is_whitespace);
    let hash = it.next()?.trim().to_string();
    let rest = it.next().unwrap_or("").trim_start();
    let rest = rest.strip_prefix('*').unwrap_or(rest);
    if hash.is_empty() || rest.is_empty() {
        return None;
    }
    Some((hash, rest.to_string()))
}

fn verify_checksums_sha<H: Digest>(shell: &Shell, input: &str, name: &str) -> CommandOutput {
    let mut output = String::new();
    let mut fail = 0usize;

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((expected_hash, file_path)) = parse_check_line(line) else {
            output.push_str(&format!("{}: invalid line: {}\n", name, line));
            fail += 1;
            continue;
        };
        let expected_hash = expected_hash.as_str();
        let file_path = file_path.as_str();
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
        let Some((expected_hash, file_path)) = parse_check_line(line) else {
            output.push_str(&format!("md5sum: invalid line: {}\n", line));
            fail += 1;
            continue;
        };
        let expected_hash = expected_hash.as_str();
        let file_path = file_path.as_str();
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
