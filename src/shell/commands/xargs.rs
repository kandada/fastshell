// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_xargs(&mut self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut max_args: Option<usize> = None;
        let mut replace_str: Option<String> = None;
        let mut null_delimited = false;
        let mut parallel: Option<usize> = None;
        let mut verbose = false;
        let mut target_cmd: Vec<String> = Vec::new();
        let mut parsing_cmd = false;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => {
                    if i + 1 < args.len() {
                        max_args = args[i + 1].parse::<usize>().ok();
                        i += 1;
                    }
                }
                a if a.starts_with("-n") && a.len() > 2 => {
                    max_args = a[2..].parse::<usize>().ok();
                }
                "-I" => {
                    if i + 1 < args.len() {
                        replace_str = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                a if a.starts_with("-I") && a.len() > 2 => {
                    replace_str = Some(a[2..].to_string());
                }
                "-0" | "--null" => {
                    null_delimited = true;
                }
                "-P" => {
                    if i + 1 < args.len() {
                        parallel = args[i + 1].parse::<usize>().ok();
                        i += 1;
                    }
                }
                a if a.starts_with("-P") && a.len() > 2 => {
                    parallel = a[2..].parse::<usize>().ok();
                }
                "-t" => {
                    verbose = true;
                }
                a if !a.starts_with('-') && !parsing_cmd => {
                    parsing_cmd = true;
                    target_cmd.push(a.to_string());
                }
                a if parsing_cmd => {
                    target_cmd.push(a.to_string());
                }
                _ => {}
            }
            i += 1;
        }

        if target_cmd.is_empty() {
            return CommandOutput::error("xargs: missing command\n".to_string(), 1);
        }

        let input = match stdin {
            Some(s) => s.to_string(),
            None => {
                if replace_str.is_some() {
                    // With -I, xargs runs the command even with empty input
                    let cmd =
                        build_command_with_replace(&target_cmd, "", replace_str.as_ref().unwrap());
                    if verbose {
                        eprintln!("{}", cmd.join(" "));
                    }
                    return self.execute(
                        &cmd[0],
                        &cmd[1..].iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                        None,
                    );
                }
                return CommandOutput::error("xargs: no input\n".to_string(), 1);
            }
        };

        let input_items: Vec<String> = if null_delimited {
            if input.is_empty() {
                Vec::new()
            } else {
                input.split('\0').map(|s| s.to_string()).collect()
            }
        } else if replace_str.is_some() {
            input
                .lines()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            input.split_whitespace().map(|s| s.to_string()).collect()
        };

        if input_items.is_empty() {
            return CommandOutput::success(String::new());
        }

        if let Some(ref rep) = replace_str {
            return exec_xargs_replace(self, &target_cmd, rep, &input_items, parallel, verbose);
        }

        let n_per = max_args.unwrap_or(input_items.len());
        let chunks: Vec<&[String]> = input_items.chunks(n_per).collect();

        exec_xargs_chunks(self, &target_cmd, &chunks, parallel, verbose)
    }
}

fn build_command_with_replace(
    target_cmd: &[String],
    replacement: &str,
    replace_str: &str,
) -> Vec<String> {
    target_cmd
        .iter()
        .map(|arg| arg.replace(replace_str, replacement))
        .collect()
}

fn exec_xargs_replace(
    shell: &mut Shell,
    target_cmd: &[String],
    replace_str: &str,
    items: &[String],
    parallel: Option<usize>,
    verbose: bool,
) -> CommandOutput {
    let max_parallel = parallel.unwrap_or(1);

    if max_parallel <= 1 {
        let mut combined_stdout = String::new();
        let mut combined_stderr = String::new();
        let mut last_exit = 0i32;

        for item in items {
            let cmd = build_command_with_replace(target_cmd, item, replace_str);
            if verbose {
                eprintln!("{}", cmd.join(" "));
            }
            let out = shell.execute(
                &cmd[0],
                &cmd[1..].iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                None,
            );
            combined_stdout.push_str(&out.stdout);
            combined_stderr.push_str(&out.stderr);
            if out.exit_code != 0 {
                last_exit = out.exit_code;
                if out.exit_code == 255 {
                    break;
                }
            }
        }

        return CommandOutput {
            stdout: combined_stdout,
            stderr: combined_stderr,
            exit_code: last_exit,
        };
    }

    run_parallel_replace(shell, target_cmd, replace_str, items, max_parallel, verbose)
}

