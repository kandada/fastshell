use sha1::Sha1;
use sha2::Digest;
use std::io::Read;
use crate::shell::{Shell, CommandOutput, list_processes};

impl Shell {
    pub fn cmd_sha1sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        cmd_hashsum_sha1(self, args, stdin)
    }

    pub fn cmd_sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut files = Vec::new();
        for arg in args {
            if !arg.starts_with('-') { files.push(arg.to_string()); }
        }

        if files.is_empty() {
            let input = match stdin {
                Some(s) => s,
                None => return CommandOutput::error("sum: missing operand\n".to_string(), 1),
            };
            let checksum = bsd_sum(input.as_bytes());
            let blocks = (input.len() + 1023) / 1024;
            return CommandOutput::success(format!("{:05} {:>5}\n", checksum, blocks));
        }

        let mut output = String::new();
        for file in &files {
            match self.vfs.read(file, &self.cwd) {
                Ok(data) => {
                    let checksum = bsd_sum(&data);
                    let blocks = (data.len() + 1023) / 1024;
                    output.push_str(&format!("{:05} {:>5} {}\n", checksum, blocks, file));
                }
                Err(e) => return CommandOutput::error(format!("sum: {}: {}\n", file, e), 1),
            }
        }
        CommandOutput::success(output)
    }

    pub fn cmd_pidof(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("pidof: missing program name\n".to_string(), 1);
        }

        let procs = match list_processes() {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("pidof: {}\n", e), 1),
        };

        let mut output = String::new();
        for arg in args {
            if arg.starts_with('-') { continue; }
            let name = arg;
            for proc in &procs {
                if &proc.comm == name || proc.comm.starts_with(name) {
                    output.push_str(&format!("{} ", proc.pid));
                }
            }
        }

        if output.is_empty() {
            CommandOutput { stdout: String::new(), stderr: String::new(), exit_code: 1 }
        } else {
            output.pop();
            output.push('\n');
            CommandOutput::success(output)
        }
    }

    pub fn cmd_nproc(&self, args: &[&str]) -> CommandOutput {
        let mut all = false;
        for arg in args {
            if *arg == "--all" { all = true; }
        }

        let count = if all {
            num_cpus::get()
        } else {
            num_cpus::get_physical()
        };

        CommandOutput::success(format!("{}\n", count))
    }

    pub fn cmd_tty(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(unix)]
        {
            let fd = unsafe { libc::ttyname(0) };
            if fd.is_null() {
                CommandOutput {
                    stdout: "not a tty\n".to_string(),
                    stderr: String::new(),
                    exit_code: 1,
                }
            } else {
                let name = unsafe { std::ffi::CStr::from_ptr(fd).to_string_lossy().to_string() };
                CommandOutput::success(name + "\n")
            }
        }
        #[cfg(not(unix))]
        {
            CommandOutput { stdout: "not a tty\n".to_string(), stderr: String::new(), exit_code: 1 }
        }
    }

    pub fn cmd_clear(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success("\x1b[2J\x1b[H".to_string())
    }

    pub fn cmd_sync(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(unix)]
        unsafe { libc::sync(); }
        CommandOutput::success(String::new())
    }

    pub fn cmd_nice(&self, args: &[&str]) -> CommandOutput {
        let mut adjustment = 10i32;
        let mut command: Option<String> = None;
        let mut cmd_args = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => {
                    if i + 1 < args.len() {
                        adjustment = args[i + 1].parse().unwrap_or(10);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-n") && arg.len() > 2 => {
                    adjustment = arg[2..].parse().unwrap_or(10);
                }
                arg if arg.starts_with('-') && arg.len() > 1 => {
                    if let Ok(adj) = arg[1..].parse::<i32>() {
                        adjustment = adj;
                    }
                }
                _ => {
                    if command.is_none() {
                        command = Some(args[i].to_string());
                    } else {
                        cmd_args.push(args[i].to_string());
                    }
                }
            }
            i += 1;
        }

        #[cfg(unix)]
        {
            if adjustment != 0 {
                unsafe { libc::nice(adjustment); }
            }
        }

        match command {
            Some(cmd) => {
                let vfs_root = self.vfs.root().to_path_buf();
                let cwd = if self.cwd == "/" { vfs_root.clone() } else { vfs_root.join(self.cwd.trim_start_matches('/')) };
                let cmd_args_ref: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
                let output = std::process::Command::new(&cmd)
                    .args(&cmd_args_ref)
                    .current_dir(&cwd)
                    .output();
                match output {
                    Ok(out) => CommandOutput {
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        exit_code: out.status.code().unwrap_or(-1),
                    },
                    Err(e) => CommandOutput::error(format!("nice: {}\n", e), 1),
                }
            }
            None => CommandOutput::success(String::new()),
        }
    }

    pub fn cmd_chown(&self, args: &[&str]) -> CommandOutput {
        if args.len() < 2 {
            return CommandOutput::error("chown: missing operand\n".to_string(), 1);
        }

        let owner_spec = args[0];
        let files = &args[1..];

        #[cfg(unix)]
        {
            let (uid, gid) = parse_owner(owner_spec);
            for file in files {
                if file.starts_with('-') { continue; }
                let resolved = match self.vfs.resolve(file, &self.cwd) {
                    Ok(p) => p,
                    Err(e) => return CommandOutput::error(format!("chown: {}: {}\n", file, e), 1),
                };
                let path_c = std::ffi::CString::new(resolved.to_string_lossy().as_bytes()).unwrap();
                let ret = unsafe { libc::chown(path_c.as_ptr(), uid, gid) };
                if ret != 0 {
                    let e = std::io::Error::last_os_error();
                    if e.raw_os_error() != Some(1) {
                        return CommandOutput::error(format!("chown: {}: {}\n", file, e), 1);
                    }
                }
            }
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_chgrp(&self, args: &[&str]) -> CommandOutput {
        if args.len() < 2 {
            return CommandOutput::error("chgrp: missing operand\n".to_string(), 1);
        }

        let group_spec = args[0];
        let files = &args[1..];

        #[cfg(unix)]
        {
            let gid = parse_group(group_spec);
            for file in files {
                if file.starts_with('-') { continue; }
                let resolved = match self.vfs.resolve(file, &self.cwd) {
                    Ok(p) => p,
                    Err(e) => return CommandOutput::error(format!("chgrp: {}: {}\n", file, e), 1),
                };
                let path_c = std::ffi::CString::new(resolved.to_string_lossy().as_bytes()).unwrap();
                unsafe { libc::chown(path_c.as_ptr(), u32::MAX, gid); }
            }
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_groups(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(unix)]
        {
            let _gid = unsafe { libc::getgid() };
            let _count = 0i32;
            let groups = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
            let mut gids = vec![0u32; groups as usize];
            unsafe { libc::getgroups(groups, gids.as_mut_ptr()) };

            let pw = unsafe { libc::getpwuid(libc::getuid()) };
            let user_name = if pw.is_null() { "?".to_string() } else {
                unsafe { std::ffi::CStr::from_ptr((*pw).pw_name).to_string_lossy().to_string() }
            };

            let mut names = Vec::new();
            for &g in &gids {
                let gr = unsafe { libc::getgrgid(g) };
                if !gr.is_null() {
                    let name = unsafe { std::ffi::CStr::from_ptr((*gr).gr_name).to_string_lossy().to_string() };
                    names.push(name);
                }
            }

            CommandOutput::success(format!("{} : {}\n", user_name, names.join(" ")))
        }
        #[cfg(not(unix))]
        {
            CommandOutput::success(String::new())
        }
    }

    pub fn cmd_dd(&self, args: &[&str]) -> CommandOutput {
        let mut ifile = String::new();
        let mut ofile = String::new();
        let mut bs: usize = 512;
        let mut count: Option<usize> = None;
        let mut skip: usize = 0;
        let mut seek: usize = 0;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "if" => { if i + 1 < args.len() { ifile = args[i + 1].to_string(); i += 1; } }
                "of" => { if i + 1 < args.len() { ofile = args[i + 1].to_string(); i += 1; } }
                "bs" => { if i + 1 < args.len() { bs = parse_dd_size(args[i + 1]); i += 1; } }
                "count" => { if i + 1 < args.len() { count = Some(args[i + 1].parse().unwrap_or(0)); i += 1; } }
                "skip" => { if i + 1 < args.len() { skip = args[i + 1].parse().unwrap_or(0); i += 1; } }
                "seek" => { if i + 1 < args.len() { seek = args[i + 1].parse().unwrap_or(0); i += 1; } }
                arg if arg.contains('=') => {
                    let parts: Vec<&str> = arg.splitn(2, '=').collect();
                    match parts[0] {
                        "if" => ifile = parts[1].to_string(),
                        "of" => ofile = parts[1].to_string(),
                        "bs" => bs = parse_dd_size(parts[1]),
                        "count" => count = parts[1].parse().ok(),
                        "skip" => skip = parts[1].parse().unwrap_or(0),
                        "seek" => seek = parts[1].parse().unwrap_or(0),
                        _ => {}
                    }
                }
                _ => {}
            }
            i += 1;
        }

        let input = if ifile.is_empty() {
            std::io::stdin().bytes().filter_map(|b| b.ok()).collect::<Vec<u8>>()
        } else {
            match self.vfs.read(&ifile, &self.cwd) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("dd: {}: {}\n", ifile, e), 1),
            }
        };

        let start = skip * bs;
        let data = if start < input.len() {
            &input[start..]
        } else {
            &[]
        };

        let max_bytes = count.unwrap_or(usize::MAX) * bs;
        let to_write = &data[..max_bytes.min(data.len())];

        let written;
        if !ofile.is_empty() {
            let resolved = match self.vfs.resolve(&ofile, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("dd: {}: {}\n", ofile, e), 1),
            };
            let mut f = match std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&resolved) {
                Ok(f) => f,
                Err(e) => return CommandOutput::error(format!("dd: {}: {}\n", ofile, e), 1),
            };
            use std::io::Write;
            if seek > 0 {
                f.write_all(&vec![0u8; seek * bs]).ok();
            }
            f.write_all(to_write).ok();
            written = to_write.len();
        } else {
            use std::io::Write;
            std::io::stdout().write_all(to_write).ok();
            written = to_write.len();
        }

        let blocks = (written + bs - 1) / bs;
        CommandOutput {
            stdout: format!("{}+0 records in\n{}+0 records out\n{} bytes transferred\n", blocks, blocks, written),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    pub fn cmd_od(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut format = "o";
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-t" => {
                    if i + 1 < args.len() { format = args[i + 1]; i += 1; }
                }
                "-A" | "-j" | "-N" | "-w" => { i += 1; } // skip these flags
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let data = if files.is_empty() {
            match stdin {
                Some(s) => s.as_bytes().to_vec(),
                None => return CommandOutput::error("od: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = Vec::new();
            for file in &files {
                match self.vfs.read(file, &self.cwd) {
                    Ok(d) => all.extend_from_slice(&d),
                    Err(e) => return CommandOutput::error(format!("od: {}: {}\n", file, e), 1),
                }
            }
            all
        };

        let mut output = String::new();
        for (offset, chunk) in data.chunks(16).enumerate() {
            output.push_str(&format!("{:07o} ", offset * 16));
            match format {
                "x" | "x1" => {
                    for &byte in chunk {
                        output.push_str(&format!(" {:02x}", byte));
                    }
                }
                "d" | "d2" => {
                    for pair in chunk.chunks(2) {
                        let val = if pair.len() == 2 {
                            i16::from_le_bytes([pair[0], pair[1]])
                        } else {
                            pair[0] as i16
                        };
                        output.push_str(&format!(" {:>6}", val));
                    }
                }
                _ => {
                    for &byte in chunk {
                        output.push_str(&format!(" {:03o}", byte));
                    }
                }
            }
            output.push('\n');
        }
        output.push_str(&format!("{:07o}\n", data.len()));

        CommandOutput::success(output)
    }

    pub fn cmd_uptime(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            if let Ok(content) = std::fs::read_to_string("/proc/uptime") {
                let uptime: f64 = content.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0.0);
                let days = (uptime / 86400.0) as u64;
                let hours = ((uptime % 86400.0) / 3600.0) as u64;
                let mins = ((uptime % 3600.0) / 60.0) as u64;
                let up = if days > 0 {
                    format!("{} day{}, {:>2}:{:02}", days, if days > 1 { "s" } else { "" }, hours, mins)
                } else {
                    format!("{:>2}:{:02}", hours, mins)
                };
                return CommandOutput::success(format!("  up {}\n", up));
            }
        }
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("sysctl")
                .arg("-n").arg("kern.boottime")
                .output().ok();
            if let Some(out) = output {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Some(rest) = s.trim().strip_prefix("{ sec = ") {
                    if let Some(pos) = rest.find(',') {
                        if let Ok(boot_secs) = rest[..pos].parse::<f64>() {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default().as_secs_f64();
                            let uptime = now - boot_secs;
                            let days = (uptime / 86400.0) as u64;
                            let hours = ((uptime % 86400.0) / 3600.0) as u64;
                            let mins = ((uptime % 3600.0) / 60.0) as u64;
                            let up = if days > 0 {
                                format!("{} day{}, {:>2}:{:02}", days, if days > 1 { "s" } else { "" }, hours, mins)
                            } else {
                                format!("{:>2}:{:02}", hours, mins)
                            };
                            return CommandOutput::success(format!("  up {}\n", up));
                        }
                    }
                }
            }
        }
        CommandOutput::success("  up ?\n".to_string())
    }

    pub fn cmd_free(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
                let mut total = 0u64; let mut free = 0u64; let mut avail = 0u64;
                for line in content.lines() {
                    if line.starts_with("MemTotal:") {
                        total = line.split_whitespace().nth(1).unwrap_or("0").parse().unwrap_or(0);
                    } else if line.starts_with("MemFree:") {
                        free = line.split_whitespace().nth(1).unwrap_or("0").parse().unwrap_or(0);
                    } else if line.starts_with("MemAvailable:") {
                        avail = line.split_whitespace().nth(1).unwrap_or("0").parse().unwrap_or(0);
                    }
                }
                let used = total.saturating_sub(if avail > 0 { avail } else { free });
                let mut out = String::new();
                out.push_str(&format!("{:>10} {:>10} {:>10} {:>10} {:>10} {:>10}\n", "total", "used", "free", "shared", "buff/cache", "available"));
                out.push_str(&format!("Mem: {:>9} {:>9} {:>9}\n", total, used, free));
                return CommandOutput::success(out);
            }
        }
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("vm_stat").output().ok();
            let mut page_size = 4096u64;
            let mut free_pages = 0u64;
            let mut active_pages = 0u64;
            let mut inactive_pages = 0u64;
            let mut wired_pages = 0u64;

            if let Some(o) = output {
                let s = String::from_utf8_lossy(&o.stdout);
                for line in s.lines() {
                    if line.contains("page size") {
                        page_size = line.split_whitespace().last().unwrap_or("4096").parse().unwrap_or(4096);
                    }
                    if line.starts_with("Pages free:") {
                        free_pages = line.split_whitespace().last().unwrap_or("0").parse::<u64>().unwrap_or(0) % 1000000;
                    }
                    if line.starts_with("Pages active:") {
                        active_pages = line.split_whitespace().last().unwrap_or("0").parse::<u64>().unwrap_or(0) % 1000000;
                    }
                    if line.starts_with("Pages inactive:") {
                        inactive_pages = line.split_whitespace().last().unwrap_or("0").parse::<u64>().unwrap_or(0) % 1000000;
                    }
                    if line.starts_with("Pages wired down:") {
                        wired_pages = line.split_whitespace().last().unwrap_or("0").parse::<u64>().unwrap_or(0) % 1000000;
                    }
                }
            }

            let free = free_pages * page_size / 1024;
            let used = (active_pages + inactive_pages + wired_pages) * page_size / 1024;
            let total = free + used;

            let mut out = String::new();
            out.push_str(&format!("{:>10} {:>10} {:>10} {:>10} {:>10} {:>10}\n", "total", "used", "free", "shared", "buff/cache", "available"));
            out.push_str(&format!("Mem: {:>9} {:>9} {:>9}\n", total, used, free));
            return CommandOutput::success(out);
        }
        #[allow(unreachable_code)]
        CommandOutput::success(String::new())
    }

    pub fn cmd_nslookup(&self, args: &[&str]) -> CommandOutput {
        let host = args.iter().find(|a| !a.starts_with('-')).copied().unwrap_or("");
        if host.is_empty() {
            return CommandOutput::error("nslookup: missing hostname\n".to_string(), 1);
        }

        if let Some(perm) = self.check_network_permission(host) {
            return perm;
        }

        let output = std::process::Command::new("nslookup")
            .arg(host)
            .output();

        match output {
            Ok(o) => CommandOutput {
                stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                stderr: String::from_utf8_lossy(&o.stderr).to_string(),
                exit_code: o.status.code().unwrap_or(1),
            },
            Err(e) => CommandOutput::error(format!("nslookup: {}\n", e), 1),
        }
    }
}

