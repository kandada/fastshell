#[cfg(feature = "git")]
use crate::shell::{Shell, CommandOutput};

#[cfg(feature = "git")]
impl Shell {
    pub fn cmd_git(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("git: missing command\n".to_string(), 1);
        }

        let subcommand = args[0];
        let rest = &args[1..];

        match subcommand {
            "clone" => self.git_clone(rest),
            "status" => self.git_status(rest),
            "add" => self.git_add(rest),
            "commit" => self.git_commit(rest),
            "push" => self.git_push(rest),
            "pull" => self.git_pull(rest),
            "init" => self.git_init(rest),
            _ => CommandOutput::error(
                format!("git: '{}' is not a supported command\n", subcommand),
                1,
            ),
        }
    }

    fn git_repo_path(&self) -> std::path::PathBuf {
        let vfs_root = self.vfs.root().to_path_buf();
        if self.cwd == "/" {
            vfs_root
        } else {
            vfs_root.join(self.cwd.trim_start_matches('/'))
        }
    }

    fn git_clone(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("git clone: missing URL\n".to_string(), 1);
        }

        let url = args[0];
        let dest = if args.len() > 1 {
            args[1].to_string()
        } else {
            crate::shell::extract_repo_name(url)
        };

        let dest_path = self.git_repo_path().join(&dest);

        match git2::Repository::clone(url, &dest_path) {
            Ok(_) => CommandOutput::success(format!(
                "Cloned into '{}'\n",
                dest
            )),
            Err(e) => CommandOutput::error(format!("git clone: {}\n", e), 1),
        }
    }

    fn git_status(&self, _args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git status: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let mut output = String::new();

        let statuses = match repo.statuses(None) {
            Ok(s) => s,
            Err(e) => return CommandOutput::error(format!("git status: {}\n", e), 1),
        };

        for entry in statuses.iter() {
            let status = entry.status();
            let path = entry.path().unwrap_or("?");

            let flags = format!(
                "{}{}",
                crate::shell::status_code(status, true),
                crate::shell::status_code(status, false),
            );
            output.push_str(&format!(" {} {}\n", flags, path));
        }

        if output.is_empty() {
            output = "nothing to commit, working tree clean\n".to_string();
        }

        CommandOutput::success(output)
    }

    fn git_add(&self, args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git add: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => return CommandOutput::error(format!("git add: {}\n", e), 1),
        };

        if args.is_empty() || args.iter().any(|a| *a == "." || *a == "-A" || *a == "--all") {
            if let Err(e) = index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None) {
                return CommandOutput::error(format!("git add: {}\n", e), 1);
            }
        } else {
            for path in args {
                if path.starts_with('-') {
                    continue;
                }
                if let Err(e) = index.add_path(std::path::Path::new(path)) {
                    return CommandOutput::error(
                        format!("git add: {}: {}\n", path, e),
                        1,
                    );
                }
            }
        }

        if let Err(e) = index.write() {
            return CommandOutput::error(format!("git add: {}\n", e), 1);
        }

        CommandOutput::success(String::new())
    }

    fn git_commit(&self, args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git commit: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let mut message = String::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-m" => {
                    if i + 1 < args.len() {
                        message = args[i + 1].to_string();
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        if message.is_empty() {
            return CommandOutput::error(
                "git commit: please supply the message (-m)\n".to_string(),
                1,
            );
        }

        let signature = match repo.signature() {
            Ok(s) => s,
            Err(e) => return CommandOutput::error(format!("git commit: {}\n", e), 1),
        };

        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => return CommandOutput::error(format!("git commit: {}\n", e), 1),
        };

        let tree_oid = match index.write_tree() {
            Ok(oid) => oid,
            Err(e) => return CommandOutput::error(format!("git commit: {}\n", e), 1),
        };

        let tree = match repo.find_tree(tree_oid) {
            Ok(t) => t,
            Err(e) => return CommandOutput::error(format!("git commit: {}\n", e), 1),
        };

        let head_oid = repo.head().ok().and_then(|h| h.target());
        let parents: Vec<git2::Commit> = match head_oid {
            Some(oid) => {
                if let Ok(c) = repo.find_commit(oid) {
                    vec![c]
                } else {
                    Vec::new()
                }
            }
            None => Vec::new(),
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

        match repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            &message,
            &tree,
            &parent_refs,
        ) {
            Ok(oid) => {
                let short = oid.to_string().chars().take(7).collect::<String>();
                CommandOutput::success(format!("[master {}] {}\n", short, message))
            }
            Err(e) => CommandOutput::error(format!("git commit: {}\n", e), 1),
        }
    }

    fn git_push(&self, args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git push: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let remote_name = args.first().copied().unwrap_or("origin");
        let branch = args.get(1).copied().unwrap_or("master");

        let mut remote = match repo.find_remote(remote_name) {
            Ok(r) => r,
            Err(_) => {
                return CommandOutput::error(
                    format!("git push: remote '{}' not found\n", remote_name),
                    1,
                );
            }
        };

        let refspec = format!("refs/heads/{}:refs/heads/{}", branch, branch);
        match remote.push(&[&refspec], None) {
            Ok(_) => CommandOutput::success(format!("Pushed to {} {}\n", remote_name, branch)),
            Err(e) => CommandOutput::error(format!("git push: {}\n", e), 1),
        }
    }

    fn git_pull(&self, args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git pull: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let remote_name = args.first().copied().unwrap_or("origin");
        let branch = args.get(1).copied().unwrap_or("master");

        let mut remote = match repo.find_remote(remote_name) {
            Ok(r) => r,
            Err(_) => {
                return CommandOutput::error(
                    format!("git pull: remote '{}' not found\n", remote_name),
                    1,
                );
            }
        };

        let _refspec = format!("refs/heads/{}:refs/heads/{}", branch, branch);
        let callbacks = git2::RemoteCallbacks::new();
        let mut fetch_opts = git2::FetchOptions::new();
        fetch_opts.remote_callbacks(callbacks);

        match remote.fetch(&[&branch], Some(&mut fetch_opts), None) {
            Ok(_) => {
                let fetch_head = match repo.find_reference("FETCH_HEAD") {
                    Ok(r) => r,
                    Err(e) => {
                        return CommandOutput::error(format!("git pull: {}\n", e), 1);
                    }
                };
                let fetch_commit = match fetch_head.peel_to_commit() {
                    Ok(c) => c,
                    Err(e) => {
                        return CommandOutput::error(format!("git pull: {}\n", e), 1);
                    }
                };
                let fetch_annotated = match repo.find_annotated_commit(fetch_commit.id()) {
                    Ok(a) => a,
                    Err(e) => {
                        return CommandOutput::error(format!("git pull: {}\n", e), 1);
                    }
                };

                let (analysis, _) = match repo.merge_analysis(&[&fetch_annotated]) {
                    Ok(a) => a,
                    Err(e) => {
                        return CommandOutput::error(format!("git pull: {}\n", e), 1);
                    }
                };

                if analysis.is_fast_forward() {
                    match repo.find_reference(&format!("refs/heads/{}", branch)) {
                        Ok(mut r) => {
                            r.set_target(fetch_commit.id(), "pull: fast-forward").ok();
                        }
                        Err(_) => {}
                    }
                    repo.set_head(&format!("refs/heads/{}", branch)).ok();
                    CommandOutput::success(format!("Pulled {} {} (fast-forward)\n", remote_name, branch))
                } else if analysis.is_normal() {
                    repo.merge(&[&fetch_annotated], None, None).ok();
                    if repo.index().map_or(false, |idx| idx.has_conflicts()) {
                        CommandOutput::success(format!("Pulled {} {} (merge conflicts)\n", remote_name, branch))
                    } else {
                        let tree_id = repo.index().and_then(|idx| idx.write_tree()).ok();
                        if let Some(tid) = tree_id {
                            if let Ok(tree) = repo.find_tree(tid) {
                                let head = repo.head().and_then(|h| h.peel_to_commit()).ok();
                                let sig = repo.signature().ok();
                                if let (Some(head), Some(sig)) = (head, sig) {
                                    let msg = format!("Merge branch '{}'", remote_name);
                                    repo.commit(Some("HEAD"), &sig, &sig, &msg, &tree, &[&head, &fetch_commit]).ok();
                                    repo.cleanup_state().ok();
                                }
                            }
                        }
                        CommandOutput::success(format!("Pulled {} {}\n", remote_name, branch))
                    }
                } else {
                    CommandOutput::success(format!("Pulled {} {} (up to date)\n", remote_name, branch))
                }
            }
            Err(e) => CommandOutput::error(format!("git pull: {}\n", e), 1),
        }
    }

    fn git_init(&self, _args: &[&str]) -> CommandOutput {
        let path = self.git_repo_path();
        match git2::Repository::init(&path) {
            Ok(_) => CommandOutput::success(format!(
                "Initialized empty Git repository in {}\n",
                path.display()
            )),
            Err(e) => CommandOutput::error(format!("git init: {}\n", e), 1),
        }
    }
}
