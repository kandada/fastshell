use crate::shell::{Shell, CommandOutput};
use std::process::Command as ProcessCommand;
use std::time::SystemTime;

#[derive(Debug, Clone)]
enum ConditionKind {
    Name(String),
    Type(char),
    Mtime { days: i64, greater_than: bool },
    Size { bytes: i64, greater_than: bool },
}

#[derive(Debug, Clone)]
struct Condition {
    negate: bool,
    kind: ConditionKind,
}

#[derive(Debug, Clone)]
enum Action {
    Print,
    Print0,
    Exec(Vec<String>),
}

impl Shell {
    pub fn cmd_find(&self, args: &[&str]) -> CommandOutput {
        let mut path = ".".to_string();
        let mut path_set = false;
        let mut conditions: Vec<Vec<Condition>> = vec![vec![]];
        let mut actions: Vec<Action> = Vec::new();
        let mut maxdepth: Option<usize> = None;
        let mut i = 0;
        let mut negate_next = false;

        while i < args.len() {
            match args[i] {
                "-maxdepth" => {
                    if i + 1 < args.len() {
                        if let Ok(d) = args[i + 1].parse::<usize>() {
                            maxdepth = Some(d);
                        }
                        i += 1;
                    }
                }
                "-name" => {
                    if i + 1 < args.len() {
                        let pat = args[i + 1].to_string();
                        add_condition(
                            &mut conditions,
                            Condition { negate: negate_next, kind: ConditionKind::Name(pat) },
                        );
                        negate_next = false;
                        i += 1;
                    }
                }
                "-type" => {
                    if i + 1 < args.len() {
                        if let Some(ch) = args[i + 1].chars().next() {
                            add_condition(
                                &mut conditions,
                                Condition { negate: negate_next, kind: ConditionKind::Type(ch) },
                            );
                        }
                        negate_next = false;
                        i += 1;
                    }
                }
                "-mtime" => {
                    if i + 1 < args.len() {
                        if let Some(cond) = parse_mtime(args[i + 1]) {
                            add_condition(
                                &mut conditions,
                                Condition { negate: negate_next, kind: cond },
                            );
                        }
                        negate_next = false;
                        i += 1;
                    }
                }
                "-size" => {
                    if i + 1 < args.len() {
                        if let Some(cond) = parse_size(args[i + 1]) {
                            add_condition(
                                &mut conditions,
                                Condition { negate: negate_next, kind: cond },
                            );
                        }
                        negate_next = false;
                        i += 1;
                    }
                }
                "-print0" => {
                    actions.push(Action::Print0);
                }
                "-exec" => {
                    i += 1;
                    let mut exec_args: Vec<String> = Vec::new();
                    while i < args.len() && args[i] != ";" && args[i] != "\\;" {
                        exec_args.push(args[i].to_string());
                        i += 1;
                    }
                    if !exec_args.is_empty() {
                        actions.push(Action::Exec(exec_args));
                    }
                }
                "-o" => {
                    conditions.push(vec![]);
                }
                "!" | "-not" => {
                    negate_next = !negate_next;
                }
                arg if !arg.starts_with('-') && !path_set => {
                    path = arg.to_string();
                    path_set = true;
                }
                _ => {}
            }
            i += 1;
        }

        if actions.is_empty() {
            actions.push(Action::Print);
        }

        let compiled_conditions: Vec<Vec<(bool, Option<regex::Regex>, ConditionKind)>> = conditions
            .iter()
            .map(|group| {
                group
                    .iter()
                    .map(|c| {
                        let compiled = match &c.kind {
                            ConditionKind::Name(pat) => Some(compile_glob(pat)),
                            _ => None,
                        };
                        (c.negate, compiled, c.kind.clone())
                    })
                    .collect()
            })
            .collect();

        let mut output = String::new();
        let mut exit_code = 0;

        // Check the starting path itself
        if let Ok(resolved) = self.vfs.resolve(&path, &self.cwd) {
            if let Ok(metadata) = resolved.metadata() {
                let entry_name = resolved
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                let start_entry = crate::vfs::DirEntry {
                    name: entry_name.clone(),
                    is_dir: metadata.is_dir(),
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                };
                if evaluate_conditions(&compiled_conditions, entry_name, &start_entry) {
                    apply_actions(&actions, &path, &mut output, &mut exit_code);
                }
            }
        }

        if let Err(e) = self.find_recursive(
            &path,
            0,
            maxdepth,
            &compiled_conditions,
            &actions,
            &mut output,
            &mut exit_code,
        ) {
            return CommandOutput::error(format!("find: {}\n", e), 1);
        }
        CommandOutput {
            stdout: output,
            stderr: String::new(),
            exit_code,
        }
    }