fn parse_dd_size(s: &str) -> usize {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix('K') { rest.parse::<f64>().unwrap_or(0.0) as usize * 1024 }
    else if let Some(rest) = s.strip_suffix('M') { rest.parse::<f64>().unwrap_or(0.0) as usize * 1024 * 1024 }
    else if let Some(rest) = s.strip_suffix('G') { rest.parse::<f64>().unwrap_or(0.0) as usize * 1024 * 1024 * 1024 }
    else { s.parse().unwrap_or(512) }
}

fn bsd_sum(data: &[u8]) -> u16 {
    let mut checksum: u16 = 0;
    for &byte in data {
        checksum = checksum.rotate_right(1).wrapping_add(byte as u16);
    }
    checksum
}

fn cmd_hashsum_sha1(shell: &Shell, args: &[&str], stdin: Option<&str>) -> CommandOutput {
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
                None => return CommandOutput::error("sha1sum: missing checksum file\n".to_string(), 1),
            }
        } else {
            match shell.vfs.read_to_string(&files[0], &shell.cwd) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("sha1sum: {}: {}\n", files[0], e), 1),
            }
        };
        let mut output = String::new();
        let mut fail = 0usize;
        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            if parts.len() < 2 { fail += 1; continue; }
            let expected = parts[0];
            let path = parts[parts.len() - 1];
            if path == "-" { continue; }
            match shell.vfs.read(path, &shell.cwd) {
                Ok(data) => {
                    let actual = format!("{:x}", Sha1::digest(&data));
                    if actual == expected {
                        output.push_str(&format!("{}: OK\n", path));
                    } else {
                        output.push_str(&format!("{}: FAILED\n", path));
                        fail += 1;
                    }
                }
                Err(e) => {
                    output.push_str(&format!("sha1sum: {}: {}\n", path, e));
                    fail += 1;
                }
            }
        }
        let ec = if fail > 0 { 1 } else { 0 };
        return CommandOutput { stdout: output, stderr: String::new(), exit_code: ec };
    }

    if files.is_empty() {
        let input = match stdin { Some(s) => s, None => return CommandOutput::error("sha1sum: missing operand\n".to_string(), 1) };
        let hash = format!("{:x}", Sha1::digest(input.as_bytes()));
        return CommandOutput::success(format!("{}  -\n", hash));
    }

    let mut output = String::new();
    for file in &files {
        match shell.vfs.read(file, &shell.cwd) {
            Ok(data) => output.push_str(&format!("{}  {}\n", format!("{:x}", Sha1::digest(&data)), file)),
            Err(e) => output.push_str(&format!("sha1sum: {}: {}\n", file, e)),
        }
    }
    CommandOutput::success(output)
}

