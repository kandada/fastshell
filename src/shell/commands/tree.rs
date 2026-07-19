// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! `tree` — directory structure listing (agent-friendly project overview).

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_tree(&self, args: &[&str]) -> CommandOutput {
        let mut path = ".".to_string();
        let mut max_depth: Option<usize> = None;
        let mut dirs_only = false;
        let mut show_hidden = false;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-L" => {
                    if i + 1 < args.len() {
                        max_depth = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                "-d" => dirs_only = true,
                "-a" => show_hidden = true,
                arg if !arg.starts_with('-') => path = arg.to_string(),
                _ => {}
            }
            i += 1;
        }

        match self.vfs.resolve(&path, &self.cwd) {
            Ok(p) if p.is_dir() => {}
            Ok(_) => {
                return CommandOutput::error(format!("tree: {}: not a directory\n", path), 1)
            }
            Err(e) => return CommandOutput::error(format!("tree: {}: {}\n", path, e), 1),
        }

        let mut output = String::new();
        let display_root = if path == "." {
            ".".to_string()
        } else {
            path.trim_end_matches('/').to_string()
        };
        output.push_str(&display_root);
        output.push('\n');

        let mut dir_count = 0usize;
        let mut file_count = 0usize;
        self.tree_recursive(
            &path,
            "",
            0,
            max_depth,
            dirs_only,
            show_hidden,
            &mut output,
            &mut dir_count,
            &mut file_count,
        );

        output.push_str(&format!(
            "\n{} director{}, {} file{}\n",
            dir_count,
            if dir_count == 1 { "y" } else { "ies" },
            file_count,
            if file_count == 1 { "" } else { "s" },
        ));
        CommandOutput::success(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_recursive(
        &self,
        path: &str,
        prefix: &str,
        depth: usize,
        max_depth: Option<usize>,
        dirs_only: bool,
        show_hidden: bool,
        output: &mut String,
        dir_count: &mut usize,
        file_count: &mut usize,
    ) {
        if let Some(md) = max_depth {
            if depth >= md {
                return;
            }
        }
        let mut entries = match self.vfs.list_dir(path, &self.cwd) {
            Ok(e) => e,
            Err(_) => return,
        };
        entries.retain(|e| show_hidden || !e.name.starts_with('.'));
        if dirs_only {
            entries.retain(|e| e.is_dir);
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        let total = entries.len();
        for (idx, entry) in entries.iter().enumerate() {
            let last = idx + 1 == total;
            let branch = if last { "└── " } else { "├── " };
            output.push_str(prefix);
            output.push_str(branch);
            output.push_str(&entry.name);
            if entry.is_dir {
                output.push('/');
            }
            output.push('\n');

            if entry.is_dir {
                *dir_count += 1;
                let child_prefix = format!("{}{}", prefix, if last { "    " } else { "│   " });
                let child_path = format!("{}/{}", path.trim_end_matches('/'), entry.name);
                self.tree_recursive(
                    &child_path,
                    &child_prefix,
                    depth + 1,
                    max_depth,
                    dirs_only,
                    show_hidden,
                    output,
                    dir_count,
                    file_count,
                );
            } else {
                *file_count += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_tree_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Shell::new(Vfs::new(dir).unwrap())
    }

    #[test]
    fn tree_basic() {
        let shell = mk_shell();
        shell.vfs.create_dir_all("sub/inner", "/").unwrap();
        shell.vfs.write("a.txt", "/", "x").unwrap();
        shell.vfs.write("sub/b.txt", "/", "y").unwrap();
        let out = shell.cmd_tree(&[]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("a.txt"));
        assert!(out.stdout.contains("sub/"));
        assert!(out.stdout.contains("b.txt"));
        assert!(out.stdout.contains("2 directories, 2 files"));
    }

    #[test]
    fn tree_depth_limit() {
        let shell = mk_shell();
        shell.vfs.create_dir_all("sub", "/").unwrap();
        shell.vfs.write("sub/deep.txt", "/", "y").unwrap();
        let out = shell.cmd_tree(&["-L", "1"]);
        assert!(out.stdout.contains("sub/"));
        assert!(!out.stdout.contains("deep.txt"));
    }

    #[test]
    fn tree_dirs_only() {
        let shell = mk_shell();
        shell.vfs.create_dir_all("sub", "/").unwrap();
        shell.vfs.write("a.txt", "/", "x").unwrap();
        let out = shell.cmd_tree(&["-d"]);
        assert!(out.stdout.contains("sub/"));
        assert!(!out.stdout.contains("a.txt"));
    }

    #[test]
    fn tree_hidden_filtered() {
        let shell = mk_shell();
        shell.vfs.write(".hidden", "/", "x").unwrap();
        shell.vfs.write("shown.txt", "/", "x").unwrap();
        let out = shell.cmd_tree(&[]);
        assert!(!out.stdout.contains(".hidden"));
        let out = shell.cmd_tree(&["-a"]);
        assert!(out.stdout.contains(".hidden"));
    }

    #[test]
    fn tree_not_a_dir() {
        let shell = mk_shell();
        shell.vfs.write("f.txt", "/", "x").unwrap();
        let out = shell.cmd_tree(&["f.txt"]);
        assert_eq!(out.exit_code, 1);
    }
}
