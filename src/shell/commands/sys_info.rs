use crate::shell::{Shell, CommandOutput, list_processes};

fn get_hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = vec![0u8; 256];
        let ret = unsafe {
            libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len())
        };
        if ret == 0 {
            let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..len]).to_string()
        } else {
            "unknown".to_string()
        }
    }
    #[cfg(not(unix))]
    {
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".to_string())
    }
}

impl Shell {
    pub fn cmd_uname(&self, args: &[&str]) -> CommandOutput {
        let mut show_all = true;
        let mut show_kernel = false;
        let mut show_node = false;
        let mut show_release = false;
        let mut show_machine = false;

        for arg in args {
            match *arg {
                "-a" => show_all = true,
                "-s" => { show_kernel = true; show_all = false; }
                "-n" => { show_node = true; show_all = false; }
                "-r" => { show_release = true; show_all = false; }
                "-m" => { show_machine = true; show_all = false; }
                _ => {}
            }
        }

        let kernel = std::env::consts::OS;
        let machine = std::env::consts::ARCH;

        let node = get_hostname();

        let release = "1.0";

        let mut parts = Vec::new();
        if show_all {
            parts.push(kernel.to_string());
            parts.push(node);
            parts.push(release.to_string());
            parts.push(machine.to_string());
        } else {
            if show_kernel { parts.push(kernel.to_string()); }
            if show_node { parts.push(node); }
            if show_release { parts.push(release.to_string()); }
            if show_machine { parts.push(machine.to_string()); }
        }

        CommandOutput::success(parts.join(" ") + "\n")
    }

    pub fn cmd_hostname(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success(get_hostname() + "\n")
    }

    pub fn cmd_whoami(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(unix)]
        {
            let uid = unsafe { libc::getuid() };
            let pw = unsafe { libc::getpwuid(uid) };
            if pw.is_null() {
                return CommandOutput::error("whoami: cannot find username\n".to_string(), 1);
            }
            let name = unsafe {
                std::ffi::CStr::from_ptr((*pw).pw_name)
                    .to_string_lossy()
                    .to_string()
            };
            CommandOutput::success(name + "\n")
        }
        #[cfg(not(unix))]
        {
            CommandOutput::success(
                std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "unknown".to_string())
                    + "\n",
            )
        }
    }

    pub fn cmd_id(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(unix)]
        {
            let uid = unsafe { libc::getuid() };
            let gid = unsafe { libc::getgid() };
            let pw = unsafe { libc::getpwuid(uid) };
            let name = if pw.is_null() {
                uid.to_string()
            } else {
                unsafe {
                    std::ffi::CStr::from_ptr((*pw).pw_name)
                        .to_string_lossy()
                        .to_string()
                }
            };
            CommandOutput::success(format!("uid={}({}) gid={}\n", uid, name, gid))
        }
        #[cfg(not(unix))]
        {
            let user = std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".to_string());
            CommandOutput::success(format!("uid=1000({}) gid=1000\n", user))
        }
    }

    pub fn cmd_pgrep(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("pgrep: missing pattern\n".to_string(), 1);
        }

        let pattern = args.iter()
            .find(|a| !a.starts_with('-'))
            .copied()
            .unwrap_or("");
        let pattern_lower = pattern.to_lowercase();

        let procs = match list_processes() {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("pgrep: {}\n", e), 1),
        };

        let mut output = String::new();
        for proc in &procs {
            if proc.comm.to_lowercase().contains(&pattern_lower) {
                output.push_str(&format!("{}\n", proc.pid));
            }
        }

        if output.is_empty() {
            CommandOutput { stdout: String::new(), stderr: String::new(), exit_code: 1 }
        } else {
            CommandOutput::success(output)
        }
    }

    pub fn cmd_pkill(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("pkill: missing pattern\n".to_string(), 1);
        }

        let mut signal = 15i32;
        let mut pattern = String::new();

        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                if let Ok(sig) = arg[1..].parse::<i32>() {
                    signal = sig;
                }
            } else if !arg.starts_with('-') {
                pattern = arg.to_string();
            }
        }

        if pattern.is_empty() {
            return CommandOutput::error("pkill: missing pattern\n".to_string(), 1);
        }

        let pattern_lower = pattern.to_lowercase();
        let procs = match list_processes() {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("pkill: {}\n", e), 1),
        };

        let mut output = String::new();
        for proc in &procs {
            if proc.comm.to_lowercase().contains(&pattern_lower) {
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(proc.pid as i32, signal);
                    }
                }
                output.push_str(&format!("killed {} (pid {})\n", proc.comm, proc.pid));
            }
        }

        CommandOutput::success(output)
    }
}