#[cfg(unix)]
fn parse_owner(spec: &str) -> (u32, u32) {
    let (user_str, group_str) = if let Some(pos) = spec.find(':') {
        (&spec[..pos], &spec[pos + 1..])
    } else if let Some(pos) = spec.find('.') {
        (&spec[..pos], &spec[pos + 1..])
    } else {
        (spec, "")
    };

    let uid = parse_user(user_str).unwrap_or(u32::MAX);
    let gid = if group_str.is_empty() { u32::MAX } else { parse_group(group_str) };
    (uid, gid)
}

#[cfg(unix)]
fn parse_user(s: &str) -> Option<u32> {
    if let Ok(uid) = s.parse::<u32>() { return Some(uid); }
    let cname = std::ffi::CString::new(s).ok()?;
    let pw = unsafe { libc::getpwnam(cname.as_ptr()) };
    if pw.is_null() { None } else { Some(unsafe { (*pw).pw_uid }) }
}

#[cfg(unix)]
fn parse_group(s: &str) -> u32 {
    if let Ok(gid) = s.parse::<u32>() { return gid; }
    let cname = std::ffi::CString::new(s).ok().unwrap_or_default();
    let gr = unsafe { libc::getgrnam(cname.as_ptr()) };
    if gr.is_null() { u32::MAX } else { unsafe { (*gr).gr_gid } }
}
