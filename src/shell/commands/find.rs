// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::sync::atomic::Ordering;
use std::time::SystemTime;

const FIND_HELP_TEXT: &str = "\
Usage: find [PATH] [OPTIONS] [EXPRESSION]

Predicates:
  -name PATTERN    File name matches shell pattern
  -iname PATTERN   Like -name, but case-insensitive
  -type [fdl]      File type: f=file, d=directory, l=symlink
  -empty           File is empty
  -mtime [+-]N     Modified time in days
  -size [+-]N[c]   File size in bytes
  -regex PATTERN   Path matches a (case-sensitive) regex
  -iregex PATTERN  Path matches a case-insensitive regex
  -newer FILE      Modified more recently than FILE

Actions:
  -print           Print file path (default)
  -print0          Print null-terminated
  -delete          Delete matching files
  -exec CMD {} \\;  Execute command per file
  -exec CMD {} +   Execute command with batch

Options:
  -maxdepth N      Maximum recursion depth
  -mindepth N      Minimum recursion depth
  -o               OR conditions
  !  -not          Negate condition

  -h, --help       Show this help\n";

#[derive(Debug, Clone, Copy, PartialEq)]
enum SizeSign {
    Greater,
    Less,
    Exact,
}

#[derive(Debug, Clone)]
enum ConditionKind {
    Name(String),
    Iname(String),
    Path(String),
    Ipath(String),
    Type(char),
    Empty,
    Mtime {
        days: i64,
        greater_than: bool,
    },
    Size {
        bytes: i64,
        sign: SizeSign,
    },
    Regex(String),
    IRegex(String),
    Newer(SystemTime),
    /// `-newermt TIME` — modification time newer than an absolute time.
    NewerMt(SystemTime),
    /// `-perm [-/]MODE` — permission bits (op: ' ' exact, '-' all, '/' any).
    Perm {
        mode: u32,
        op: char,
    },
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
    Delete,
    /// `-printf <format>` (common specifiers: %f %p %h %s %y %%; \n \t \0 \\)
    Printf(String),
    /// -exec cmd args {} ;   (run per matched file)
    Exec(Vec<String>),
    /// -exec cmd args {} +   (run once with all matched files)
    ExecBatch(Vec<String>),
}

