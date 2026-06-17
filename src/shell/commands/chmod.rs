use crate::shell::{Shell, CommandOutput};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct SymbolicClause {
    who_owner: bool,
    who_group: bool,
    who_other: bool,
    op: Op,
    perm_read: bool,
    perm_write: bool,
    perm_exec: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Add,
    Remove,
    Set,
}

enum ChmodAction {
    Octal(u32),
    Symbolic(Vec<SymbolicClause>),
}

impl SymbolicClause {
    fn apply(&self, current_mode: u32) -> u32 {
        let mut mask: u32 = 0;
        if self.perm_read {
            if self.who_owner { mask |= 0o400; }
            if self.who_group { mask |= 0o040; }
            if self.who_other { mask |= 0o004; }
        }
        if self.perm_write {
            if self.who_owner { mask |= 0o200; }
            if self.who_group { mask |= 0o020; }
            if self.who_other { mask |= 0o002; }
        }
        if self.perm_exec {
            if self.who_owner { mask |= 0o100; }
            if self.who_group { mask |= 0o010; }
            if self.who_other { mask |= 0o001; }
        }

        let mut who_mask: u32 = 0;
        if self.who_owner { who_mask |= 0o700; }
        if self.who_group { who_mask |= 0o070; }
        if self.who_other { who_mask |= 0o007; }

        match self.op {
            Op::Add => current_mode | mask,
            Op::Remove => current_mode & !mask,
            Op::Set => {
                // Also clear setuid/setgid/sticky bits that fall within who_mask
                let cleared = current_mode & !who_mask;
                cleared | mask
            }
        }
    }
}

fn parse_symbolic_mode(mode_str: &str) -> Result<Vec<SymbolicClause>, String> {
    let mut clauses = Vec::new();
    for part in mode_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        clauses.push(parse_one_clause(part)?);
    }
    Ok(clauses)
}

fn parse_one_clause(s: &str) -> Result<SymbolicClause, String> {
    let op_idx = s.find(|c| c == '+' || c == '-' || c == '=');

    let (who_str, rest) = match op_idx {
        Some(pos) => s.split_at(pos),
        None => return Err(format!("chmod: {}: invalid mode", s)),
    };

    let op_char = rest.chars().next().unwrap();
    let perm_str = &rest[1..];

    let op = match op_char {
        '+' => Op::Add,
        '-' => Op::Remove,
        '=' => Op::Set,
        _ => return Err(format!("chmod: {}: invalid operator", s)),
    };

    let (who_owner, who_group, who_other) = if who_str.is_empty() {
        (true, true, true)
    } else {
        let mut u = false;
        let mut g = false;
        let mut o = false;
        for ch in who_str.chars() {
            match ch {
                'u' => u = true,
                'g' => g = true,
                'o' => o = true,
                'a' => { u = true; g = true; o = true; }
                _ => return Err(format!("chmod: {}: invalid who", s)),
            }
        }
        (u, g, o)
    };

    let mut perm_read = false;
    let mut perm_write = false;
    let mut perm_exec = false;
    for ch in perm_str.chars() {
        match ch {
            'r' => perm_read = true,
            'w' => perm_write = true,
            'x' => perm_exec = true,
            _ => return Err(format!("chmod: {}: invalid permission", s)),
        }
    }
    if !perm_read && !perm_write && !perm_exec {
        return Err(format!("chmod: {}: missing permission", s));
    }

    Ok(SymbolicClause {
        who_owner,
        who_group,
        who_other,
        op,
        perm_read,
        perm_write,
        perm_exec,
    })
}

fn is_symbolic(s: &str) -> bool {
    s.chars().any(|c| c == '+' || c == '-' || c == '=' || c == ',')
}

fn is_octal(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() && s.len() <= 4
}

fn parse_chmod_action(mode_str: &str) -> Result<ChmodAction, String> {
    if is_octal(mode_str) {
        let m = u32::from_str_radix(mode_str, 8)
            .map_err(|_| format!("chmod: {}: invalid mode", mode_str))?;
        Ok(ChmodAction::Octal(m))
    } else if is_symbolic(mode_str) {
        let clauses = parse_symbolic_mode(mode_str)?;
        Ok(ChmodAction::Symbolic(clauses))
    } else {
        Err(format!("chmod: {}: invalid mode", mode_str))
    }
}

fn compute_mode(action: &ChmodAction, current_mode: u32) -> u32 {
    match action {
        ChmodAction::Octal(m) => *m,
        ChmodAction::Symbolic(clauses) => {
            let mut mode = current_mode;
            for clause in clauses {
                mode = clause.apply(mode);
            }
            mode
        }
    }
}

fn get_entries_recursive(base: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = vec![base.to_path_buf()];
    if base.is_dir() {
        let children: Vec<PathBuf> = {
            let rd = fs::read_dir(base).map_err(|e| format!("{}", e))?;
            rd.filter_map(|e| e.ok()).map(|e| e.path()).collect()
        };
        for child in &children {
            let sub = get_entries_recursive(child)?;
            result.extend(sub);
        }
    }
    Ok(result)
}

impl Shell {
    pub fn cmd_chmod(&self, args: &[&str]) -> CommandOutput {
        let mut recursive = false;
        let mut idx = 0;

        while idx < args.len() && args[idx] == "-R" {
            recursive = true;
            idx += 1;
        }

        if idx >= args.len() {
            return CommandOutput::error("chmod: missing operand\n".to_string(), 1);
        }

        let mode_str = args[idx];
        let file_args = &args[(idx + 1)..];

        if file_args.is_empty() {
            return CommandOutput::error("chmod: missing operand\n".to_string(), 1);
        }

        #[cfg(not(unix))]
        {
            return CommandOutput::error(
                "chmod: not supported on this platform\n".to_string(),
                1,
            );
        }

        #[cfg(unix)]
        {
            let action = match parse_chmod_action(mode_str) {
                Ok(a) => a,
                Err(e) => return CommandOutput::error(format!("{}\n", e), 1),
            };

            for path in file_args {
                let target = match self.vfs.resolve(path, &self.cwd) {
                    Ok(p) => p,
                    Err(e) => {
                        return CommandOutput::error(
                            format!("chmod: {}: {}\n", path, e),
                            1,
                        );
                    }
                };

                let entries = if recursive {
                    let mut entries = match get_entries_recursive(&target) {
                        Ok(e) => e,
                        Err(e) => {
                            return CommandOutput::error(
                                format!("chmod: {}: {}\n", path, e),
                                1,
                            );
                        }
                    };
                    // Reverse so child entries (deeper paths) are processed before their parents
                    // to avoid losing directory execute permission needed to access children.
                    entries.reverse();
                    entries
                } else {
                    vec![target]
                };

                for entry in &entries {
                    use std::os::unix::fs::PermissionsExt;
                    let current = entry
                        .metadata()
                        .map(|m| m.permissions().mode())
                        .unwrap_or(0o644);
                    let new_mode = compute_mode(&action, current);
                    let perms = fs::Permissions::from_mode(new_mode);
                    if let Err(e) = fs::set_permissions(entry, perms) {
                        return CommandOutput::error(
                            format!("chmod: {}: {}\n", path, e),
                            1,
                        );
                    }
                }
            }

            CommandOutput::success(String::new())
        }
    }
}
