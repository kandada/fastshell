use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_test(&self, args: &[&str]) -> CommandOutput {
        let args: Vec<&str> = args.iter().filter(|&&a| a != "[" && a != "]").copied().collect();
        let result = evaluate_test(&args, self);
        match result {
            Some(true) => CommandOutput::success(String::new()),
            Some(false) => CommandOutput { stdout: String::new(), stderr: String::new(), exit_code: 1 },
            None => CommandOutput { stdout: String::new(), stderr: String::new(), exit_code: 2 },
        }
    }
}

fn evaluate_test(args: &[&str], shell: &Shell) -> Option<bool> {
    if args.is_empty() {
        return Some(false);
    }

    if args[0] == "!" {
        return evaluate_test(&args[1..], shell).map(|v| !v);
    }

    match args.len() {
        1 => Some(!args[0].is_empty()),
        2 => {
            match args[0] {
                "-z" => Some(args[1].is_empty()),
                "-n" => Some(!args[1].is_empty()),
                "-d" => Some(shell.vfs.is_dir(args[1], &shell.cwd)),
                "-f" => Some(shell.vfs.is_file(args[1], &shell.cwd)),
                "-e" => Some(shell.vfs.exists(args[1], &shell.cwd)),
                "-s" => {
                    match shell.vfs.resolve(args[1], &shell.cwd) {
                        Ok(p) => Some(p.metadata().map(|m| m.len() > 0).unwrap_or(false)),
                        Err(_) => Some(false),
                    }
                }
                _ => None,
            }
        }
        3 => {
            match args[1] {
                "=" => Some(args[0] == args[2]),
                "!=" => Some(args[0] != args[2]),
                "==" => Some(args[0] == args[2]),
                "-eq" => {
                    let a = args[0].parse::<i64>().ok();
                    let b = args[2].parse::<i64>().ok();
                    if a.is_none() || b.is_none() { return None; }
                    Some(a == b)
                }
                "-ne" => {
                    let a = args[0].parse::<i64>().ok();
                    let b = args[2].parse::<i64>().ok();
                    if a.is_none() || b.is_none() { return None; }
                    Some(a != b)
                }
                "-lt" => {
                    let a = args[0].parse::<i64>().ok();
                    let b = args[2].parse::<i64>().ok();
                    if a.is_none() || b.is_none() { return None; }
                    Some(a < b)
                }
                "-le" => {
                    let a = args[0].parse::<i64>().ok();
                    let b = args[2].parse::<i64>().ok();
                    if a.is_none() || b.is_none() { return None; }
                    Some(a <= b)
                }
                "-gt" => {
                    let a = args[0].parse::<i64>().ok();
                    let b = args[2].parse::<i64>().ok();
                    if a.is_none() || b.is_none() { return None; }
                    Some(a > b)
                }
                "-ge" => {
                    let a = args[0].parse::<i64>().ok();
                    let b = args[2].parse::<i64>().ok();
                    if a.is_none() || b.is_none() { return None; }
                    Some(a >= b)
                }
                _ => None,
            }
        }
        _ => None,
    }
}