impl Shell {
    pub fn cmd_find(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(FIND_HELP_TEXT.to_string());
        }
        let mut path = ".".to_string();
        let mut path_set = false;
        let mut conditions: Vec<Vec<Condition>> = vec![vec![]];
        let mut actions: Vec<Action> = Vec::new();
        let mut maxdepth: Option<usize> = None;
        let mut mindepth: Option<usize> = None;
        let mut i = 0;
        let mut negate_next = false;
        // `-prune` is an action in GNU find; the predicate group immediately
        // before it selects the directories not to descend into (and whose
        // output is suppressed, because the OR short-circuits).
        let mut has_prune = false;
        let mut prune_groups: Vec<usize> = Vec::new();

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
                "-mindepth" => {
                    if i + 1 < args.len() {
                        if let Ok(d) = args[i + 1].parse::<usize>() {
                            mindepth = Some(d);
                        }
                        i += 1;
                    }
                }
                "-empty" => {
                    add_condition(
                        &mut conditions,
                        Condition {
                            negate: negate_next,
                            kind: ConditionKind::Empty,
                        },
                    );
                    negate_next = false;
                }
                "-name" => {
                    if i + 1 < args.len() {
                        let pat = args[i + 1].to_string();
                        add_condition(
                            &mut conditions,
                            Condition {
                                negate: negate_next,
                                kind: ConditionKind::Name(pat),
                            },
                        );
                        negate_next = false;
                        i += 1;
                    }
                }
                "-iname" => {
                    if i + 1 < args.len() {
                        let pat = args[i + 1].to_string();
                        add_condition(
                            &mut conditions,
                            Condition {
                                negate: negate_next,
                                kind: ConditionKind::Iname(pat),
                            },
                        );
                        negate_next = false;
                        i += 1;
                    }
                }
                "-path" => {
                    if i + 1 < args.len() {
                        add_condition(
                            &mut conditions,
                            Condition {
                                negate: negate_next,
                                kind: ConditionKind::Path(args[i + 1].to_string()),
                            },
                        );
                        negate_next = false;
                        i += 1;
                    }
                }
                "-ipath" => {
                    if i + 1 < args.len() {
                        add_condition(
                            &mut conditions,
                            Condition {
                                negate: negate_next,
                                kind: ConditionKind::Ipath(args[i + 1].to_string()),
                            },
                        );
                        negate_next = false;
                        i += 1;
                    }
                }
                // `-print` is the default action; record it explicitly so that
                // `-prune -o ... -print` still prints the non-pruned branch.
                "-print" => {
                    actions.push(Action::Print);
                }
                "-prune" => {
                    has_prune = true;
                    if !conditions.is_empty() {
                        let idx = conditions.len() - 1;
                        if !prune_groups.contains(&idx) {
                            prune_groups.push(idx);
                        }
                    }
                }
                "-type" => {
                    if i + 1 < args.len() {
                        if let Some(ch) = args[i + 1].chars().next() {
                            add_condition(
                                &mut conditions,
                                Condition {
                                    negate: negate_next,
                                    kind: ConditionKind::Type(ch),
                                },
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
                                Condition {
                                    negate: negate_next,
                                    kind: cond,
                                },
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
                                Condition {
                                    negate: negate_next,
                                    kind: cond,
                                },
                            );
                        }
                        negate_next = false;
                        i += 1;
                    }
                }
                "-regex" => {
                    if i + 1 < args.len() {
                        add_condition(
                            &mut conditions,
                            Condition {
                                negate: negate_next,
                                kind: ConditionKind::Regex(args[i + 1].to_string()),
                            },
                        );
                        negate_next = false;
                        i += 1;
                    }
                }
                "-iregex" => {
                    if i + 1 < args.len() {
                        add_condition(
                            &mut conditions,
                            Condition {
                                negate: negate_next,
                                kind: ConditionKind::IRegex(args[i + 1].to_string()),
                            },
                        );
                        negate_next = false;
                        i += 1;
                    }
                }
                "-perm" => {
                    if i + 1 < args.len() {
                        let a = args[i + 1];
                        let (op, rest) = match a.chars().next() {
                            Some('-') => ('-', &a[1..]),
                            Some('/') => ('/', &a[1..]),
                            _ => (' ', a),
                        };
                        if let Ok(mode) = u32::from_str_radix(rest, 8) {
                            add_condition(
                                &mut conditions,
                                Condition {
                                    negate: negate_next,
                                    kind: ConditionKind::Perm { mode, op },
                                },
                            );
                        }
                        negate_next = false;
                        i += 1;
                    }
                }
                "-newermt" => {
                    if i + 1 < args.len() {
                        if let Some(t) = parse_ref_time(args[i + 1]) {
                            add_condition(
                                &mut conditions,
                                Condition {
                                    negate: negate_next,
                                    kind: ConditionKind::NewerMt(t),
                                },
                            );
                        }
                        negate_next = false;
                        i += 1;
                    }
                }
                "-newer" => {
                    if i + 1 < args.len() {
                        let ref_time = self
                            .vfs
                            .resolve(args[i + 1], &self.cwd)
                            .ok()
                            .and_then(|p| std::fs::metadata(&p).ok())
                            .and_then(|m| m.modified().ok());
                        if let Some(t) = ref_time {
                            add_condition(
                                &mut conditions,
                                Condition {
                                    negate: negate_next,
                                    kind: ConditionKind::Newer(t),
                                },
                            );
                        }
                        negate_next = false;
                        i += 1;
                    }
                }
                "-print0" => {
                    actions.push(Action::Print0);
                }
                "-delete" => {
                    actions.push(Action::Delete);
                }
                "-printf" => {
                    if i + 1 < args.len() {
                        actions.push(Action::Printf(args[i + 1].to_string()));
                        i += 1;
                    }
                }
                "-exec" => {
                    i += 1;
                    let mut exec_args: Vec<String> = Vec::new();
                    let mut batch = false;
                    while i < args.len() && args[i] != ";" && args[i] != "\\;" {
                        if args[i] == "+" {
                            batch = true;
                            break;
                        }
                        exec_args.push(args[i].to_string());
                        i += 1;
                    }
                    if !exec_args.is_empty() {
                        if batch {
                            actions.push(Action::ExecBatch(exec_args));
                        } else {
                            actions.push(Action::Exec(exec_args));
                        }
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
                _ => crate::warn!("find: warning: unsupported option '{}'", args[i]),
            }
            i += 1;
        }

        if actions.is_empty() && !has_prune {
            actions.push(Action::Print);
        }

        let action_groups: Vec<usize> = (0..conditions.len())
            .filter(|i| !prune_groups.contains(i))
            .collect();