    fn find_recursive(
        &self,
        path: &str,
        depth: usize,
        maxdepth: Option<usize>,
        conditions: &[Vec<(bool, Option<regex::Regex>, ConditionKind)>],
        actions: &[Action],
        output: &mut String,
        exit_code: &mut i32,
    ) -> Result<(), crate::vfs::VfsError> {
        if let Some(md) = maxdepth {
            if depth >= md {
                return Ok(());
            }
        }
        let entries = self.vfs.list_dir(path, &self.cwd)?;

        for entry in &entries {
            let entry_path = format!("{}/{}", path.trim_end_matches('/'), entry.name);

            if evaluate_conditions(conditions, entry.name.clone(), entry) {
                apply_actions(actions, &entry_path, output, exit_code);
            }

            if entry.is_dir {
                if maxdepth.map_or(true, |md| depth < md) {
                    let _ = self.find_recursive(
                        &entry_path,
                        depth + 1,
                        maxdepth,
                        conditions,
                        actions,
                        output,
                        exit_code,
                    );
                }
            }
        }

        Ok(())
    }
}

fn apply_actions(actions: &[Action], entry_path: &str, output: &mut String, exit_code: &mut i32) {
    for action in actions {
        match action {
            Action::Print => {
                output.push_str(&format!("{}\n", entry_path));
            }
            Action::Print0 => {
                output.push_str(entry_path);
                output.push('\0');
            }
            Action::Exec(cmd_args) => {
                let args: Vec<String> = cmd_args
                    .iter()
                    .map(|a| {
                        if a == "{}" {
                            entry_path.to_string()
                        } else {
                            a.clone()
                        }
                    })
                    .collect();

                let mut cmd = ProcessCommand::new(&args[0]);
                if args.len() > 1 {
                    cmd.args(&args[1..]);
                }
                match cmd.output() {
                    Ok(out) => {
                        if out.status.success() {
                            output.push_str(&String::from_utf8_lossy(&out.stdout));
                        } else {
                            *exit_code = out.status.code().unwrap_or(1);
                        }
                    }
                    Err(e) => {
                        *exit_code = 1;
                        output.push_str(&format!(
                            "find: '{}' failed: {}\n",
                            args[0], e
                        ));
                    }
                }
            }
        }
    }
}

fn evaluate_conditions(
    conditions: &[Vec<(bool, Option<regex::Regex>, ConditionKind)>],
    name: String,
    entry: &crate::vfs::DirEntry,
) -> bool {
    if conditions.is_empty() || conditions.iter().all(|g| g.is_empty()) {
        return true;
    }
    for group in conditions {
        if group.is_empty() {
            continue;
        }
        let group_match = group.iter().all(|(negate, compiled, kind)| {
            let result = match kind {
                ConditionKind::Name(_) => match compiled {
                    Some(re) => re.is_match(&name),
                    None => true,
                },
                ConditionKind::Type(ch) => match ch {
                    'd' => entry.is_dir,
                    'f' => !entry.is_dir,
                    _ => true,
                },
                ConditionKind::Mtime { days, greater_than } => {
                    match entry.modified {
                        Some(mod_time) => {
                            let now = SystemTime::now();
                            let file_age_secs = match now.duration_since(mod_time) {
                                Ok(d) => d.as_secs() as i64,
                                Err(_) => -1,
                            };
                            if file_age_secs < 0 {
                                return false;
                            }
                            let file_days = file_age_secs / 86400;
                            if *greater_than {
                                file_days > *days
                            } else {
                                file_days < *days
                            }
                        }
                        None => false,
                    }
                }
                ConditionKind::Size { bytes, greater_than } => {
                    let size = entry.size as i64;
                    if *greater_than {
                        size > *bytes
                    } else {
                        size < *bytes
                    }
                }
            };
            if *negate { !result } else { result }
        });

        if group_match {
            return true;
        }
    }

    false
}

fn add_condition(groups: &mut Vec<Vec<Condition>>, cond: Condition) {
    if groups.is_empty() {
        groups.push(vec![]);
    }
    let last = groups.last_mut().unwrap();
    last.push(cond);
}

fn parse_mtime(arg: &str) -> Option<ConditionKind> {
    if arg.len() < 2 {
        return None;
    }
    let (sign, num_str) = match arg.chars().next().unwrap() {
        '+' => (true, &arg[1..]),
        '-' => (false, &arg[1..]),
        _ => return None,
    };
    let days: i64 = num_str.parse().ok()?;
    Some(ConditionKind::Mtime {
        days,
        greater_than: sign,
    })
}

fn parse_size(arg: &str) -> Option<ConditionKind> {
    if arg.len() < 3 {
        return None;
    }
    let (greater_than, rest) = match arg.chars().next().unwrap() {
        '+' => (true, &arg[1..]),
        '-' => (false, &arg[1..]),
        _ => return None,
    };

    let (num_str, unit) = if rest.ends_with('k') || rest.ends_with('K') {
        (&rest[..rest.len() - 1], 'k')
    } else if rest.ends_with('M') {
        (&rest[..rest.len() - 1], 'M')
    } else if rest.ends_with('G') {
        (&rest[..rest.len() - 1], 'G')
    } else {
        return None;
    };

    let value: i64 = num_str.parse().ok()?;
    let bytes = match unit {
        'k' => value * 1024,
        'M' => value * 1024 * 1024,
        'G' => value * 1024 * 1024 * 1024,
        _ => value,
    };

    Some(ConditionKind::Size {
        bytes,
        greater_than,
    })
}

