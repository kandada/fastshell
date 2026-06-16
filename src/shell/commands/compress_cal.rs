use crate::shell::{Shell, CommandOutput};
use std::io::{Read, Write};

impl Shell {
    pub fn cmd_bzip2(&self, args: &[&str]) -> CommandOutput {
        let mut to_stdout = false;
        let mut keep = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-c" | "--stdout" => to_stdout = true,
                "-k" | "--keep" => keep = true,
                _ if arg.starts_with('-') => {}
                _ => files.push(arg.to_string()),
            }
        }
        if files.is_empty() { return CommandOutput::error("bzip2: missing file operand\n".to_string(), 1); }

        let mut output_bytes = Vec::new();
        for file in &files {
            let input = match self.vfs.read(file, &self.cwd) { Ok(b) => b, Err(e) => return CommandOutput::error(format!("bzip2: {}: {}\n", file, e), 1) };
            let mut compressed = Vec::new();
            {
                let mut encoder = bzip2::write::BzEncoder::new(&mut compressed, bzip2::Compression::default());
                encoder.write_all(&input).ok();
                encoder.finish().ok();
            }
            if to_stdout { output_bytes.extend_from_slice(&compressed); }
            else {
                let out = format!("{}.bz2", file);
                if let Err(e) = self.vfs.write_bytes(&out, &self.cwd, &compressed) { return CommandOutput::error(format!("bzip2: {}: {}\n", out, e), 1); }
                if !keep { let _ = self.vfs.remove_file(file, &self.cwd); }
            }
        }
        if to_stdout { CommandOutput::success(String::from_utf8_lossy(&output_bytes).to_string()) }
        else { CommandOutput::success(String::new()) }
    }

    pub fn cmd_bunzip2(&self, args: &[&str]) -> CommandOutput {
        let mut to_stdout = false;
        let mut keep = false;
        let mut files = Vec::new();
        for arg in args {
            match *arg {
                "-c" | "--stdout" => to_stdout = true,
                "-k" | "--keep" => keep = true,
                _ if arg.starts_with('-') => {} _ => files.push(arg.to_string()),
            }
        }
        if files.is_empty() { return CommandOutput::error("bunzip2: missing file operand\n".to_string(), 1); }
        let mut output_bytes = Vec::new();
        for file in &files {
            let compressed = match self.vfs.read(file, &self.cwd) { Ok(b) => b, Err(e) => return CommandOutput::error(format!("bunzip2: {}: {}\n", file, e), 1) };
            let mut decompressed = Vec::new();
            if let Err(e) = bzip2::read::BzDecoder::new(&compressed[..]).read_to_end(&mut decompressed) {
                return CommandOutput::error(format!("bunzip2: {}: {}\n", file, e), 1);
            }
            if to_stdout { output_bytes.extend_from_slice(&decompressed); }
            else {
                let out = file.strip_suffix(".bz2").unwrap_or(file);
                if let Err(e) = self.vfs.write_bytes(out, &self.cwd, &decompressed) { return CommandOutput::error(format!("bunzip2: {}: {}\n", out, e), 1); }
                if !keep { let _ = self.vfs.remove_file(file, &self.cwd); }
            }
        }
        if to_stdout { CommandOutput::success(String::from_utf8_lossy(&output_bytes).to_string()) }
        else { CommandOutput::success(String::new()) }
    }

    pub fn cmd_xz(&self, args: &[&str]) -> CommandOutput {
        let mut to_stdout = false; let mut keep = false; let mut files = Vec::new();
        for arg in args { match *arg { "-c"|"--stdout" => to_stdout = true, "-k"|"--keep" => keep = true, _ if arg.starts_with('-') => {}, _ => files.push(arg.to_string()) } }
        if files.is_empty() { return CommandOutput::error("xz: missing file operand\n".to_string(), 1); }
        let mut output_bytes = Vec::new();
        for file in &files {
            let input = match self.vfs.read(file, &self.cwd) { Ok(b) => b, Err(e) => return CommandOutput::error(format!("xz: {}: {}\n", file, e), 1) };
            let mut compressed = Vec::new();
            { let mut e = xz2::write::XzEncoder::new(&mut compressed, 6); e.write_all(&input).ok(); e.finish().ok(); }
            if to_stdout { output_bytes.extend_from_slice(&compressed); }
            else { let out = format!("{}.xz", file); self.vfs.write_bytes(&out, &self.cwd, &compressed).ok(); if !keep { let _ = self.vfs.remove_file(file, &self.cwd); } }
        }
        if to_stdout { CommandOutput::success(String::from_utf8_lossy(&output_bytes).to_string()) }
        else { CommandOutput::success(String::new()) }
    }

    pub fn cmd_unxz(&self, args: &[&str]) -> CommandOutput {
        let mut to_stdout = false; let mut keep = false; let mut files = Vec::new();
        for arg in args { match *arg { "-c"|"--stdout" => to_stdout = true, "-k"|"--keep" => keep = true, _ if arg.starts_with('-') => {}, _ => files.push(arg.to_string()) } }
        if files.is_empty() { return CommandOutput::error("unxz: missing file operand\n".to_string(), 1); }
        let mut output_bytes = Vec::new();
        for file in &files {
            let compressed = match self.vfs.read(file, &self.cwd) { Ok(b) => b, Err(e) => return CommandOutput::error(format!("unxz: {}: {}\n", file, e), 1) };
            let mut decompressed = Vec::new();
            if let Err(e) = xz2::read::XzDecoder::new(&compressed[..]).read_to_end(&mut decompressed) { return CommandOutput::error(format!("unxz: {}: {}\n", file, e), 1); }
            if to_stdout { output_bytes.extend_from_slice(&decompressed); }
            else { let out = file.strip_suffix(".xz").unwrap_or(file); self.vfs.write_bytes(out, &self.cwd, &decompressed).ok(); if !keep { let _ = self.vfs.remove_file(file, &self.cwd); } }
        }
        if to_stdout { CommandOutput::success(String::from_utf8_lossy(&output_bytes).to_string()) }
        else { CommandOutput::success(String::new()) }
    }

    pub fn cmd_zcat(&self, args: &[&str]) -> CommandOutput {
        let mut files = Vec::new();
        for arg in args { if !arg.starts_with('-') { files.push(arg.to_string()); } }
        if files.is_empty() { return CommandOutput::error("zcat: missing file operand\n".to_string(), 1); }
        let mut output = Vec::new();
        for file in &files {
            let data = match self.vfs.read(file, &self.cwd) { Ok(b) => b, Err(e) => return CommandOutput::error(format!("zcat: {}: {}\n", file, e), 1) };
            let mut d = Vec::new();
            if let Err(_) = flate2::read::GzDecoder::new(&data[..]).read_to_end(&mut d) {
                return CommandOutput::error(format!("zcat: {}: invalid gzip\n", file), 1);
            }
            output.extend_from_slice(&d);
        }
        CommandOutput::success(String::from_utf8_lossy(&output).to_string())
    }

    pub fn cmd_dos2unix(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if files.is_empty() { return CommandOutput::error("dos2unix: missing file operand\n".to_string(), 1); }
        for file in &files {
            let content = match self.vfs.read_to_string(file, &self.cwd) { Ok(c) => c, Err(e) => return CommandOutput::error(format!("dos2unix: {}: {}\n", file, e), 1) };
            let converted = content.replace("\r\n", "\n");
            if let Err(e) = self.vfs.write(file, &self.cwd, &converted) { return CommandOutput::error(format!("dos2unix: {}: {}\n", file, e), 1); }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_unix2dos(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if files.is_empty() { return CommandOutput::error("unix2dos: missing file operand\n".to_string(), 1); }
        for file in &files {
            let content = match self.vfs.read_to_string(file, &self.cwd) { Ok(c) => c, Err(e) => return CommandOutput::error(format!("unix2dos: {}: {}\n", file, e), 1) };
            let converted = content.replace("\n", "\r\n");
            if let Err(e) = self.vfs.write(file, &self.cwd, &converted) { return CommandOutput::error(format!("unix2dos: {}: {}\n", file, e), 1); }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_cal(&self, args: &[&str]) -> CommandOutput {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let (cur_year, cur_month, _) = crate::shell::civil_from_days((now / 86400) as i32);
        let mut month: u32 = cur_month as u32;
        let mut year: i32 = cur_year;

        let mut i = 0;
        while i < args.len() {
            if !args[i].starts_with('-') {
                if let Ok(m) = args[i].parse::<u32>() {
                    if m >= 1 && m <= 12 { month = m; }
                    else if m > 12 { month = (m % 100) as u32; year = (m / 100) as i32; }
                }
            }
            i += 1;
        }

        let days_in_month = [31,28,31,30,31,30,31,31,30,31,30,31];
        let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days = if month == 2 && is_leap { 29 } else { days_in_month[(month - 1) as usize] };

        let first_day = day_of_week(year, month, 1);

        let months = ["", "January","February","March","April","May","June","July","August","September","October","November","December"];
        let mut out = String::new();
        out.push_str(&format!("     {} {}\n", months[month as usize], year));
        out.push_str("Su Mo Tu We Th Fr Sa\n");

        for _ in 0..first_day { out.push_str("   "); }
        for d in 1..=days {
            out.push_str(&format!("{:>2} ", d));
            if (first_day + d as u32) % 7 == 0 { out.push('\n'); }
        }
        if (first_day + days) % 7 != 0 { out.push('\n'); }

        CommandOutput::success(out)
    }
}

fn day_of_week(y: i32, m: u32, d: u32) -> u32 {
    let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let c = (y / 100) as u32;
    let y2 = (y % 100) as u32;
    ((d + (13*(m+1))/5 + y2 + y2/4 + c/4 - 2*c) % 7) as u32
}
