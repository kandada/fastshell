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
            "log" => self.git_log(rest),
            "diff" => self.git_diff(rest),
            "checkout" => self.git_checkout(rest),
            "branch" => self.git_branch(rest),
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
                        let tree_id = repo.index().and_then(|mut idx| idx.write_tree()).ok();
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

    fn git_log(&self, args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git log: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let mut oneline = false;
        let mut limit: Option<usize> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--oneline" => oneline = true,
                "-n" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        let mut revwalk = match repo.revwalk() {
            Ok(r) => r,
            Err(e) => return CommandOutput::error(format!("git log: {}\n", e), 1),
        };
        revwalk.push_head().ok();

        let mut output = String::new();
        let mut count = 0;

        for oid_result in revwalk {
            if let Some(n) = limit {
                if count >= n {
                    break;
                }
            }

            let oid = match oid_result {
                Ok(o) => o,
                Err(_) => continue,
            };

            let commit = match repo.find_commit(oid) {
                Ok(c) => c,
                Err(_) => continue,
            };

            count += 1;

            let short_hash: String = oid.to_string().chars().take(7).collect();
            let message = commit.message().unwrap_or("");
            let first_line = message.lines().next().unwrap_or("");

            if oneline {
                output.push_str(&format!("{} {}\n", short_hash, first_line));
            } else {
                let author = commit.author();
                let time = commit.time();
                let seconds = time.seconds();
                output.push_str(&format!("commit {}\n", oid));
                output.push_str(&format!("Author: {} <{}>\n", author.name().unwrap_or(""), author.email().unwrap_or("")));
                output.push_str(&format!("Date:   {}\n", format_timestamp(seconds)));
                output.push('\n');
                for line in message.lines() {
                    output.push_str(&format!("    {}\n", line));
                }
                output.push('\n');
            }
        }

        if output.is_empty() {
            output = "fatal: your current branch does not have any commits yet\n".to_string();
            return CommandOutput::error(output, 1);
        }

        CommandOutput::success(output)
    }

    fn git_diff(&self, args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git diff: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let mut cached = false;
        let mut commit_arg: Option<&str> = None;

        for arg in args {
            match *arg {
                "--cached" | "--staged" => cached = true,
                a if !a.starts_with('-') => commit_arg = Some(a),
                _ => {}
            }
        }

        if cached {
            let tree = match repo.head().and_then(|h| h.peel_to_tree()) {
                Ok(t) => t,
                Err(_) => {
                    // No HEAD tree - no staged changes to diff
                    return CommandOutput::success(String::new());
                }
            };

            let diff = match repo.diff_tree_to_index(Some(&tree), None, None) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
            };

            self.format_git_diff(&repo, &diff)
        } else if let Some(commit_str) = commit_arg {
            let obj = match repo.revparse_single(commit_str) {
                Ok(o) => o,
                Err(e) => return CommandOutput::error(format!("git diff: {}: {}\n", commit_str, e), 1),
            };
            let commit = match obj.peel_to_commit() {
                Ok(c) => c,
                Err(_) => {
                    match obj.peel_to_tree() {
                        Ok(t) => {
                            let diff = match repo.diff_tree_to_workdir(Some(&t), None) {
                                Ok(d) => d,
                                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
                            };
                            return self.format_git_diff(&repo, &diff);
                        }
                        Err(e) => return CommandOutput::error(format!("git diff: {}: {}\n", commit_str, e), 1),
                    }
                }
            };
            let tree = match commit.tree() {
                Ok(t) => t,
                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
            };

            let diff = match repo.diff_tree_to_workdir(Some(&tree), None) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
            };

            self.format_git_diff(&repo, &diff)
        } else {
            let diff = match repo.diff_index_to_workdir(None, None) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
            };

            self.format_git_diff(&repo, &diff)
        }
    }

    fn format_git_diff(&self, _repo: &git2::Repository, diff: &git2::Diff) -> CommandOutput {
        let mut output = String::new();

        let print_result = diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let origin = line.origin();
            let content = std::str::from_utf8(line.content()).unwrap_or("");
            match origin {
                '+' | '-' | ' ' => {
                    output.push(origin);
                    output.push_str(content);
                }
                'F' | 'H' => {
                    output.push_str(content);
                }
                _ => {
                    output.push_str(content);
                }
            }
            true
        });

        if let Err(e) = print_result {
            return CommandOutput::error(format!("git diff: {}\n", e), 1);
        }

        if output.is_empty() {
            CommandOutput::success(String::new())
        } else {
            CommandOutput {
                stdout: output,
                stderr: String::new(),
                exit_code: 1,
            }
        }
    }

    fn git_checkout(&self, args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git checkout: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let mut create_branch = false;
        let mut branch_name: Option<&str> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-b" => {
                    create_branch = true;
                    if i + 1 < args.len() {
                        branch_name = Some(args[i + 1]);
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => {
                    branch_name = Some(arg);
                }
                _ => {}
            }
            i += 1;
        }

        let branch_name = match branch_name {
            Some(b) => b,
            None => {
                return CommandOutput::error(
                    "git checkout: missing branch name\n".to_string(),
                    1,
                );
            }
        };

        let refname = format!("refs/heads/{}", branch_name);

        if create_branch {
            let target = match repo.head().and_then(|h| h.peel_to_commit()) {
                Ok(c) => c,
                Err(_) => {
                    return CommandOutput::error(
                        "git checkout: failed to resolve HEAD\n".to_string(),
                        1,
                    );
                }
            };
            if let Err(e) = repo.branch(branch_name, &target, false) {
                return CommandOutput::error(format!("git checkout: {}\n", e), 1);
            }
            let _ = repo.set_head(&refname);
        } else {
            // Verify the branch exists
            match repo.find_branch(branch_name, git2::BranchType::Local) {
                Ok(_) => {}
                Err(_) => {
                    let refs = repo.references().ok();
                    let is_ref = refs.map_or(false, |mut r| {
                        r.any(|rr| rr.map_or(false, |rref| rref.name() == Some(&refname)))
                    });
                    if is_ref {
                        let _ = repo.set_head(&refname);
                    } else {
                        return CommandOutput::error(
                            format!(
                                "git checkout: branch '{}' not found\n",
                                branch_name
                            ),
                            1,
                        );
                    }
                }
            }
            let _ = repo.set_head(&refname);
        }

        match repo.checkout_head(None) {
            Ok(_) => {
                if create_branch {
                    CommandOutput::success(format!("Switched to a new branch '{}'\n", branch_name))
                } else {
                    CommandOutput::success(format!("Switched to branch '{}'\n", branch_name))
                }
            }
            Err(e) => CommandOutput::error(format!("git checkout: {}\n", e), 1),
        }
    }

    fn git_branch(&self, args: &[&str]) -> CommandOutput {
        let repo = match git2::Repository::open(self.git_repo_path()) {
            Ok(r) => r,
            Err(e) => {
                return CommandOutput::error(
                    format!("git branch: not a git repository: {}\n", e),
                    1,
                );
            }
        };

        let mut delete_branch: Option<&str> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-d" | "-D" => {
                    if i + 1 < args.len() {
                        delete_branch = Some(args[i + 1]);
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        if let Some(branch_name) = delete_branch {
            let mut branch = match repo.find_branch(branch_name, git2::BranchType::Local) {
                Ok(b) => b,
                Err(_) => {
                    return CommandOutput::error(
                        format!(
                            "git branch: branch '{}' not found\n",
                            branch_name
                        ),
                        1,
                    );
                }
            };
            match branch.delete() {
                Ok(_) => CommandOutput::success(format!("Deleted branch '{}'\n", branch_name)),
                Err(e) => CommandOutput::error(format!("git branch: {}\n", e), 1),
            }
        } else {
            let head_name = repo
                .head()
                .ok()
                .and_then(|h| h.shorthand().map(|s| s.to_string()));

            let branches = match repo.branches(None) {
                Ok(b) => b,
                Err(e) => return CommandOutput::error(format!("git branch: {}\n", e), 1),
            };

            let mut output = String::new();
            let mut branch_list = Vec::new();

            for branch_result in branches {
                let (branch, _) = match branch_result {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let name = match branch.name() {
                    Ok(Some(n)) => n.to_string(),
                    _ => continue,
                };
                branch_list.push(name);
            }

            branch_list.sort();

            for name in &branch_list {
                if Some(name.as_str()) == head_name.as_deref() {
                    output.push_str(&format!("* {}\n", name));
                } else {
                    output.push_str(&format!("  {}\n", name));
                }
            }

            if output.is_empty() {
                output = "No branches yet (create one with 'git checkout -b <name>')\n".to_string();
            }

            CommandOutput::success(output)
        }
    }
}

#[cfg(feature = "git")]
fn format_timestamp(seconds: i64) -> String {
    // Simple naive conversion from unix timestamp to readable date
    // We don't have chrono, so do a simple calculation
    let days_since_epoch = seconds / 86400;
    // Rough date calculation (since epoch is 1970-01-01)
    let mut year: i64 = 1970;
    let mut remaining_days = days_since_epoch;

    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let month_days = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month: usize = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining_days < md as i64 {
            month = i;
            break;
        }
        remaining_days -= md as i64;
    }

    let day = remaining_days + 1;
    let month_names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let month_name = month_names[month];

    let time_of_day = seconds.rem_euclid(86400);
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let secs = time_of_day % 60;

    // We don't know timezone, just show the computed time
    format!(
        "{} {} {:02}:{:02}:{:02} {} +0000",
        month_name, day, hours, minutes, secs, year
    )
}

#[cfg(feature = "git")]
fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
