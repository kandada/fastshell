// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_renice(&self, args: &[&str]) -> CommandOutput {
        // GNU renice: `renice [-n] priority [-p] pid...`. The FIRST non-flag
        // operand is the priority; the rest are pids.
        let mut priority: i32 = 0;
        let mut have_priority = false;
        let mut pids: Vec<u32> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if a == "-n" {
                if i + 1 < args.len() {
                    priority = args[i + 1].parse().unwrap_or(0);
                    have_priority = true;
                    i += 1;
                }
            } else if let Some(v) = a.strip_prefix("-n") {
                if !v.is_empty() {
                    priority = v.parse().unwrap_or(0);
                    have_priority = true;
                }
            } else if a == "-p" || a == "-g" || a == "-u" {
                // operand-kind flag: ignored (all treated as pids)
            } else if a.starts_with('-') {
                // other flags ignored
            } else if let Ok(n) = a.parse::<i64>() {
                if !have_priority {
                    priority = n as i32;
                    have_priority = true;
                } else {
                    pids.push(n.max(0) as u32);
                }
            }
            i += 1;
        }
        if pids.is_empty() {
            return CommandOutput::error("renice: missing pid\n".to_string(), 1);
        }
        let self_pid = std::process::id();
        for &pid in &pids {
            // Refuse to change this process's own priority — that starves the
            // host app (ANR). `renice 5 -p $$` must not touch us.
            if pid == self_pid {
                return CommandOutput::error(
                    "renice: refusing to change this process's priority\n".to_string(),
                    1,
                );
            }
            #[cfg(unix)]
            unsafe {
                libc::setpriority(libc::PRIO_PROCESS, pid, priority);
            }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_nohup(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("nohup: missing command\n".to_string(), 1);
        }
        let cmd = args[0];
        let cmd_args: Vec<&str> = args[1..]
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        let vfs_root = self.vfs.root().to_path_buf();
        let cwd = if self.cwd == "/" {
            vfs_root.clone()
        } else {
            vfs_root.join(self.cwd.trim_start_matches('/'))
        };
        let out_path = cwd.join("nohup.out");
        // Only spawn when allowed (mobile → in-process via the executor's
        // `nohup CMD` handling; spawning here could SIGSYS).
        if !self.allow_subprocess {
            return CommandOutput::error(
                format!("nohup: cannot run '{cmd}' (external commands are disabled)\n"),
                127,
            );
        }
        let out_file = match std::fs::File::create(&out_path) {
            Ok(f) => f,
            Err(_) => {
                return CommandOutput::error(
                    format!("nohup: cannot create {}\n", out_path.display()),
                    1,
                )
            }
        };
        let child = match std::process::Command::new(cmd)
            .args(&cmd_args)
            .current_dir(&cwd)
            .stdout(std::process::Stdio::from(out_file))
            .stderr(std::process::Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("nohup: {}\n", e), 1),
        };
        CommandOutput::success(format!(
            "nohup: appending output to '{}', pid {}\n",
            out_path.display(),
            child.id()
        ))
    }

    pub fn cmd_chroot(&self, args: &[&str]) -> CommandOutput {
        if args.len() < 2 {
            return CommandOutput::error("chroot: missing operand\n".to_string(), 1);
        }
        let newroot = args[0];
        let cmd = args[1];
        let cmd_args: Vec<&str> = args[2..].to_vec();
        let resolved = match self.vfs.resolve(newroot, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("chroot: {}: {}\n", newroot, e), 1),
        };
        // Never call the privileged chroot(2): on Android the app seccomp policy
        // kills the process (SIGSYS), and the unconditional chdir("/") would
        // change the whole process cwd. Approximate by running the command with
        // the target directory as its working directory.
        if !self.allow_subprocess {
            return CommandOutput::error(
                format!("chroot: cannot run '{cmd}' (external commands are disabled)\n"),
                127,
            );
        }
        let output = std::process::Command::new(cmd)
            .args(&cmd_args)
            .current_dir(&resolved)
            .output();
        match output {
            Ok(o) => CommandOutput {
                stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                stderr: String::from_utf8_lossy(&o.stderr).to_string(),
                exit_code: o.status.code().unwrap_or(-1),
            },
            Err(e) => CommandOutput::error(format!("chroot: {}\n", e), 1),
        }
    }

    pub fn cmd_mkfifo(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("mkfifo: missing operand\n".to_string(), 1);
        }
        #[cfg(unix)]
        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("mkfifo: {}: {}\n", file, e), 1),
            };
            let path_c = match std::ffi::CString::new(resolved.to_string_lossy().as_bytes()) {
                Ok(c) => c,
                Err(_) => {
                    return CommandOutput::error("invalid path (contains NUL)\n".to_string(), 1)
                }
            };
            if unsafe { libc::mkfifo(path_c.as_ptr(), 0o666) } != 0 {
                return CommandOutput::error(
                    format!("mkfifo: {}: {}\n", file, std::io::Error::last_os_error()),
                    1,
                );
            }
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
                "-m" => {
                    if i + 1 < args.len() {
                        mode = Some(u32::from_str_radix(args[i + 1], 8).unwrap_or(0o755));
                        i += 1;
                    }
                }
                "-d" => dir = true,
                arg if arg.starts_with("-m") && arg.len() > 2 => {
                    mode = u32::from_str_radix(&arg[2..], 8).ok();
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }
        if files.len() < 2 {
            return CommandOutput::error("install: missing file operand\n".to_string(), 1);
        }
        let dest = files.pop().unwrap();
        for src in &files {
            let src_bytes = match self.vfs.read(src, &self.cwd) {
                Ok(b) => b,
                Err(e) => return CommandOutput::error(format!("install: {}: {}\n", src, e), 1),
            };
            let dest_path = if dir {
                let name = std::path::Path::new(src).file_name().unwrap_or_default();
                format!("{}/{}", dest, name.to_string_lossy())
            } else {
                dest.clone()
            };
            if let Err(e) = self.vfs.write_bytes(&dest_path, &self.cwd, &src_bytes) {
                return CommandOutput::error(format!("install: {}: {}\n", dest_path, e), 1);
            }
            if let Some(m) = mode {
                let resolved = self.vfs.resolve(&dest_path, &self.cwd).ok();
                #[cfg(unix)]
                if let Some(ref r) = resolved {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(r, std::fs::Permissions::from_mode(m)).ok();
                }
            }
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_shred(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("shred: missing file operand\n".to_string(), 1);
        }
        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("shred: {}: {}\n", file, e), 1),
            };
            let len = resolved.metadata().map(|m| m.len()).unwrap_or(0);
            let mut f = match std::fs::OpenOptions::new().write(true).open(&resolved) {
                Ok(f) => f,
                Err(e) => return CommandOutput::error(format!("shred: {}: {}\n", file, e), 1),
            };
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
                "-l" => {
                    if i + 1 < args.len() {
                        length = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }
        if files.is_empty() {
            return CommandOutput::error("fallocate: missing file operand\n".to_string(), 1);
        }
        let len = length.unwrap_or(0);
        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("fallocate: {}: {}\n", file, e), 1),
            };
            let f = match std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&resolved)
            {
                Ok(f) => f,
                Err(e) => return CommandOutput::error(format!("fallocate: {}: {}\n", file, e), 1),
            };
            f.set_len(len).ok();
        }
        CommandOutput::success(String::new())
    }

    pub fn cmd_telnet(&self, args: &[&str]) -> CommandOutput {
        let args: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if args.is_empty() {
            return CommandOutput::error("telnet: missing host\n".to_string(), 1);
        }
        run_system_cmd(self, "telnet", &args)
    }

    pub fn cmd_traceroute(&self, args: &[&str]) -> CommandOutput {
        let args: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if args.is_empty() {
            return CommandOutput::error("traceroute: missing host\n".to_string(), 1);
        }
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
        if args.is_empty() && stdin.is_none() {
            return CommandOutput::error("patch: missing input\n".to_string(), 1);
        }
        let s_args: Vec<&str> = args.to_vec();
        run_system_cmd(self, "patch", &s_args)
    }

    pub fn cmd_mknod(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.len() < 2 {
            return CommandOutput::error("mknod: missing operand\n".to_string(), 1);
        }
        // mknod(2) is a privileged syscall; Android's app seccomp policy kills
        // the process (SIGSYS). Refuse instead of crashing.
        CommandOutput::error(
            "mknod: operation not permitted in the sandbox\n".to_string(),
            1,
        )
    }

    pub fn cmd_mount(&self, args: &[&str]) -> CommandOutput {
        run_system_cmd(self, "mount", args)
    }

    pub fn cmd_umount(&self, args: &[&str]) -> CommandOutput {
        run_system_cmd(self, "umount", args)
    }
}

fn run_system_cmd(shell: &Shell, cmd: &str, args: &[&str]) -> CommandOutput {
    // Honor `allow_subprocess`: on mobile this never spawns (avoids SIGSYS).
    shell.run_external(cmd, args)
}