        let compiled_conditions: Vec<Vec<(bool, Option<regex::Regex>, ConditionKind)>> = conditions
            .iter()
            .map(|group| {
                group
                    .iter()
                    .map(|c| {
                        let compiled = match &c.kind {
                            ConditionKind::Name(pat) => Some(compile_glob_ci(pat, false)),
                            ConditionKind::Iname(pat) => Some(compile_glob_ci(pat, true)),
                            ConditionKind::Path(pat) => Some(compile_glob_ci(pat, false)),
                            ConditionKind::Ipath(pat) => Some(compile_glob_ci(pat, true)),
                            ConditionKind::Regex(pat) => regex::Regex::new(pat).ok(),
                            ConditionKind::IRegex(pat) => {
                                regex::Regex::new(&format!("(?i){}", pat)).ok()
                            }
                            _ => None,
                        };
                        (c.negate, compiled, c.kind.clone())
                    })
                    .collect()
            })
            .collect();

        let mut output = String::new();
        let mut exit_code = 0;
        let mut batch_paths: Vec<String> = Vec::new();

        // Check the starting path itself
        let mut start_pruned = false;
        if let Ok(resolved) = self.vfs.resolve(&path, &self.cwd) {
            if let Ok(metadata) = std::fs::symlink_metadata(&resolved) {
                let entry_name = resolved
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                let start_entry = crate::vfs::DirEntry {
                    name: entry_name.clone(),
                    is_dir: resolved.metadata().map(|m| m.is_dir()).unwrap_or(false),
                    is_symlink: metadata.file_type().is_symlink(),
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                };
                start_pruned = !prune_groups.is_empty()
                    && eval_subset(
                        &compiled_conditions,
                        &prune_groups,
                        &entry_name,
                        &path,
                        &start_entry,
                        &self.vfs,
                        &self.cwd,
                    );
                let action_match = if action_groups.is_empty() {
                    compiled_conditions.iter().all(|g| g.is_empty())
                } else {
                    eval_subset(
                        &compiled_conditions,
                        &action_groups,
                        &entry_name,
                        &path,
                        &start_entry,
                        &self.vfs,
                        &self.cwd,
                    )
                };
                if !start_pruned && action_match && mindepth.map_or(true, |md| md == 0) {
                    apply_actions(
                        &actions,
                        &path,
                        &start_entry,
                        self,
                        &mut output,
                        &mut exit_code,
                        &mut batch_paths,
                    );
                }
            }
        }

        if !start_pruned {
            if let Err(e) = self.find_recursive(
                &path,
                0,
                maxdepth,
                mindepth,
                &compiled_conditions,
                &prune_groups,
                &action_groups,
                &actions,
                &mut output,
                &mut exit_code,
                &mut batch_paths,
            ) {
                return CommandOutput::error(format!("find: {}\n", e), 1);
            }
        }

        // `-exec ... {} +`: run once with all matched paths substituted.
        if !batch_paths.is_empty() {
            for action in &actions {
                if let Action::ExecBatch(cmd_args) = action {
                    let mut args: Vec<String> = Vec::new();
                    let mut replaced = false;
                    for a in cmd_args {
                        if a == "{}" {
                            args.extend(batch_paths.iter().cloned());
                            replaced = true;
                        } else {
                            args.push(a.clone());
                        }
                    }
                    if !replaced {
                        args.extend(batch_paths.iter().cloned());
                    }
                    run_exec_builtin(self, &args, &mut output, &mut exit_code);
                }
            }
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
        mindepth: Option<usize>,
        conditions: &[Vec<(bool, Option<regex::Regex>, ConditionKind)>],
        prune_groups: &[usize],
        action_groups: &[usize],
        actions: &[Action],
        output: &mut String,
        exit_code: &mut i32,
        batch_paths: &mut Vec<String>,
    ) -> Result<(), crate::vfs::VfsError> {
        if let Some(md) = maxdepth {
            if depth >= md {
                return Ok(());
            }
        }
        let entries = self.vfs.list_dir(path, &self.cwd)?;

        for entry in &entries {
            if self.cancel.load(Ordering::SeqCst) {
                return Ok(());
            }
            let entry_path = format!("{}/{}", path.trim_end_matches('/'), entry.name);
            let pruned = !prune_groups.is_empty()
                && eval_subset(
                    conditions,
                    prune_groups,
                    &entry.name,
                    &entry_path,
                    entry,
                    &self.vfs,
                    &self.cwd,
                );

            if entry.is_dir && !pruned {
                if maxdepth.map_or(true, |md| depth < md) {
                    let _ = self.find_recursive(
                        &entry_path,
                        depth + 1,
                        maxdepth,
                        mindepth,
                        conditions,
                        prune_groups,
                        action_groups,
                        actions,
                        output,
                        exit_code,
                        batch_paths,
                    );
                }
            }

            // A pruned entry is neither descended into nor printed.
            if pruned {
                continue;
            }

            // mindepth: entries listed here are at `depth + 1`.
            if mindepth.map_or(true, |md| depth + 1 >= md) {
                let action_match = if action_groups.is_empty() {
                    conditions.iter().all(|g| g.is_empty())
                } else {
                    eval_subset(
                        conditions,
                        action_groups,
                        &entry.name,
                        &entry_path,
                        entry,
                        &self.vfs,
                        &self.cwd,
                    )
                };
                if action_match {
                    apply_actions(
                        actions,
                        &entry_path,
                        entry,
                        self,
                        output,
                        exit_code,
                        batch_paths,
                    );
                }
            }
        }

        Ok(())
    }
}

