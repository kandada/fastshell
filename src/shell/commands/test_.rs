// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_test(&self, args: &[&str]) -> CommandOutput {
        let args: Vec<&str> = args
            .iter()
            .filter(|&&a| a != "[" && a != "]")
            .copied()
            .collect();
        let result = evaluate_test(&args, self);
        match result {
            Some(true) => CommandOutput::success(String::new()),
            Some(false) => CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 1,
            },
            None => CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 2,
            },
        }
    }
}

fn evaluate_test(args: &[&str], shell: &Shell) -> Option<bool> {
    if args.is_empty() {
        return Some(false);
    }

    // `( expr )` grouping, when the whole expression is wrapped.
    if args[0] == "(" {
        let mut depth = 0i32;
        let mut end = None;
        for (i, a) in args.iter().enumerate() {
            match *a {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        if end == Some(args.len() - 1) {
            return evaluate_test(&args[1..args.len() - 1], shell);
        }
    }

    if args[0] == "!" {
        return evaluate_test(&args[1..], shell).map(|v| !v);
    }

    // `-o` (OR) has lower precedence than `-a` (AND); both short-circuit.
    if let Some(i) = find_top_level(args, "-o") {
        if evaluate_test(&args[..i], shell) == Some(true) {
            return Some(true);
        }
        return evaluate_test(&args[i + 1..], shell);
    }
    if let Some(i) = find_top_level(args, "-a") {
        if evaluate_test(&args[..i], shell) == Some(false) {
            return Some(false);
        }
        return evaluate_test(&args[i + 1..], shell);
    }

    match args.len() {
        1 => Some(!args[0].is_empty()),
        2 => match args[0] {
            "-z" => Some(args[1].is_empty()),
            "-n" => Some(!args[1].is_empty()),
            "-d" => Some(shell.vfs.is_dir(args[1], &shell.cwd)),
            "-f" => Some(shell.vfs.is_file(args[1], &shell.cwd)),
            "-e" => Some(shell.vfs.exists(args[1], &shell.cwd)),
            // `-L`/`-h`: inspect the link ITSELF (must not follow it).
            "-L" | "-h" => match shell.vfs.resolve_no_follow(args[1], &shell.cwd) {
                Ok(p) => Some(
                    std::fs::symlink_metadata(&p)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false),
                ),
                Err(_) => Some(false),
            },
            "-r" | "-w" | "-x" => match shell.vfs.resolve(args[1], &shell.cwd) {
                Ok(p) => Some(p.metadata().is_ok()),
                Err(_) => Some(false),
            },
            "-s" => match shell.vfs.resolve(args[1], &shell.cwd) {
                Ok(p) => Some(p.metadata().map(|m| m.len() > 0).unwrap_or(false)),
                Err(_) => Some(false),
            },
            _ => None,
        },
        3 => match args[1] {
            "=" => Some(args[0] == args[2]),
            "!=" => Some(args[0] != args[2]),
            "==" => Some(args[0] == args[2]),
            "-eq" => {
                let a = args[0].parse::<i64>().ok();
                let b = args[2].parse::<i64>().ok();
                if a.is_none() || b.is_none() {
                    return None;
                }
                Some(a == b)
            }
            "-ne" => {
                let a = args[0].parse::<i64>().ok();
                let b = args[2].parse::<i64>().ok();
                if a.is_none() || b.is_none() {
                    return None;
                }
                Some(a != b)
            }
            "-lt" => {
                let a = args[0].parse::<i64>().ok();
                let b = args[2].parse::<i64>().ok();
                if a.is_none() || b.is_none() {
                    return None;
                }
                Some(a < b)
            }
            "-le" => {
                let a = args[0].parse::<i64>().ok();
                let b = args[2].parse::<i64>().ok();
                if a.is_none() || b.is_none() {
                    return None;
                }
                Some(a <= b)
            }
            "-gt" => {
                let a = args[0].parse::<i64>().ok();
                let b = args[2].parse::<i64>().ok();
                if a.is_none() || b.is_none() {
                    return None;
                }
                Some(a > b)
            }
            "-ge" => {
                let a = args[0].parse::<i64>().ok();
                let b = args[2].parse::<i64>().ok();
                if a.is_none() || b.is_none() {
                    return None;
                }
                Some(a >= b)
            }
            "-nt" | "-ot" => {
                // file newer/older than: compare mtime.
                let mt = |p: &str| -> Option<std::time::SystemTime> {
                    shell
                        .vfs
                        .resolve(p, &shell.cwd)
                        .ok()
                        .and_then(|pb| pb.metadata().ok())
                        .and_then(|m| m.modified().ok())
                };
                match (mt(args[0]), mt(args[2])) {
                    (Some(a), Some(b)) => Some(if args[1] == "-nt" { a > b } else { a < b }),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// Index of the first top-level `op` token (ignoring tokens inside `( )`).
fn find_top_level(args: &[&str], op: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, a) in args.iter().enumerate() {
        match *a {
            "(" => depth += 1,
            ")" => depth -= 1,
            _ if depth == 0 && *a == op => return Some(i),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Vfs;

    fn mk_shell() -> Shell {
        let dir = std::env::temp_dir().join(format!(
            "fastshell_test_cmd_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_test_file_newer_than() {
        let mut s = mk_shell();
        s.vfs.write("old.txt", "/", "old").unwrap();
        s.vfs.write("new.txt", "/", "new").unwrap();
        // Ensure different mtimes by touching old first.
        let out = s.cmd_test(&["new.txt", "-nt", "old.txt"]);
        assert_eq!(out.exit_code, 0, "new.txt should be newer");
    }

    #[test]
    fn test_test_file_older_than() {
        let mut s = mk_shell();
        s.vfs.write("old.txt", "/", "old").unwrap();
        s.vfs.write("new.txt", "/", "new").unwrap();
        let out = s.cmd_test(&["old.txt", "-ot", "new.txt"]);
        assert_eq!(out.exit_code, 0, "old.txt should be older");
    }

    #[test]
    fn test_test_not_symlink() {
        let mut s = mk_shell();
        s.vfs.write("plain.txt", "/", "x").unwrap();
        // A regular file is not a symlink → -L returns false → exit 1.
        let out = s.cmd_test(&["-L", "plain.txt"]);
        assert_eq!(out.exit_code, 1, "plain file should not be a symlink");
    }
}