fn compile_glob(pattern: &str) -> regex::Regex {
    let mut regex_str = String::new();
    regex_str.push('^');
    for ch in pattern.chars() {
        match ch {
            '*' => regex_str.push_str(".*"),
            '?' => regex_str.push('.'),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                regex_str.push('\\');
                regex_str.push(ch);
            }
            _ => regex_str.push(ch),
        }
    }
    regex_str.push('$');
    regex::Regex::new(&regex_str).unwrap_or_else(|_| {
        let escaped = regex::escape(pattern);
        regex::Regex::new(&format!("^{}$", escaped)).unwrap()
    })
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fastshell_find_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        Shell::new(setup_vfs())
    }

    #[test]
    fn test_find_basic() {
        let shell = mk_shell();
        let r = shell.cmd_mkdir(&["a"]);
        assert_eq!(r.exit_code, 0, "mkdir a failed: {:?}", r);
        let r = shell.cmd_mkdir(&["a/b"]);
        assert_eq!(r.exit_code, 0, "mkdir a/b failed: {:?}", r);
        let r = shell.cmd_touch(&["a/file.txt"]);
        assert_eq!(r.exit_code, 0, "touch a/file.txt failed: {:?}", r);
        let r = shell.cmd_touch(&["a/b/nested.txt"]);
        assert_eq!(r.exit_code, 0, "touch a/b/nested.txt failed: {:?}", r);

        let out = shell.cmd_find(&["a"]);
        assert!(out.stdout.contains("a/file.txt"));
        assert!(out.stdout.contains("a/b/nested.txt"));

        let out = shell.cmd_find(&["a", "-name", "*.txt", "-type", "f"]);
        assert!(out.stdout.contains("a/file.txt"));
    }

    #[test]
    fn test_find_type_d() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["src"]);
        shell.cmd_touch(&["src/main.rs"]);

        let out = shell.cmd_find(&["src", "-type", "d"]);
        assert!(out.stdout.contains("src"));
    }

    #[test]
    fn test_find_maxdepth() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["a"]);
        shell.cmd_mkdir(&["a/b"]);
        shell.cmd_mkdir(&["a/b/c"]);
        shell.cmd_touch(&["a/b/c/deep.txt"]);

        let out = shell.cmd_find(&["a", "-maxdepth", "1"]);
        assert!(out.stdout.contains("a/b"));
        assert!(!out.stdout.contains("a/b/c"));
    }

    #[test]
    fn test_find_mtime() {
        let shell = mk_shell();
        shell.cmd_touch(&["new.txt"]);

        let out = shell.cmd_find(&[".", "-mtime", "-365"]);
        assert!(out.stdout.contains("new.txt"));

        let out = shell.cmd_find(&[".", "-mtime", "+9999"]);
        assert!(!out.stdout.contains("new.txt"));
    }

    #[test]
    fn test_find_size() {
        let shell = mk_shell();
        let out = shell.cmd_find(&[".", "-size", "-100M"]);
        assert!(out.stdout.contains("."));
    }

    #[test]
    fn test_find_not() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["testdir"]);
        shell.cmd_touch(&["testfile.txt"]);

        let out = shell.cmd_find(&[".", "!", "-type", "d", "-maxdepth", "1"]);
        assert!(!out.stdout.contains("testdir"));
        assert!(out.stdout.contains("testfile.txt"));
    }

    #[test]
    fn test_find_or() {
        let shell = mk_shell();
        shell.cmd_touch(&["a.txt"]);
        shell.cmd_touch(&["b.md"]);
        shell.cmd_touch(&["c.rs"]);

        let out = shell.cmd_find(&[".", "-name", "*.txt", "-o", "-name", "*.md", "-maxdepth", "1"]);
        assert!(out.stdout.contains("a.txt"));
        assert!(out.stdout.contains("b.md"));
        assert!(!out.stdout.contains("c.rs"));
    }

    #[test]
    fn test_find_print0() {
        let shell = mk_shell();
        shell.cmd_touch(&["file.txt"]);

        let out = shell.cmd_find(&[".", "-print0", "-maxdepth", "1"]);
        assert!(out.stdout.contains('\0'));
    }

    #[test]
    fn test_find_exec() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["sub"]);
        shell.cmd_touch(&["sub/a.txt"]);

        let out = shell.cmd_find(&["sub", "-name", "*.txt", "-exec", "echo", "found", "{}", ";"]);
        // echo should output "found sub/a.txt" including the newline
        assert!(out.stdout.contains("sub/a.txt"));
    }

    #[test]
    fn test_find_not_found_path() {
        let shell = mk_shell();
        let out = shell.cmd_find(&["/nonexistent_path_xyz"]);
        assert_ne!(out.exit_code, 0);
    }
}
