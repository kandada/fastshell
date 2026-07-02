// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{list_processes, CommandOutput, Shell};

impl Shell {
    pub fn cmd_logger(&self, args: &[&str]) -> CommandOutput {
        let msg = args.join(" ");
        #[cfg(unix)]
        {
            let msg_c = std::ffi::CString::new(msg.as_str()).unwrap_or_default();
            // SAFETY: syslog reads format + args synchronously; pointers live for duration of call
            unsafe {
                libc::syslog(
                    libc::LOG_USER | libc::LOG_NOTICE,
                    b"%s\0".as_ptr() as *const _,
                    msg_c.as_ptr(),
                );
            }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_dmesg(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(target_os = "linux")]
        {
            if let Ok(c) = std::fs::read_to_string("/dev/kmsg") {
                return CommandOutput::success(c);
            }
        }
        #[cfg(target_os = "macos")]
        {
            let o = std::process::Command::new("dmesg").output().ok();
            if let Some(o) = o {
                return CommandOutput {
                    stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                    stderr: String::new(),
                    exit_code: 0,
                };
            }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_pstree(&self, _args: &[&str]) -> CommandOutput {
        let procs = match list_processes() {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("pstree: {}\n", e), 1),
        };
        let mut children: std::collections::HashMap<u32, Vec<u32>> =
            std::collections::HashMap::new();
        let mut roots = Vec::new();
        let mut pid_map: std::collections::HashMap<u32, &crate::shell::ProcInfo> =
            std::collections::HashMap::new();
        for p in &procs {
            pid_map.insert(p.pid, p);
        }

        for p in &procs {
            if p.ppid == 0 || p.ppid == 1 || !pid_map.contains_key(&p.ppid) {
                roots.push(p.pid);
            } else {
                children.entry(p.ppid).or_default().push(p.pid);
            }
        }

        let mut output = String::new();
        for &root in &roots {
            print_tree(root, "", true, &pid_map, &children, &mut output);
        }
        CommandOutput::success(output)
    }

    pub fn cmd_killall(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("killall: missing program name\n".to_string(), 1);
        }
        let name = args
            .iter()
            .find(|a| !a.starts_with('-'))
            .copied()
            .unwrap_or("");
        let procs = match list_processes() {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("killall: {}\n", e), 1),
        };
        for proc in &procs {
            if proc.comm == name || proc.comm.starts_with(name) {
                #[cfg(unix)]
                unsafe {
                    libc::kill(proc.pid as i32, 15);
                }
            }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_watch(&self, args: &[&str]) -> CommandOutput {
        let cmd_args: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if cmd_args.is_empty() {
            return CommandOutput::error("watch: missing command\n".to_string(), 1);
        }
        let mut output = String::new();
        for _ in 0..1 {
            let vfs_root = self.vfs.root().to_path_buf();
            let cwd = if self.cwd == "/" {
                vfs_root.clone()
            } else {
                vfs_root.join(self.cwd.trim_start_matches('/'))
            };
            match std::process::Command::new(cmd_args[0])
                .args(&cmd_args[1..])
                .current_dir(&cwd)
                .output()
            {
                Ok(o) => {
                    output.push_str(&String::from_utf8_lossy(&o.stdout));
                    output.push('\n');
                }
                Err(e) => {
                    output.push_str(&format!("watch: {}\n", e));
                }
            }
        }
        CommandOutput::success(output)
    }

    pub fn cmd_logname(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(unix)]
        {
            let name = unsafe { libc::getlogin() };
            if name.is_null() {
                return CommandOutput::error("logname: no login name\n".to_string(), 1);
            }
            CommandOutput::success(
                unsafe { std::ffi::CStr::from_ptr(name).to_string_lossy().to_string() } + "\n",
            )
        }
        #[cfg(not(unix))]
        {
            CommandOutput::success(std::env::var("USER").unwrap_or_default() + "\n")
        }
    }

    pub fn cmd_who(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(target_os = "linux")]
        {
            let o = std::process::Command::new("who").output().ok();
            if let Some(o) = o {
                return CommandOutput {
                    stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                    stderr: String::new(),
                    exit_code: 0,
                };
            }
        }
        #[cfg(target_os = "macos")]
        {
            let o = std::process::Command::new("who").output().ok();
            if let Some(o) = o {
                return CommandOutput {
                    stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                    stderr: String::new(),
                    exit_code: 0,
                };
            }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_reset(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success("\x1b[0m\x1b[2J\x1b[H\x1b[?25h".to_string())
    }

    pub fn cmd_hexdump(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut files = Vec::new();
        for arg in args {
            if !arg.starts_with('-') {
                files.push(arg.to_string());
            }
        }
        let data = if files.is_empty() {
            match stdin {
                Some(s) => s.as_bytes().to_vec(),
                None => return CommandOutput::error("hexdump: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = Vec::new();
            for f in &files {
                match self.vfs.read(f, &self.cwd) {
                    Ok(d) => all.extend_from_slice(&d),
                    Err(e) => return CommandOutput::error(format!("hexdump: {}: {}\n", f, e), 1),
                }
            }
            all
        };
        let mut output = String::new();
        for (offset, chunk) in data.chunks(16).enumerate() {
            output.push_str(&format!("{:08x}  ", offset * 16));
            for j in 0..16 {
                if j < chunk.len() {
                    output.push_str(&format!("{:02x} ", chunk[j]));
                } else {
                    output.push_str("   ");
                }
                if j == 7 {
                    output.push(' ');
                }
            }
            output.push_str(" |");
            for &b in chunk {
                output.push(if b >= 0x20 && b < 0x7f {
                    b as char
                } else {
                    '.'
                });
            }
            output.push_str("|\n");
        }
        if !data.is_empty() {
            output.push_str(&format!("{:08x}\n", data.len()));
        }
        CommandOutput::success(output)
    }

    pub fn cmd_sha3sum(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        use sha3::{Digest, Sha3_256};
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
                        return CommandOutput::error(
                            "sha3sum: missing checksum file\n".to_string(),
                            1,
                        )
                    }
                }
            } else {
                match self.vfs.read_to_string(&files[0], &self.cwd) {
                    Ok(c) => c,
                    Err(e) => {
                        return CommandOutput::error(format!("sha3sum: {}: {}\n", files[0], e), 1)
                    }
                }
            };
            let mut out = String::new();
            let mut fail = 0usize;
            for line in input.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                if parts.len() < 2 {
                    fail += 1;
                    continue;
                }
                let path = parts[parts.len() - 1];
                if path == "-" {
                    continue;
                }
                match self.vfs.read(path, &self.cwd) {
                    Ok(data) => {
                        let actual = format!("{:x}", Sha3_256::digest(&data));
                        if actual == parts[0] {
                            out.push_str(&format!("{}: OK\n", path));
                        } else {
                            out.push_str(&format!("{}: FAILED\n", path));
                            fail += 1;
                        }
                    }
                    Err(e) => {
                        out.push_str(&format!("sha3sum: {}: {}\n", path, e));
                        fail += 1;
                    }
                }
            }
            return CommandOutput {
                stdout: out,
                stderr: String::new(),
                exit_code: if fail > 0 { 1 } else { 0 },
            };
        }
        if files.is_empty() {
            let input = match stdin {
                Some(s) => s,
                None => return CommandOutput::error("sha3sum: missing operand\n".to_string(), 1),
            };
            return CommandOutput::success(format!(
                "{}  -\n",
                format!("{:x}", Sha3_256::digest(input.as_bytes()))
            ));
        }
        let mut out = String::new();
        for f in &files {
            match self.vfs.read(f, &self.cwd) {
                Ok(d) => out.push_str(&format!(
                    "{}  {}\n",
                    format!("{:x}", Sha3_256::digest(&d)),
                    f
                )),
                Err(e) => out.push_str(&format!("sha3sum: {}: {}\n", f, e)),
            }
        }
        CommandOutput::success(out)
    }

    pub fn cmd_tsort(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let input = if args.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("tsort: missing input\n".to_string(), 1),
            }
        } else {
            args.join(" ")
        };
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        for line in input.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                pairs.push((parts[0], parts[1]));
            }
        }
        let mut graph: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        let mut in_degree: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for &(u, v) in &pairs {
            graph.entry(u).or_default().push(v);
            graph.entry(v).or_default();
            *in_degree.entry(v).or_default() += 1;
            in_degree.entry(u).or_default();
        }
        let mut queue: Vec<&str> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&n, _)| n)
            .collect();
        let mut output = String::new();
        while let Some(node) = queue.pop() {
            output.push_str(node);
            output.push('\n');
            if let Some(neighbors) = graph.get(node) {
                for &nbr in neighbors {
                    let d = in_degree.get_mut(nbr).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        queue.push(nbr);
                    }
                }
            }
        }
        CommandOutput::success(output)
    }
}

fn print_tree(
    pid: u32,
    prefix: &str,
    is_last: bool,
    pid_map: &std::collections::HashMap<u32, &crate::shell::ProcInfo>,
    children: &std::collections::HashMap<u32, Vec<u32>>,
    output: &mut String,
) {
    let _connector = if is_last { " \\- " } else { " |- " };
    if let Some(p) = pid_map.get(&pid) {
        output.push_str(&format!("{}{}({})\n", prefix, p.comm, pid));
    }
    if let Some(kids) = children.get(&pid) {
        let new_prefix = format!("{}{}", prefix, if is_last { "   " } else { "|  " });
        for (i, &kid) in kids.iter().enumerate() {
            print_tree(
                kid,
                &new_prefix,
                i == kids.len() - 1,
                pid_map,
                children,
                output,
            );
        }
    }
}