fn exec_xargs_chunks(
    shell: &mut Shell,
    target_cmd: &[String],
    chunks: &[&[String]],
    parallel: Option<usize>,
    verbose: bool,
) -> CommandOutput {
    let max_parallel = parallel.unwrap_or(1);

    if max_parallel <= 1 {
        let mut combined_stdout = String::new();
        let mut combined_stderr = String::new();
        let mut last_exit = 0i32;

        for chunk in chunks {
            let cmd_name = &target_cmd[0];
            let mut all_args: Vec<&str> = target_cmd[1..].iter().map(|s| s.as_str()).collect();
            all_args.extend(chunk.iter().map(|s| s.as_str()));

            if verbose {
                let mut full = vec![cmd_name.as_str()];
                full.extend(&all_args);
                eprintln!("{}", full.join(" "));
            }

            let out = shell.execute(cmd_name, &all_args, None);
            combined_stdout.push_str(&out.stdout);
            combined_stderr.push_str(&out.stderr);
            if out.exit_code != 0 {
                last_exit = out.exit_code;
                if out.exit_code == 255 {
                    break;
                }
            }
        }

        return CommandOutput {
            stdout: combined_stdout,
            stderr: combined_stderr,
            exit_code: last_exit,
        };
    }

    run_parallel_chunks(shell, target_cmd, chunks, max_parallel, verbose)
}

fn run_parallel_replace(
    _shell: &Shell,
    target_cmd: &[String],
    replace_str: &str,
    items: &[String],
    max_parallel: usize,
    verbose: bool,
) -> CommandOutput {
    let cmd_clone = target_cmd.to_vec();
    let rep_clone = replace_str.to_string();
    let items_clone: Vec<String> = items.iter().map(|s| s.clone()).collect();

    let mut outputs: Vec<(String, String, i32)> = Vec::new();

    {
        let mut handles = Vec::new();
        let items_chunks: Vec<Vec<String>> = items_clone
            .chunks((items_clone.len() + max_parallel - 1) / max_parallel)
            .map(|c| c.to_vec())
            .collect();

        for chunk in items_chunks {
            let cmd = cmd_clone.clone();
            let rep = rep_clone.clone();
            let handle = std::thread::spawn(move || {
                let mut results = Vec::new();
                for item in &chunk {
                    let full_cmd = build_command_with_replace(&cmd, item, &rep);
                    if verbose {
                        eprintln!("{}", full_cmd.join(" "));
                    }
                    // In threads, we can't call shell.execute because it borrows mutably
                    // So use std::process::Command directly
                    let result = std::process::Command::new(&full_cmd[0])
                        .args(&full_cmd[1..])
                        .output();
                    match result {
                        Ok(out) => {
                            results.push((
                                String::from_utf8_lossy(&out.stdout).to_string(),
                                String::from_utf8_lossy(&out.stderr).to_string(),
                                out.status.code().unwrap_or(1),
                            ));
                        }
                        Err(e) => {
                            results.push((
                                String::new(),
                                format!("xargs: {}: {}\n", full_cmd[0], e),
                                127,
                            ));
                        }
                    }
                }
                results
            });
            handles.push(handle);
        }

        for handle in handles {
            if let Ok(results) = handle.join() {
                outputs.extend(results);
            }
        }
    }

    let mut combined_stdout = String::new();
    let mut combined_stderr = String::new();
    let mut last_exit = 0i32;

    for (so, se, ec) in outputs {
        combined_stdout.push_str(&so);
        combined_stderr.push_str(&se);
        if ec != 0 {
            last_exit = ec;
        }
    }

    CommandOutput {
        stdout: combined_stdout,
        stderr: combined_stderr,
        exit_code: if last_exit != 0 { 123 } else { 0 },
    }
}

