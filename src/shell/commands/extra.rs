use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_renice(&self, args: &[&str]) -> CommandOutput {
        let mut priority = 10i32;
        let mut pids = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => { if i + 1 < args.len() { priority = args[i+1].parse().unwrap_or(10); i += 1; } }
                arg if arg.starts_with("-n") && arg.len() > 2 => { priority = arg[2..].parse().unwrap_or(10); }
                arg if !arg.starts_with('-') => { if let Ok(p) = arg.parse::<u32>() { pids.push(p); } }
                _ => {}
            }
            i += 1;
        }
        if pids.is_empty() { return CommandOutput::error("renice: missing pid\n".to_string(), 1); }
        #[cfg(unix)]
        for &pid in &pids { unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, priority); } }
        CommandOutput::success(String::new())
    }

    pub fn cmd_nohup(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() { return CommandOutput::error("nohup: missing command\n".to_string(), 1); }
        let cmd = args[0];
        let cmd_args: Vec<&str> = args[1..].iter().filter(|a| !a.starts_with('-')).copied().collect();
        let vfs_root = self.vfs.root().to_path_buf();
        let cwd = if self.cwd == "/" { vfs_root.clone() } else { vfs_root.join(self.cwd.trim_start_matches('/')) };
        let out_path = cwd.join("nohup.out");
        let out_file = match std::fs::File::create(&out_path) {
            Ok(f) => f,
            Err(_) => return CommandOutput::error(format!("nohup: cannot create {}\n", out_path.display()), 1),
        };
        let mut child = match std::process::Command::new(cmd).args(&cmd_args).current_dir(&cwd)
            .stdout(std::process::Stdio::from(out_file))
            .stderr(std::process::Stdio::inherit()).spawn() {
            Ok(c) => c, Err(e) => return CommandOutput::error(format!("nohup: {}\n", e), 1),
        };
        CommandOutput::success(format!("nohup: appending output to '{}', pid {}\n", out_path.display(), child.id()))
    }

    pub fn cmd_chroot(&self, args: &[&str]) -> CommandOutput {
        if args.len() < 2 { return CommandOutput::error("chroot: missing operand\n".to_string(), 1); }
        let newroot = args[0];
        let cmd = args[1];
        let cmd_args: Vec<&str> = args[2..].to_vec();
        let resolved = match self.vfs.resolve(newroot, &self.cwd) { Ok(p) => p, Err(e) => return CommandOutput::error(format!("chroot: {}: {}\n", newroot, e), 1) };
        #[cfg(unix)]
        {
            let path_c = std::ffi::CString::new(resolved.to_string_lossy().as_bytes()).unwrap();
            unsafe { libc::chroot(path_c.as_ptr()); libc::chdir(b"/\0".as_ptr() as *const _); }
        }
        let output = std::process::Command::new(cmd).args(&cmd_args).output();
        match output {
            Ok(o) => CommandOutput { stdout: String::from_utf8_lossy(&o.stdout).to_string(), stderr: String::from_utf8_lossy(&o.stderr).to_string(), exit_code: o.status.code().unwrap_or(-1) },
            Err(e) => CommandOutput::error(format!("chroot: {}\n", e), 1),
        }
    }

    pub fn cmd_mkfifo(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if files.is_empty() { return CommandOutput::error("mkfifo: missing operand\n".to_string(), 1); }
        #[cfg(unix)]
        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) { Ok(p) => p, Err(e) => return CommandOutput::error(format!("mkfifo: {}: {}\n", file, e), 1) };
            let path_c = std::ffi::CString::new(resolved.to_string_lossy().as_bytes()).unwrap();
            if unsafe { libc::mkfifo(path_c.as_ptr(), 0o666) } != 0 { return CommandOutput::error(format!("mkfifo: {}: {}\n", file, std::io::Error::last_os_error()), 1); }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_install(&self, args: &[&str]) -> CommandOutput {
        let mut mode: Option<u32> = None;
        let mut dir = false;
        let mut files = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-m" => { if i + 1 < args.len() { mode = Some(u32::from_str_radix(args[i+1], 8).unwrap_or(0o755)); i += 1; } }
                "-d" => dir = true,
                arg if arg.starts_with("-m") && arg.len() > 2 => { mode = u32::from_str_radix(&arg[2..], 8).ok(); }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }
        if files.len() < 2 { return CommandOutput::error("install: missing file operand\n".to_string(), 1); }
        let dest = files.pop().unwrap();
        for src in &files {
            let src_bytes = match self.vfs.read(src, &self.cwd) { Ok(b) => b, Err(e) => return CommandOutput::error(format!("install: {}: {}\n", src, e), 1) };
            let dest_path = if dir {
                let name = std::path::Path::new(src).file_name().unwrap_or_default();
                format!("{}/{}", dest, name.to_string_lossy())
            } else { dest.clone() };
            if let Err(e) = self.vfs.write_bytes(&dest_path, &self.cwd, &src_bytes) { return CommandOutput::error(format!("install: {}: {}\n", dest_path, e), 1); }
            if let Some(m) = mode {
                let resolved = self.vfs.resolve(&dest_path, &self.cwd).ok();
                #[cfg(unix)] if let Some(ref r) = resolved { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(r, std::fs::Permissions::from_mode(m)).ok(); }
            }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_shred(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if files.is_empty() { return CommandOutput::error("shred: missing file operand\n".to_string(), 1); }
        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) { Ok(p) => p, Err(e) => return CommandOutput::error(format!("shred: {}: {}\n", file, e), 1) };
            let len = resolved.metadata().map(|m| m.len()).unwrap_or(0);
            let mut f = match std::fs::OpenOptions::new().write(true).open(&resolved) { Ok(f) => f, Err(e) => return CommandOutput::error(format!("shred: {}: {}\n", file, e), 1) };
            use std::io::Write;
            for _ in 0..3 {
                f.write_all(&vec![0xAAu8; len as usize]).ok();
                f.write_all(&vec![0x55u8; len as usize]).ok();
                f.write_all(&vec![0xFFu8; len as usize]).ok();
            }
            let _ = std::fs::remove_file(&resolved);
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_fallocate(&self, args: &[&str]) -> CommandOutput {
        let mut length: Option<u64> = None;
        let mut files = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-l" => { if i + 1 < args.len() { length = args[i+1].parse().ok(); i += 1; } }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }
        if files.is_empty() { return CommandOutput::error("fallocate: missing file operand\n".to_string(), 1); }
        let len = length.unwrap_or(0);
        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) { Ok(p) => p, Err(e) => return CommandOutput::error(format!("fallocate: {}: {}\n", file, e), 1) };
            let f = match std::fs::OpenOptions::new().write(true).create(true).open(&resolved) { Ok(f) => f, Err(e) => return CommandOutput::error(format!("fallocate: {}: {}\n", file, e), 1) };
            f.set_len(len).ok();
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_telnet(&self, args: &[&str]) -> CommandOutput {
        let args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if args.is_empty() { return CommandOutput::error("telnet: missing host\n".to_string(), 1); }
        run_system_cmd(self, "telnet", &args)
    }

    pub fn cmd_traceroute(&self, args: &[&str]) -> CommandOutput {
        let args: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if args.is_empty() { return CommandOutput::error("traceroute: missing host\n".to_string(), 1); }
        run_system_cmd(self, "traceroute", &args)
    }

    pub fn cmd_ifconfig(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system_cmd(self, "ifconfig", &s_args)
    }

    pub fn cmd_netstat(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system_cmd(self, "netstat", &s_args)
    }

    pub fn cmd_nc(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system_cmd(self, "nc", &s_args)
    }

    pub fn cmd_patch(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.is_empty() && stdin.is_none() { return CommandOutput::error("patch: missing input\n".to_string(), 1); }
        let s_args: Vec<&str> = args.to_vec();
        run_system_cmd(self, "patch", &s_args)
    }

    pub fn cmd_mknod(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        if files.len() < 2 { return CommandOutput::error("mknod: missing operand\n".to_string(), 1); }
        #[cfg(unix)]
        {
            let resolved = match self.vfs.resolve(files[0], &self.cwd) { Ok(p) => p, Err(e) => return CommandOutput::error(format!("mknod: {}: {}\n", files[0], e), 1) };
            let path_c = std::ffi::CString::new(resolved.to_string_lossy().as_bytes()).unwrap();
            let mode: libc::mode_t = u32::from_str_radix(files[1], 8).unwrap_or(0o666) as libc::mode_t | libc::S_IFREG;
            unsafe { libc::mknod(path_c.as_ptr(), mode, 0); }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_mount(&self, args: &[&str]) -> CommandOutput {
        run_system_cmd(self, "mount", args)
    }

    pub fn cmd_umount(&self, args: &[&str]) -> CommandOutput {
        run_system_cmd(self, "umount", args)
    }
}

fn run_system_cmd(shell: &Shell, cmd: &str, args: &[&str]) -> CommandOutput {
    let vfs_root = shell.vfs.root().to_path_buf();
    let cwd = if shell.cwd == "/" { vfs_root.clone() } else { vfs_root.join(shell.cwd.trim_start_matches('/')) };
    match std::process::Command::new(cmd).args(args).current_dir(&cwd).output() {
        Ok(o) => CommandOutput { stdout: String::from_utf8_lossy(&o.stdout).to_string(), stderr: String::from_utf8_lossy(&o.stderr).to_string(), exit_code: o.status.code().unwrap_or(-1) },
        Err(e) => CommandOutput::error(format!("{}: {}\n", cmd, e), 1),
    }
}