fn apply_actions(
    actions: &[Action],
    entry_path: &str,
    entry: &crate::vfs::DirEntry,
    shell: &Shell,
    output: &mut String,
    exit_code: &mut i32,
    batch_paths: &mut Vec<String>,
) {
    let vfs = &shell.vfs;
    let cwd = &shell.cwd;
    for action in actions {
        match action {
            Action::Print => {
                output.push_str(&format!("{}\n", entry_path));
            }
            Action::Print0 => {
                output.push_str(entry_path);
                output.push('\0');
            }
            Action::Printf(fmt) => {
                output.push_str(&find_printf(fmt, entry_path, entry));
            }
            Action::Delete => {
                // Try as directory first, fall back to file
                if let Err(e) = vfs.remove_dir_all(entry_path, cwd) {
                    if let Err(e2) = vfs.remove_file(entry_path, cwd) {
                        *exit_code = 1;
                        output.push_str(&format!(
                            "find: cannot delete '{}': {} / {}\n",
                            entry_path, e, e2
                        ));
                    }
                }
            }
            Action::ExecBatch(_) => {
                // Collected here, executed once after the traversal.
                batch_paths.push(entry_path.to_string());
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
                run_exec_builtin(shell, &args, output, exit_code);
            }
        }
    }
}

/// Runs an `-exec` command through the built-in shell (sandbox-safe: no OS
/// process is spawned, so this works on mobile where no binaries exist).
fn run_exec_builtin(shell: &Shell, args: &[String], output: &mut String, exit_code: &mut i32) {
    if args.is_empty() {
        return;
    }
    let mut sub = shell.clone();
    let arg_refs: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
    let result = sub.execute(&args[0], &arg_refs, None);
    output.push_str(&result.stdout);
    if result.exit_code != 0 {
        *exit_code = result.exit_code;
        if !result.stderr.is_empty() {
            output.push_str(&result.stderr);
        }
    }
}

/// Minimal `-printf` formatter (GNU find's common specifiers).
fn find_printf(fmt: &str, entry_path: &str, entry: &crate::vfs::DirEntry) -> String {
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('0') => out.push('\0'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            '%' => match chars.next() {
                Some('f') => out.push_str(&entry.name),
                Some('p') => out.push_str(entry_path),
                Some('h') => out.push_str(
                    entry_path
                        .rsplit_once('/')
                        .map(|(d, _)| if d.is_empty() { "/" } else { d })
                        .unwrap_or("."),
                ),
                Some('s') => out.push_str(&entry.size.to_string()),
                Some('y') => out.push(if entry.is_symlink {
                    'l'
                } else if entry.is_dir {
                    'd'
                } else {
                    'f'
                }),
                Some('%') => out.push('%'),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            },
            _ => out.push(c),
        }
    }
    out
}