fn run_parallel_chunks(
    _shell: &Shell,
    target_cmd: &[String],
    chunks: &[&[String]],
    max_parallel: usize,
    verbose: bool,
) -> CommandOutput {
    let cmd_clone = target_cmd.to_vec();
    let chunks_clone: Vec<Vec<String>> = chunks
        .iter()
        .map(|c| c.iter().map(|s| s.to_string()).collect())
        .collect();

    let mut outputs: Vec<(String, String, i32)> = Vec::new();

    {
        let mut handles = Vec::new();
        let batch_size = (chunks_clone.len() + max_parallel - 1) / max_parallel;

        for batch in chunks_clone.chunks(batch_size) {
            let cmd = cmd_clone.clone();
            let batch_owned: Vec<Vec<String>> = batch.iter().map(|c| c.clone()).collect();
            let handle = std::thread::spawn(move || {
                let mut results = Vec::new();
                for chunk in &batch_owned {
                    let full_cmd = &cmd[0];
                    let mut all_args: Vec<String> = cmd[1..].to_vec();
                    all_args.extend(chunk.iter().cloned());
                    if verbose {
                        eprintln!("{} {}", full_cmd, all_args.join(" "));
                    }
                    let result = std::process::Command::new(full_cmd)
                        .args(&all_args)
                        .output();
                    match result {
                        Ok(out) => {
                            results.push((
                                String::from_utf8_lossy(&out.stdout).to_string(),
                                String::from_utf8_lossy(&out.stderr).to_string(),
                                out.status.code().unwrap_or(1),
                            ));
                        }
                        Err(e) => {
                            results.push((
                                String::new(),
                                format!("xargs: {}: {}\n", full_cmd, e),
                                127,
                            ));
                        }
                    }
                }
                results
            });
            handles.push(handle);
        }

        for handle in handles {
            if let Ok(results) = handle.join() {
                outputs.extend(results);
            }
        }
    }

    let mut combined_stdout = String::new();
    let mut combined_stderr = String::new();
    let mut last_exit = 0i32;

    for (so, se, ec) in outputs {
        combined_stdout.push_str(&so);
        combined_stderr.push_str(&se);
        if ec != 0 {
            last_exit = ec;
        }
    }

    CommandOutput {
        stdout: combined_stdout,
        stderr: combined_stderr,
        exit_code: if last_exit != 0 { 123 } else { 0 },
    }
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
            std::env::temp_dir().join(format!("fastshell_xargs_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        let s = Shell::new(setup_vfs());
        s
    }

    #[test]
    fn test_xargs_basic() {
        let mut shell = mk_shell();
        let out = shell.cmd_xargs(&["echo"], Some("hello world foo"));
        assert!(out.stdout.contains("hello"));
        assert!(out.stdout.contains("world"));
        assert!(out.stdout.contains("foo"));
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_xargs_n() {
        let mut shell = mk_shell();
        let out = shell.cmd_xargs(&["-n", "1", "echo"], Some("hello world"));
        assert_eq!(out.stdout.trim().lines().count(), 2);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_xargs_null() {
        let mut shell = mk_shell();
        let out = shell.cmd_xargs(&["-0", "echo"], Some("hello\0world\0"));
        assert!(out.stdout.contains("hello"));
        assert!(out.stdout.contains("world"));
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_xargs_replace() {
        let mut shell = mk_shell();
        let out = shell.cmd_xargs(&["-I{}", "echo", "{}", "suffix"], Some("hello\nworld\n"));
        assert!(out.stdout.contains("hello suffix"));
        assert!(out.stdout.contains("world suffix"));
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_xargs_no_input() {
        let mut shell = mk_shell();
        let out = shell.cmd_xargs(&["echo"], None);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_xargs_missing_command() {
        let mut shell = mk_shell();
        let out = shell.cmd_xargs(&[], Some("input"));
        assert_ne!(out.exit_code, 0);
    }
}
