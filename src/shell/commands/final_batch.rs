// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_whois(&self, args: &[&str]) -> CommandOutput {
        let host = args
            .iter()
            .find(|a| !a.starts_with('-'))
            .copied()
            .unwrap_or("");
        if host.is_empty() {
            return CommandOutput::error("whois: missing hostname\n".to_string(), 1);
        }
        run_system(self, "whois", &[host])
    }

    pub fn cmd_hostid(&self, _args: &[&str]) -> CommandOutput {
        #[cfg(all(unix, not(target_os = "android")))]
        {
            let id = unsafe { libc::gethostid() };
            return CommandOutput::success(format!("{:08x}\n", id));
        }
        #[allow(unreachable_code)]
        CommandOutput::success("00000000\n".to_string())
    }

    pub fn cmd_bc(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let input: String;
        if args.is_empty() {
            if let Some(s) = stdin {
                input = s.to_string();
            } else {
                return CommandOutput::error("bc: missing expression\n".to_string(), 1);
            }
        } else {
            input = args.join(" ");
        }

        let mut output = String::new();
        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() || line == "quit" {
                continue;
            }
            match eval_bc_expr(line) {
                Ok(val) => output.push_str(&format!("{}\n", format_bc_val(val))),
                Err(e) => output.push_str(&format!("bc: {}\n", e)),
            }
        }
        CommandOutput::success(output)
    }

    pub fn cmd_iostat(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "iostat", &s_args)
    }

    pub fn cmd_vmstat(&self, args: &[&str]) -> CommandOutput {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let mut out = String::new();
            if let Ok(s) = std::fs::read_to_string("/proc/vmstat") {
                for line in s.lines().take(30) {
                    out.push_str(line);
                    out.push('\n');
                }
                return CommandOutput::success(out);
            }
        }
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "vmstat", &s_args)
    }

    pub fn cmd_lsblk(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "lsblk", &s_args)
    }

    pub fn cmd_lsof(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "lsof", &s_args)
    }

    pub fn cmd_dig(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "dig", &s_args)
    }

    pub fn cmd_rsync(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "rsync", &s_args)
    }

    pub fn cmd_hdparm(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "hdparm", &s_args)
    }

    pub fn cmd_smartctl(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "smartctl", &s_args)
    }

    pub fn cmd_blkid(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "blkid", &s_args)
    }

    pub fn cmd_lsusb(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "lsusb", &s_args)
    }

    pub fn cmd_ss(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "ss", &s_args)
    }

    pub fn cmd_ip(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "ip", &s_args)
    }

    pub fn cmd_ethtool(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "ethtool", &s_args)
    }

    pub fn cmd_service(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "service", &s_args)
    }

    pub fn cmd_showmount(&self, args: &[&str]) -> CommandOutput {
        let s_args: Vec<&str> = args.to_vec();
        run_system(self, "showmount", &s_args)
    }
}

fn run_system(shell: &Shell, cmd: &str, args: &[&str]) -> CommandOutput {
    let vfs_root = shell.vfs.root().to_path_buf();
    let cwd = if shell.cwd == "/" {
        vfs_root.clone()
    } else {
        vfs_root.join(shell.cwd.trim_start_matches('/'))
    };
    match std::process::Command::new(cmd)
        .args(args)
        .current_dir(&cwd)
        .output()
    {
        Ok(o) => CommandOutput {
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
            exit_code: o.status.code().unwrap_or(-1),
        },
        Err(e) => CommandOutput::error(format!("{}: {}\n", cmd, e), 1),
    }
}

fn eval_bc_expr(expr: &str) -> Result<f64, String> {
    let expr = expr.trim();
    if let Some(pos) = expr.find('+') {
        return Ok(eval_bc_expr(&expr[..pos])? + eval_bc_expr(&expr[pos + 1..])?);
    }
    if let Some(pos) = expr.rfind('-').filter(|&p| p > 0) {
        return Ok(eval_bc_expr(&expr[..pos])? - eval_bc_expr(&expr[pos + 1..])?);
    }
    if let Some(pos) = expr.find('*') {
        return Ok(eval_bc_expr(&expr[..pos])? * eval_bc_expr(&expr[pos + 1..])?);
    }
    if let Some(pos) = expr.find('/') {
        let b = eval_bc_expr(&expr[pos + 1..])?;
        if b == 0.0 {
            return Err("divide by zero".to_string());
        }
        return Ok(eval_bc_expr(&expr[..pos])? / b);
    }
    if let Some(pos) = expr.find('%') {
        let b = eval_bc_expr(&expr[pos + 1..])?;
        if b == 0.0 {
            return Err("divide by zero".to_string());
        }
        return Ok(eval_bc_expr(&expr[..pos])? % b);
    }
    if let Some(pos) = expr.find('^') {
        return Ok(eval_bc_expr(&expr[..pos])?.powf(eval_bc_expr(&expr[pos + 1..])?));
    }
    expr.trim()
        .parse::<f64>()
        .map_err(|_| format!("syntax error: {}", expr))
}

fn format_bc_val(v: f64) -> String {
    if (v - v.round()).abs() < 1e-10 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}