/// Parse a `-newermt` time: epoch seconds or `YYYY-MM-DD[ HH:MM:SS]`.
fn parse_ref_time(s: &str) -> Option<SystemTime> {
    if let Ok(secs) = s.parse::<i64>() {
        return Some(if secs >= 0 {
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64)
        } else {
            std::time::UNIX_EPOCH - std::time::Duration::from_secs((-secs) as u64)
        });
    }
    let (date, time) = match s.split_once(' ') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let mo: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let (hh, mm, ss) = match time {
        Some(t) => {
            let mut tp = t.split(':');
            (
                tp.next().and_then(|x| x.parse().ok()).unwrap_or(0i64),
                tp.next().and_then(|x| x.parse().ok()).unwrap_or(0i64),
                tp.next().and_then(|x| x.parse().ok()).unwrap_or(0i64),
            )
        }
        None => (0, 0, 0),
    };
    // Days since 1970-01-01 (civil-from-days algorithm).
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hh * 3600 + mm * 60 + ss;
    Some(if secs >= 0 {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64)
    } else {
        std::time::UNIX_EPOCH - std::time::Duration::from_secs((-secs) as u64)
    })
}

/// OR across the selected condition groups (each group is an AND of its
/// predicates). An empty group matches everything (GNU `-o -print`).
fn eval_subset(
    conditions: &[Vec<(bool, Option<regex::Regex>, ConditionKind)>],
    indices: &[usize],
    name: &str,
    path: &str,
    entry: &crate::vfs::DirEntry,
    vfs: &crate::vfs::Vfs,
    cwd: &str,
) -> bool {
    for &i in indices {
        if let Some(group) = conditions.get(i) {
            if group.is_empty() || group_matches(group, name, path, entry, vfs, cwd) {
                return true;
            }
        }
    }
    false
}

fn group_matches(
    group: &[(bool, Option<regex::Regex>, ConditionKind)],
    name: &str,
    path: &str,
    entry: &crate::vfs::DirEntry,
    vfs: &crate::vfs::Vfs,
    cwd: &str,
) -> bool {
    group.iter().all(|(negate, compiled, kind)| {
        let result = match kind {
            ConditionKind::Name(_) | ConditionKind::Iname(_) => match compiled {
                Some(re) => re.is_match(name),
                None => true,
            },
            // `-path`/`-ipath` (and `-regex`) match the full path, not the name.
            ConditionKind::Path(_) | ConditionKind::Ipath(_) => match compiled {
                Some(re) => re.is_match(path),
                None => true,
            },
            ConditionKind::Regex(_) | ConditionKind::IRegex(_) => match compiled {
                Some(re) => re.is_match(path),
                None => false,
            },
            ConditionKind::Newer(ref_time) => match entry.modified {
                Some(mod_time) => mod_time > *ref_time,
                None => false,
            },
            ConditionKind::NewerMt(ref_time) => match entry.modified {
                Some(mod_time) => mod_time > *ref_time,
                None => false,
            },
            ConditionKind::Perm { mode, op } => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let actual = vfs
                        .resolve(path, cwd)
                        .ok()
                        .and_then(|p| std::fs::metadata(&p).ok())
                        .map(|md| md.permissions().mode() & 0o7777);
                    match actual {
                        Some(a) => match op {
                            '-' => a & mode == *mode,
                            '/' => *mode == 0 || a & mode != 0,
                            _ => a == *mode,
                        },
                        None => false,
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = (mode, op);
                    true
                }
            }
            ConditionKind::Type(ch) => match ch {
                // GNU `-type` uses lstat: a symlink is `l`, never `f`/`d`.
                'd' => entry.is_dir && !entry.is_symlink,
                'f' => !entry.is_dir && !entry.is_symlink,
                'l' => entry.is_symlink,
                _ => true,
            },
            ConditionKind::Empty => {
                if entry.is_dir {
                    vfs.list_dir(path, cwd)
                        .map(|e| e.is_empty())
                        .unwrap_or(false)
                } else {
                    entry.size == 0
                }
            }
            ConditionKind::Mtime { days, greater_than } => match entry.modified {
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
            },
            ConditionKind::Size { bytes, sign } => {
                let size = entry.size as i64;
                match sign {
                    SizeSign::Greater => size > *bytes,
                    SizeSign::Less => size < *bytes,
                    SizeSign::Exact => size == *bytes,
                }
            }
        };
        if *negate {
            !result
        } else {
            result
        }
    })
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
    let (sign, rest) = match arg.chars().next()? {
        '+' => (SizeSign::Greater, &arg[1..]),
        '-' => (SizeSign::Less, &arg[1..]),
        _ => (SizeSign::Exact, arg),
    };
    if rest.is_empty() {
        return None;
    }
    // Unit suffix (`c` bytes, `w` 2-byte words, `b` 512-byte blocks, k/M/G).
    let (num_str, unit) = match rest.chars().last()? {
        'c' => (&rest[..rest.len() - 1], 'c'),
        'w' => (&rest[..rest.len() - 1], 'w'),
        'b' => (&rest[..rest.len() - 1], 'b'),
        'k' | 'K' => (&rest[..rest.len() - 1], 'k'),
        'M' => (&rest[..rest.len() - 1], 'M'),
        'G' => (&rest[..rest.len() - 1], 'G'),
        _ if rest.chars().all(|c| c.is_ascii_digit()) => (rest, 'b'),
        _ => return None,
    };
    let value: i64 = num_str.parse().ok()?;
    let mult: i64 = match unit {
        'c' => 1,
        'w' => 2,
        'k' => 1024,
        'M' => 1024 * 1024,
        'G' => 1024 * 1024 * 1024,
        _ => 512,
    };
    Some(ConditionKind::Size {
        bytes: value.saturating_mul(mult),
        sign,
    })
}

