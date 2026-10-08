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
        // Flags: `-l`/`--mathlib` sets scale=20; `-q`/`-s`/`-w` are tolerated.
        let mut scale: usize = 0;
        let mut operands: Vec<&str> = Vec::new();
        for a in args {
            match *a {
                "-l" | "--mathlib" => scale = 20,
                "-q" | "--quiet" | "-s" | "--standard" | "-w" | "--warn" => {}
                _ => operands.push(a),
            }
        }
        let input: String = if operands.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("bc: missing expression\n".to_string(), 1),
            }
        } else {
            operands.join(" ")
        };

        let mut output = String::new();
        let mut ibase: u32 = 10;
        let mut obase: u32 = 10;
        for line in input.lines() {
            // bc accepts several `;`-separated statements per line.
            for stmt in line.split(';') {
                let stmt = stmt.trim();
                if stmt.is_empty() || stmt == "quit" {
                    continue;
                }
                if let Some(v) = stmt.strip_prefix("scale=") {
                    if let Ok(n) = v.trim().parse::<usize>() {
                        scale = n;
                    }
                    continue;
                }
                if let Some(v) = stmt.strip_prefix("ibase=") {
                    if let Ok(n) = v.trim().parse::<u32>() {
                        if (2..=16).contains(&n) {
                            ibase = n;
                        }
                    }
                    continue;
                }
                if let Some(v) = stmt.strip_prefix("obase=") {
                    if let Ok(n) = v.trim().parse::<u32>() {
                        if (2..=16).contains(&n) {
                            obase = n;
                        }
                    }
                    continue;
                }
                let expanded = bc_expand_math(stmt);
                let expanded = if ibase != 10 {
                    bc_convert_input_base(&expanded, ibase)
                } else {
                    expanded
                };
                match eval_bc_expr(&expanded) {
                    Ok(val) => {
                        output.push_str(&format!("{}\n", format_bc_val_obase(val, scale, obase)))
                    }
                    Err(e) => output.push_str(&format!("bc: {}\n", e)),
                }
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
        let host = args
            .iter()
            .find(|a| !a.starts_with('-'))
            .copied()
            .unwrap_or("");
        if host.is_empty() {
            return CommandOutput::error("dig: missing hostname\n".to_string(), 1);
        }
        if let Some(perm) = self.check_network_permission(host) {
            return perm;
        }
        match self.dns_resolve(host) {
            Ok(addrs) => {
                let mut out = format!("; <<>> DiG (fastshell builtin) <<>> {}\n", host);
                out.push_str(";; ANSWER SECTION:\n");
                for a in &addrs {
                    let kind = if a.is_ipv4() { "A" } else { "AAAA" };
                    out.push_str(&format!("{}\tIN\t{}\t{}\n", host, kind, a.ip()));
                }
                CommandOutput::success(out)
            }
            Err(e) => CommandOutput::error(format!("dig: {}\n", e), 1),
        }
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
    // Honor `allow_subprocess`: on mobile this never spawns (avoids SIGSYS).
    shell.run_external(cmd, args)
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

/// Format a result honoring `scale` (number of fractional digits). `scale == 0`
/// keeps the legacy formatting (full precision) to avoid changing existing output.
fn digit_char(d: u32) -> char {
    if d < 10 {
        (b'0' + d as u8) as char
    } else {
        (b'A' + (d - 10) as u8) as char
    }
}

fn format_bc_int_base(mut n: i64, base: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(digit_char((n % base as i64) as u32));
        n /= base as i64;
    }
    out.iter().rev().collect()
}

/// Format a bc value in `obase` (2..=16); base 10 uses the scaled decimal form.
fn format_bc_val_obase(v: f64, scale: usize, obase: u32) -> String {
    if obase == 10 {
        return format_bc_val_scaled(v, scale);
    }
    let neg = v < 0.0;
    let a = v.abs();
    let int_part = a.trunc() as i64;
    let frac = a - int_part as f64;
    let mut s = format_bc_int_base(int_part, obase);
    if scale > 0 && frac > 0.0 {
        s.push('.');
        let mut f = frac;
        for _ in 0..scale {
            f *= obase as f64;
            let d = f.trunc() as u32;
            s.push(digit_char(d));
            f -= d as f64;
        }
    }
    if neg {
        format!("-{}", s)
    } else {
        s
    }
}

/// Reinterpret decimal digit runs in `s` as base-`ibase` numbers (ibase < 10).
fn bc_convert_input_base(s: &str, ibase: u32) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphanumeric() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let tok: String = chars[start..i].iter().collect();
            let is_digits = tok.chars().all(|c| c.is_ascii_digit());
            let is_hex_like = ibase > 10
                && !tok.is_empty()
                && tok.chars().all(|c| c.is_ascii_hexdigit())
                && tok.chars().any(|c| c.is_ascii_uppercase());
            if is_digits || is_hex_like {
                if let Ok(v) = i64::from_str_radix(&tok, ibase) {
                    out.push_str(&v.to_string());
                    continue;
                }
            }
            out.push_str(&tok);
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn format_bc_val_scaled(v: f64, scale: usize) -> String {
    if scale == 0 {
        format_bc_val(v)
    } else {
        format!("{:.*}", scale, v)
    }
}

fn format_bc_val(v: f64) -> String {
    if (v - v.round()).abs() < 1e-10 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

/// Expands math-library calls (`sqrt`, `l`(ln), `e`(exp), `s`(sin), `c`(cos),
/// `a`(atan)) into numeric literals so `bc -l` math works with the plain
/// expression evaluator.
fn bc_expand_math(stmt: &str) -> String {
    let mut out = stmt.to_string();
    for name in ["sqrt", "l", "e", "s", "c", "a"] {
        while let Some(pos) = find_bc_call(&out, name) {
            let bytes = out.as_bytes();
            let mut depth = 0i32;
            let mut end = None;
            let mut i = pos;
            while i < bytes.len() {
                match bytes[i] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            let Some(end) = end else { break };
            let arg = &out[pos + name.len() + 1..end];
            let x = match eval_bc_expr(arg) {
                Ok(v) => v,
                Err(_) => break,
            };
            let v = match name {
                "sqrt" => x.sqrt(),
                "l" => x.ln(),
                "e" => x.exp(),
                "s" => x.sin(),
                "c" => x.cos(),
                "a" => x.atan(),
                _ => x,
            };
            out = format!("{}{}{}", &out[..pos], format_bc_val(v), &out[end + 1..]);
        }
    }
    out
}

fn find_bc_call(s: &str, name: &str) -> Option<usize> {
    let pat = format!("{name}(");
    let mut from = 0;
    while let Some(rel) = s[from..].find(&pat) {
        let pos = from + rel;
        let prev_ok = pos == 0
            || !(s.as_bytes()[pos - 1].is_ascii_alphanumeric() || s.as_bytes()[pos - 1] == b'_');
        if prev_ok {
            return Some(pos);
        }
        from = pos + 1;
    }
    None
}