fn compile_glob_ci(pattern: &str, ignore_case: bool) -> regex::Regex {
    let mut regex_str = String::new();
    if ignore_case {
        regex_str.push_str("(?i)");
    }
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
        regex::Regex::new(&format!("^{}$", escaped))
            .unwrap_or_else(|_| regex::Regex::new(".*").unwrap())
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
        let dir =
            std::env::temp_dir().join(format!("fastshell_find_test_{}_{}", std::process::id(), n));
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
    fn test_find_iname_case_insensitive() {
        let shell = mk_shell();
        shell.cmd_touch(&["Report.PDF"]);
        shell.cmd_touch(&["notes.txt"]);

        // -iname matches case-insensitively.
        let out = shell.cmd_find(&[".", "-iname", "*.pdf"]);
        assert!(
            out.stdout.contains("Report.PDF"),
            "-iname should match Report.PDF: {}",
            out.stdout
        );
        assert!(
            !out.stdout.contains("notes.txt"),
            "-iname *.pdf should not match notes.txt"
        );
    }

    #[test]
    fn test_find_name_is_case_sensitive() {
        let shell = mk_shell();
        shell.cmd_touch(&["Report.PDF"]);

        // -name is case-sensitive, so *.pdf must NOT match Report.PDF.
        let out = shell.cmd_find(&[".", "-name", "*.pdf"]);
        assert!(
            !out.stdout.contains("Report.PDF"),
            "-name *.pdf should not match Report.PDF"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_find_type_l() {
        let shell = mk_shell();
        shell.cmd_touch(&["target.txt"]);
        // Create a symlink to target.txt.
        let root = shell.vfs.root().to_path_buf();
        let _ = std::os::unix::fs::symlink(root.join("target.txt"), root.join("link.txt"));

        let out = shell.cmd_find(&[".", "-type", "l"]);
        assert!(
            out.stdout.contains("link.txt"),
            "-type l should match the symlink: {}",
            out.stdout
        );
        assert!(
            !out.stdout.contains("target.txt"),
            "-type l should not match a regular file: {}",
            out.stdout
        );
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

        let out = shell.cmd_find(&[
            ".",
            "-name",
            "*.txt",
            "-o",
            "-name",
            "*.md",
            "-maxdepth",
            "1",
        ]);
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
    fn test_find_prune() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["keep"]);
        shell.cmd_mkdir(&["skip"]);
        shell.cmd_touch(&["keep/a.md"]);
        shell.cmd_touch(&["skip/b.md"]);

        let out = shell.cmd_find(&[
            ".", "-path", "./skip", "-prune", "-o", "-name", "*.md", "-print",
        ]);
        assert!(out.stdout.contains("./keep/a.md"), "{}", out.stdout);
        assert!(
            !out.stdout.contains("skip/b.md"),
            "pruned dir must not be descended: {}",
            out.stdout
        );
        assert!(
            !out.stdout.contains("./skip\n"),
            "pruned dir itself must not be printed: {}",
            out.stdout
        );
    }

    #[test]
    fn test_find_path_matches_full_path() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["a"]);
        shell.cmd_mkdir(&["a/b"]);
        shell.cmd_touch(&["a/b/c.txt"]);

        let out = shell.cmd_find(&[".", "-path", "*/b/*"]);
        assert!(out.stdout.contains("./a/b/c.txt"), "{}", out.stdout);
    }

    #[test]
    fn test_find_not_found_path() {
        let shell = mk_shell();
        let out = shell.cmd_find(&["/nonexistent_path_xyz"]);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_find_help() {
        let mut shell = mk_shell();
        let out = shell.execute("find", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_find_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("find", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
