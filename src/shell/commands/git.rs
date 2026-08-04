// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Built-in `git` command backed by git2-rs (libgit2). Designed to behave
//! like real git closely enough that the AACodeApp `GitManager` (and coding
//! agents) can parse the output: porcelain status, `--format=` log,
//! stash/remote/merge/reset/fetch, real branch names, and git-compatible
//! exit codes.

#[cfg(feature = "git")]
use crate::shell::{CommandOutput, Shell};

#[cfg(feature = "git")]
const GIT_HELP_TEXT: &str = "\
Usage: git <command> [<options>...]

Available commands:
  clone    Clone a repository
  init     Initialize a new repository
  status   Show working tree status
  add      Add file contents to the index
  commit   Record changes to the repository
  push     Update remote refs along with associated objects
  pull     Fetch from and integrate with another repository
  fetch    Download objects and refs from another repository
  log      Show commit logs
  diff     Show changes between commits
  checkout Switch branches or restore working tree files
  branch   List, create, or delete branches
  reset    Reset current HEAD to the specified state
  stash    Stash the changes in a dirty working directory
  remote   Manage set of tracked repositories
  merge    Join two or more development histories together
  rev-parse Pick out and massage parameters
  show     Show various types of objects
  rm       Remove files from the working tree and index
  mv       Move or rename a file, directory, or symlink
  restore  Restore working tree files
  config   Get and set repository or global options
  clean    Remove untracked files from the working tree

  -h, --help  Show this help\n";

/// Process-wide per-repository locks. Serializes git mutations on the same
/// repository across ALL fastshell instances in the process (e.g. an agent
/// task's sandbox instance and the app UI's global instance), preventing
/// `.git/index` lock contention and interleaved ref updates.
#[cfg(feature = "git")]
static REPO_LOCKS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, std::sync::Arc<std::sync::Mutex<()>>>>,
> = std::sync::OnceLock::new();

#[cfg(feature = "git")]
fn repo_lock(path: &std::path::Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut map = REPO_LOCKS
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.entry(canon)
        .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(())))
        .clone()
}

#[cfg(feature = "git")]
impl Shell {
    pub fn cmd_git(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() || args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(GIT_HELP_TEXT.to_string());
        }

        // Android: OpenSSL's compiled-in CA paths (/usr/local/ssl) don't
        // exist on-device, so certificate verification always failed. The
        // host app exports SSL_CERT_FILE pointing at a PEM bundle built from
        // the system CA store — hand it to libgit2 explicitly (once).
        init_ssl_certs();

        let subcommand = args[0];
        let rest = &args[1..];

        // Serialize all git operations on the same repository (process-wide).
        let lock = repo_lock(&self.git_repo_path());
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

        match subcommand {
            "clone" => self.git_clone(rest),
            "status" => self.git_status(rest),
            "add" => self.git_add(rest),
            "commit" => self.git_commit(rest),
            "push" => self.git_push(rest),
            "pull" => self.git_pull(rest),
            "fetch" => self.git_fetch(rest),
            "init" => self.git_init(rest),
            "log" => self.git_log(rest),
            "diff" => self.git_diff(rest),
            "checkout" => self.git_checkout(rest),
            "branch" => self.git_branch(rest),
            "reset" => self.git_reset(rest),
            "stash" => self.git_stash(rest),
            "remote" => self.git_remote(rest),
            "merge" => self.git_merge(rest),
            "rev-parse" => self.git_rev_parse(rest),
            "show" => self.git_show(rest),
            "rm" => self.git_rm(rest),
            "mv" => self.git_mv(rest),
            "restore" => self.git_restore(rest),
            "config" => self.git_config(rest),
            "clean" => self.git_clean(rest),
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

    fn open_repo(&self, cmd: &str) -> Result<git2::Repository, CommandOutput> {
        git2::Repository::open(self.git_repo_path()).map_err(|e| {
            CommandOutput::error(format!("git {}: not a git repository: {}\n", cmd, e), 128)
        })
    }

    // ───────────────────────────── clone ─────────────────────────────

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

        // Local path clone: resolve relative paths against the sandbox cwd
        // (libgit2 would otherwise resolve them against the process cwd).
        let url_owned: String;
        let url = if !url.contains("://") && !url.starts_with("git@") {
            let local = self.git_repo_path().join(url);
            if local.exists() {
                url_owned = local.to_string_lossy().to_string();
                &url_owned
            } else {
                url
            }
        } else {
            url
        };

        let mut fetch_opts = git2::FetchOptions::new();
        fetch_opts.remote_callbacks(remote_callbacks());
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_opts);

        match builder.clone(url, &dest_path) {
            Ok(_) => CommandOutput::success(format!("Cloned into '{}'\n", dest)),
            Err(e) => CommandOutput::error(format!("git clone: {}\n", auth_hint(&e)), 128),
        }
    }

    // ───────────────────────────── status ────────────────────────────

    fn git_status(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("status") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let porcelain = args.iter().any(|a| *a == "--porcelain" || *a == "--short" || *a == "-s");
        let want_branch = args.iter().any(|a| *a == "-b" || *a == "--branch");

        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let statuses = match repo.statuses(Some(&mut opts)) {
            Ok(s) => s,
            Err(e) => return CommandOutput::error(format!("git status: {}\n", e), 1),
        };

        let mut output = String::new();

        if porcelain {
            if want_branch {
                output.push_str(&format!("## {}", head_branch_name(&repo)));
                // Append upstream + ahead/behind when available.
                if let Some((upstream, ahead, behind)) = upstream_info(&repo) {
                    output.push_str(&format!("...{}", upstream));
                    match (ahead, behind) {
                        (0, 0) => {}
                        (a, 0) => output.push_str(&format!(" [ahead {}]", a)),
                        (0, b) => output.push_str(&format!(" [behind {}]", b)),
                        (a, b) => output.push_str(&format!(" [ahead {}, behind {}]", a, b)),
                    }
                }
                output.push('\n');
            }
            for entry in statuses.iter() {
                let status = entry.status();
                let path = entry.path().unwrap_or("?");
                let (x, y) = porcelain_codes(status);
                output.push_str(&format!("{}{} {}\n", x, y, path));
            }
            return CommandOutput::success(output);
        }

        // Human-readable format (legacy).
        output.push_str(&format!("On branch {}\n", head_branch_name(&repo)));
        let mut any = false;
        for entry in statuses.iter() {
            let status = entry.status();
            let path = entry.path().unwrap_or("?");
            let flags = format!(
                "{}{}",
                crate::shell::status_code(status, true),
                crate::shell::status_code(status, false),
            );
            output.push_str(&format!(" {} {}\n", flags, path));
            any = true;
        }
        if !any {
            output.push_str("nothing to commit, working tree clean\n");
        }
        CommandOutput::success(output)
    }

    // ───────────────────────────── add ───────────────────────────────

    fn git_add(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("add") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => return CommandOutput::error(format!("git add: {}\n", e), 1),
        };

        // Strip `--` separator (used by GitManager: `git add -- <paths>`).
        let paths: Vec<&str> = args.iter().filter(|a| **a != "--").copied().collect();

        if paths.is_empty()
            || paths
                .iter()
                .any(|a| *a == "." || *a == "-A" || *a == "--all" || *a == "-u")
        {
            if let Err(e) = index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None) {
                return CommandOutput::error(format!("git add: {}\n", e), 1);
            }
            // add_all does not record deletions of tracked files; sync them.
            if let Err(e) = index.update_all(["*"].iter(), None) {
                return CommandOutput::error(format!("git add: {}\n", e), 1);
            }
        } else {
            for path in paths {
                if path.starts_with('-') {
                    continue;
                }
                let p = std::path::Path::new(path);
                let abs = self.git_repo_path().join(path);
                if abs.exists() {
                    if let Err(e) = index.add_path(p) {
                        return CommandOutput::error(format!("git add: {}: {}\n", path, e), 1);
                    }
                } else {
                    // Deleted file: record removal.
                    if let Err(e) = index.remove_path(p) {
                        return CommandOutput::error(format!("git add: {}: {}\n", path, e), 1);
                    }
                }
            }
        }

        if let Err(e) = index.write() {
            return CommandOutput::error(format!("git add: {}\n", e), 1);
        }

        CommandOutput::success(String::new())
    }

    // ───────────────────────────── commit ────────────────────────────

    fn git_commit(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("commit") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut message = String::new();
        let mut stage_tracked = false;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-m" | "--message" => {
                    if i + 1 < args.len() {
                        message = args[i + 1].to_string();
                        i += 1;
                    }
                }
                "-a" | "--all" => stage_tracked = true,
                "-am" | "-ma" => {
                    stage_tracked = true;
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

        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => return CommandOutput::error(format!("git commit: {}\n", e), 1),
        };

        if stage_tracked {
            if let Err(e) = index.update_all(["*"].iter(), None) {
                return CommandOutput::error(format!("git commit: {}\n", e), 1);
            }
            if let Err(e) = index.write() {
                return CommandOutput::error(format!("git commit: {}\n", e), 1);
            }
        }

        let signature = signature_or_default(&repo);

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

        let branch = head_branch_name(&repo);
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
                CommandOutput::success(format!("[{} {}] {}\n", branch, short, message))
            }
            Err(e) => CommandOutput::error(format!("git commit: {}\n", e), 1),
        }
    }

    // ─────────────────────────── push / pull / fetch ─────────────────

    fn git_push(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("push") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        let remote_name = positional.first().copied().unwrap_or("origin");
        let branch_arg = positional.get(1).copied().unwrap_or("HEAD");
        let branch = if branch_arg == "HEAD" {
            head_branch_name(&repo)
        } else {
            branch_arg.to_string()
        };

        let mut remote = match repo.find_remote(remote_name) {
            Ok(r) => r,
            Err(_) => {
                return CommandOutput::error(
                    format!("git push: remote '{}' not found\n", remote_name),
                    128,
                );
            }
        };

        let mut opts = git2::PushOptions::new();
        opts.remote_callbacks(remote_callbacks());

        let refspec = format!("refs/heads/{}:refs/heads/{}", branch, branch);
        match remote.push(&[&refspec], Some(&mut opts)) {
            Ok(_) => CommandOutput::success(format!("Pushed to {} {}\n", remote_name, branch)),
            Err(e) => CommandOutput::error(format!("git push: {}\n", auth_hint(&e)), 128),
        }
    }

    fn git_fetch(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("fetch") {
            Ok(r) => r,
            Err(e) => return e,
        };
        let remote_name = args
            .iter()
            .find(|a| !a.starts_with('-'))
            .copied()
            .unwrap_or("origin");
        let mut remote = match repo.find_remote(remote_name) {
            Ok(r) => r,
            Err(_) => {
                return CommandOutput::error(
                    format!("git fetch: remote '{}' not found\n", remote_name),
                    128,
                );
            }
        };
        let mut fetch_opts = git2::FetchOptions::new();
        fetch_opts.remote_callbacks(remote_callbacks());
        // Empty refspec list → use the remote's configured refspecs.
        match remote.fetch(&[] as &[&str], Some(&mut fetch_opts), None) {
            Ok(_) => CommandOutput::success(format!("Fetched {}\n", remote_name)),
            Err(e) => CommandOutput::error(format!("git fetch: {}\n", auth_hint(&e)), 128),
        }
    }

    fn git_pull(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("pull") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();
        let remote_name = positional.first().copied().unwrap_or("origin");
        let branch_arg = positional.get(1).copied().unwrap_or("HEAD");
        let branch = if branch_arg == "HEAD" {
            head_branch_name(&repo)
        } else {
            branch_arg.to_string()
        };

        let mut remote = match repo.find_remote(remote_name) {
            Ok(r) => r,
            Err(_) => {
                return CommandOutput::error(
                    format!("git pull: remote '{}' not found\n", remote_name),
                    128,
                );
            }
        };

        let mut fetch_opts = git2::FetchOptions::new();
        fetch_opts.remote_callbacks(remote_callbacks());

        if let Err(e) = remote.fetch(&[branch.as_str()], Some(&mut fetch_opts), None) {
            return CommandOutput::error(format!("git pull: {}\n", auth_hint(&e)), 128);
        }

        let fetch_head = match repo.find_reference("FETCH_HEAD") {
            Ok(r) => r,
            Err(e) => return CommandOutput::error(format!("git pull: {}\n", e), 1),
        };
        let fetch_commit = match fetch_head.peel_to_commit() {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("git pull: {}\n", e), 1),
        };
        let fetch_annotated = match repo.find_annotated_commit(fetch_commit.id()) {
            Ok(a) => a,
            Err(e) => return CommandOutput::error(format!("git pull: {}\n", e), 1),
        };

        let (analysis, _) = match repo.merge_analysis(&[&fetch_annotated]) {
            Ok(a) => a,
            Err(e) => return CommandOutput::error(format!("git pull: {}\n", e), 1),
        };

        if analysis.is_up_to_date() {
            return CommandOutput::success("Already up to date.\n".to_string());
        }

        if analysis.is_fast_forward() {
            let refname = format!("refs/heads/{}", branch);
            match repo.find_reference(&refname) {
                Ok(mut r) => {
                    if let Err(e) = r.set_target(fetch_commit.id(), "pull: fast-forward") {
                        return CommandOutput::error(format!("git pull: {}\n", e), 1);
                    }
                }
                Err(_) => {
                    if let Err(e) =
                        repo.reference(&refname, fetch_commit.id(), true, "pull: fast-forward")
                    {
                        return CommandOutput::error(format!("git pull: {}\n", e), 1);
                    }
                }
            }
            let _ = repo.set_head(&refname);
            let mut cb = git2::build::CheckoutBuilder::new();
            cb.force();
            let _ = repo.checkout_head(Some(&mut cb));
            return CommandOutput::success(format!(
                "Updated {} to {} (fast-forward)\n",
                branch,
                &fetch_commit.id().to_string()[..7]
            ));
        }

        if analysis.is_normal() {
            if let Err(e) = repo.merge(&[&fetch_annotated], None, None) {
                return CommandOutput::error(format!("git pull: {}\n", e), 1);
            }
            let has_conflicts = repo.index().map_or(false, |idx| idx.has_conflicts());
            if has_conflicts {
                return CommandOutput::error(
                    format!(
                        "CONFLICT (content): merge conflict pulling {}/{}\nAutomatic merge failed; fix conflicts and then commit the result.\n",
                        remote_name, branch
                    ),
                    1,
                );
            }
            let tree_id = repo.index().and_then(|mut idx| idx.write_tree()).ok();
            if let Some(tid) = tree_id {
                if let Ok(tree) = repo.find_tree(tid) {
                    let head = repo.head().and_then(|h| h.peel_to_commit()).ok();
                    if let Some(head) = head {
                        let sig = signature_or_default(&repo);
                        let msg = format!("Merge branch '{}' of {}", branch, remote_name);
                        let _ = repo.commit(
                            Some("HEAD"),
                            &sig,
                            &sig,
                            &msg,
                            &tree,
                            &[&head, &fetch_commit],
                        );
                        let _ = repo.cleanup_state();
                        let mut cb = git2::build::CheckoutBuilder::new();
                        cb.force();
                        let _ = repo.checkout_head(Some(&mut cb));
                    }
                }
            }
            return CommandOutput::success(format!("Merged {}/{}\n", remote_name, branch));
        }

        CommandOutput::success("Already up to date.\n".to_string())
    }

    // ───────────────────────────── init ──────────────────────────────

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

    // ───────────────────────────── log ───────────────────────────────

    fn git_log(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("log") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut oneline = false;
        let mut limit: Option<usize> = None;
        let mut format: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            match a {
                "--oneline" => oneline = true,
                "-n" | "--max-count" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                _ if a.starts_with("--format=") => {
                    format = Some(a["--format=".len()..].to_string());
                }
                _ if a.starts_with("--pretty=format:") => {
                    format = Some(a["--pretty=format:".len()..].to_string());
                }
                _ if a.starts_with("--pretty=") => {
                    let p = &a["--pretty=".len()..];
                    if p == "oneline" {
                        oneline = true;
                    }
                }
                _ if a.starts_with("-n") && a.len() > 2 => {
                    limit = a[2..].parse().ok();
                }
                _ if a.starts_with('-')
                    && a.len() > 1
                    && a[1..].chars().all(|c| c.is_ascii_digit()) =>
                {
                    limit = a[1..].parse().ok();
                }
                _ => {}
            }
            i += 1;
        }

        let mut revwalk = match repo.revwalk() {
            Ok(r) => r,
            Err(e) => return CommandOutput::error(format!("git log: {}\n", e), 1),
        };
        if revwalk.push_head().is_err() {
            return CommandOutput::error(
                "fatal: your current branch does not have any commits yet\n".to_string(),
                128,
            );
        }

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

            if let Some(ref fmt) = format {
                output.push_str(&format_commit(&commit, fmt));
                output.push('\n');
                continue;
            }

            let short_hash: String = oid.to_string().chars().take(7).collect();
            let message = commit.message().unwrap_or("");
            let first_line = message.lines().next().unwrap_or("");

            if oneline {
                output.push_str(&format!("{} {}\n", short_hash, first_line));
            } else {
                let author = commit.author();
                let time = commit.time();
                output.push_str(&format!("commit {}\n", oid));
                output.push_str(&format!(
                    "Author: {} <{}>\n",
                    author.name().unwrap_or(""),
                    author.email().unwrap_or("")
                ));
                output.push_str(&format!("Date:   {}\n", format_timestamp(time.seconds())));
                output.push('\n');
                for line in message.lines() {
                    output.push_str(&format!("    {}\n", line));
                }
                output.push('\n');
            }
        }

        if output.is_empty() {
            return CommandOutput::error(
                "fatal: your current branch does not have any commits yet\n".to_string(),
                128,
            );
        }

        CommandOutput::success(output)
    }

    // ───────────────────────────── diff ──────────────────────────────

    fn git_diff(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("diff") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut cached = false;
        let mut rev_arg: Option<String> = None;
        let mut pathspecs: Vec<String> = Vec::new();

        for arg in args {
            match *arg {
                "--cached" | "--staged" => cached = true,
                "--" => {}
                a if !a.starts_with('-') => {
                    // Disambiguate: existing file → pathspec; else revision.
                    let on_disk = self.git_repo_path().join(a).exists();
                    let known_in_index = repo
                        .index()
                        .ok()
                        .map(|idx| idx.get_path(std::path::Path::new(a), 0).is_some())
                        .unwrap_or(false);
                    if on_disk || known_in_index {
                        pathspecs.push(a.to_string());
                    } else if rev_arg.is_none() && repo.revparse_single(a).is_ok() {
                        rev_arg = Some(a.to_string());
                    } else {
                        pathspecs.push(a.to_string());
                    }
                }
                _ => {}
            }
        }

        let mut opts = git2::DiffOptions::new();
        for p in &pathspecs {
            opts.pathspec(p);
        }

        let diff = if cached {
            let tree = repo.head().and_then(|h| h.peel_to_tree()).ok();
            match repo.diff_tree_to_index(tree.as_ref(), None, Some(&mut opts)) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
            }
        } else if let Some(rev) = rev_arg {
            let obj = match repo.revparse_single(&rev) {
                Ok(o) => o,
                Err(e) => {
                    return CommandOutput::error(format!("git diff: {}: {}\n", rev, e), 128)
                }
            };
            let tree = match obj.peel_to_tree() {
                Ok(t) => t,
                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
            };
            match repo.diff_tree_to_workdir_with_index(Some(&tree), Some(&mut opts)) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
            }
        } else {
            match repo.diff_index_to_workdir(None, Some(&mut opts)) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("git diff: {}\n", e), 1),
            }
        };

        format_git_diff(&diff)
    }

    // ───────────────────────────── checkout ──────────────────────────

    fn git_checkout(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("checkout") {
            Ok(r) => r,
            Err(e) => return e,
        };

        // `git checkout -- <files>`: restore files from the index.
        if let Some(sep) = args.iter().position(|a| *a == "--") {
            let files = &args[sep + 1..];
            let mut cb = git2::build::CheckoutBuilder::new();
            cb.force();
            for f in files {
                cb.path(f);
            }
            return match repo.checkout_index(None, Some(&mut cb)) {
                Ok(_) => CommandOutput::success(String::new()),
                Err(e) => CommandOutput::error(format!("git checkout: {}\n", e), 1),
            };
        }

        let mut create_branch = false;
        let mut branch_name: Option<&str> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-b" | "-B" => {
                    create_branch = true;
                    if i + 1 < args.len() {
                        branch_name = Some(args[i + 1]);
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => {
                    if branch_name.is_none() {
                        branch_name = Some(arg);
                    }
                }
                _ => {}
            }
            i += 1;
        }

        let branch_name = match branch_name {
            Some(b) => b,
            None => {
                return CommandOutput::error("git checkout: missing branch name\n".to_string(), 1);
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
            if let Err(e) = repo.set_head(&refname) {
                return CommandOutput::error(format!("git checkout: {}\n", e), 1);
            }
            // Same tree as HEAD — no working tree changes needed.
            return CommandOutput::success(format!(
                "Switched to a new branch '{}'\n",
                branch_name
            ));
        }

        // Switch to an existing branch (or resolvable revision): update the
        // working tree AND the index to the target, safely.
        let (object, reference) = match repo.revparse_ext(branch_name) {
            Ok(x) => x,
            Err(_) => {
                return CommandOutput::error(
                    format!(
                        "git checkout: pathspec '{}' did not match any branch\n",
                        branch_name
                    ),
                    1,
                );
            }
        };

        let mut cb = git2::build::CheckoutBuilder::new();
        cb.safe();
        if let Err(e) = repo.checkout_tree(&object, Some(&mut cb)) {
            return CommandOutput::error(
                format!(
                    "git checkout: cannot switch branch: {} (commit or stash your changes)\n",
                    e
                ),
                1,
            );
        }

        let result = match reference {
            Some(r) => repo.set_head(r.name().unwrap_or(&refname)),
            None => repo.set_head_detached(object.id()),
        };
        if let Err(e) = result {
            return CommandOutput::error(format!("git checkout: {}\n", e), 1);
        }

        CommandOutput::success(format!("Switched to branch '{}'\n", branch_name))
    }

    // ───────────────────────────── branch ────────────────────────────

    fn git_branch(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("branch") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut delete_branch: Option<&str> = None;
        let mut create_name: Option<&str> = None;
        let mut show_all = false;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-d" | "-D" => {
                    if i + 1 < args.len() {
                        delete_branch = Some(args[i + 1]);
                        i += 1;
                    }
                }
                "-a" | "--all" => show_all = true,
                arg if !arg.starts_with('-') => create_name = Some(arg),
                _ => {}
            }
            i += 1;
        }

        if let Some(branch_name) = delete_branch {
            let mut branch = match repo.find_branch(branch_name, git2::BranchType::Local) {
                Ok(b) => b,
                Err(_) => {
                    return CommandOutput::error(
                        format!("git branch: branch '{}' not found\n", branch_name),
                        1,
                    );
                }
            };
            return match branch.delete() {
                Ok(_) => CommandOutput::success(format!("Deleted branch '{}'\n", branch_name)),
                Err(e) => CommandOutput::error(format!("git branch: {}\n", e), 1),
            };
        }

        // `git branch <name>`: create a branch at HEAD without switching.
        if let Some(name) = create_name {
            let target = match repo.head().and_then(|h| h.peel_to_commit()) {
                Ok(c) => c,
                Err(_) => {
                    return CommandOutput::error(
                        "git branch: failed to resolve HEAD\n".to_string(),
                        1,
                    );
                }
            };
            return match repo.branch(name, &target, false) {
                Ok(_) => CommandOutput::success(String::new()),
                Err(e) => CommandOutput::error(format!("git branch: {}\n", e), 1),
            };
        }

        let head_name = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(|s| s.to_string()));

        let filter = if show_all {
            None
        } else {
            Some(git2::BranchType::Local)
        };
        let branches = match repo.branches(filter) {
            Ok(b) => b,
            Err(e) => return CommandOutput::error(format!("git branch: {}\n", e), 1),
        };

        let mut output = String::new();
        let mut local_list = Vec::new();
        let mut remote_list = Vec::new();

        for branch_result in branches {
            let (branch, btype) = match branch_result {
                Ok(b) => b,
                Err(_) => continue,
            };
            let name = match branch.name() {
                Ok(Some(n)) => n.to_string(),
                _ => continue,
            };
            match btype {
                git2::BranchType::Local => local_list.push(name),
                git2::BranchType::Remote => remote_list.push(name),
            }
        }

        local_list.sort();
        remote_list.sort();

        for name in &local_list {
            if Some(name.as_str()) == head_name.as_deref() {
                output.push_str(&format!("* {}\n", name));
            } else {
                output.push_str(&format!("  {}\n", name));
            }
        }
        for name in &remote_list {
            output.push_str(&format!("  remotes/{}\n", name));
        }

        if output.is_empty() {
            output = "No branches yet (create one with 'git checkout -b <name>')\n".to_string();
        }

        CommandOutput::success(output)
    }

    // ───────────────────────────── reset ─────────────────────────────

    fn git_reset(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("reset") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let mut mode: Option<git2::ResetType> = None;
        let mut target: Option<&str> = None;
        let mut paths: Vec<&str> = Vec::new();
        let mut after_sep = false;

        for a in args {
            match *a {
                "--hard" => mode = Some(git2::ResetType::Hard),
                "--soft" => mode = Some(git2::ResetType::Soft),
                "--mixed" => mode = Some(git2::ResetType::Mixed),
                "--" => after_sep = true,
                s if !s.starts_with('-') => {
                    if after_sep {
                        paths.push(s);
                    } else if target.is_none() && repo.revparse_single(s).is_ok() {
                        target = Some(s);
                    } else {
                        paths.push(s);
                    }
                }
                _ => {}
            }
        }

        // Path-mode reset: unstage the given paths (index ← HEAD).
        if !paths.is_empty() && mode.is_none() {
            let head_obj = repo.head().ok().and_then(|h| h.peel(git2::ObjectType::Commit).ok());
            return match repo.reset_default(head_obj.as_ref(), &paths) {
                Ok(_) => CommandOutput::success(String::new()),
                Err(e) => CommandOutput::error(format!("git reset: {}\n", e), 1),
            };
        }

        let spec = target.unwrap_or("HEAD");
        let obj = match repo.revparse_single(spec) {
            Ok(o) => o,
            Err(e) => return CommandOutput::error(format!("git reset: {}: {}\n", spec, e), 128),
        };
        let mode = mode.unwrap_or(git2::ResetType::Mixed);
        match repo.reset(&obj, mode, None) {
            Ok(_) => {
                let label = match mode {
                    git2::ResetType::Hard => "HEAD is now at",
                    _ => "Unstaged changes after reset to",
                };
                let short = obj.id().to_string().chars().take(7).collect::<String>();
                CommandOutput::success(format!("{} {}\n", label, short))
            }
            Err(e) => CommandOutput::error(format!("git reset: {}\n", e), 1),
        }
    }

    // ───────────────────────────── stash ─────────────────────────────

    fn git_stash(&self, args: &[&str]) -> CommandOutput {
        let mut repo = match self.open_repo("stash") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let sub = args.first().copied().unwrap_or("push");
        match sub {
            "push" | "save" | "-u" => {
                let include_untracked = args.iter().any(|a| *a == "-u" || *a == "--include-untracked");
                let sig = signature_or_default(&repo);
                let mut flags = git2::StashFlags::DEFAULT;
                if include_untracked {
                    flags |= git2::StashFlags::INCLUDE_UNTRACKED;
                }
                match repo.stash_save2(&sig, None, Some(flags)) {
                    Ok(oid) => CommandOutput::success(format!(
                        "Saved working directory and index state ({})\n",
                        &oid.to_string()[..7]
                    )),
                    Err(e) if e.code() == git2::ErrorCode::NotFound => {
                        CommandOutput::success("No local changes to save\n".to_string())
                    }
                    Err(e) => CommandOutput::error(format!("git stash: {}\n", e), 1),
                }
            }
            "pop" | "apply" => {
                let drop_after = sub == "pop";
                let mut opts = git2::StashApplyOptions::new();
                let apply = repo.stash_apply(0, Some(&mut opts));
                match apply {
                    Ok(_) => {
                        if drop_after {
                            let _ = repo.stash_drop(0);
                        }
                        CommandOutput::success("Stash applied.\n".to_string())
                    }
                    Err(e) => CommandOutput::error(format!("git stash {}: {}\n", sub, e), 1),
                }
            }
            "drop" => match repo.stash_drop(0) {
                Ok(_) => CommandOutput::success("Dropped stash@{0}\n".to_string()),
                Err(e) => CommandOutput::error(format!("git stash drop: {}\n", e), 1),
            },
            "list" => {
                let mut output = String::new();
                let _ = repo.stash_foreach(|idx, msg, _oid| {
                    output.push_str(&format!("stash@{{{}}}: {}\n", idx, msg));
                    true
                });
                CommandOutput::success(output)
            }
            _ => CommandOutput::error(
                format!("git stash: unsupported subcommand '{}'\n", sub),
                1,
            ),
        }
    }

    // ───────────────────────────── remote ────────────────────────────

    fn git_remote(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("remote") {
            Ok(r) => r,
            Err(e) => return e,
        };

        match args.first().copied() {
            None | Some("-v") | Some("--verbose") => {
                let names = match repo.remotes() {
                    Ok(n) => n,
                    Err(e) => return CommandOutput::error(format!("git remote: {}\n", e), 1),
                };
                let verbose = matches!(args.first().copied(), Some("-v") | Some("--verbose"));
                let mut output = String::new();
                for name in names.iter().flatten() {
                    if verbose {
                        let url = repo
                            .find_remote(name)
                            .ok()
                            .and_then(|r| r.url().map(|u| u.to_string()))
                            .unwrap_or_default();
                        output.push_str(&format!("{}\t{} (fetch)\n", name, url));
                        output.push_str(&format!("{}\t{} (push)\n", name, url));
                    } else {
                        output.push_str(&format!("{}\n", name));
                    }
                }
                CommandOutput::success(output)
            }
            Some("get-url") => {
                let name = match args.get(1) {
                    Some(n) => *n,
                    None => {
                        return CommandOutput::error(
                            "git remote get-url: missing remote name\n".to_string(),
                            1,
                        )
                    }
                };
                match repo.find_remote(name) {
                    Ok(r) => {
                        CommandOutput::success(format!("{}\n", r.url().unwrap_or_default()))
                    }
                    Err(_) => CommandOutput::error(
                        format!("error: No such remote '{}'\n", name),
                        2,
                    ),
                }
            }
            Some("add") => {
                let (name, url) = match (args.get(1), args.get(2)) {
                    (Some(n), Some(u)) => (*n, *u),
                    _ => {
                        return CommandOutput::error(
                            "git remote add: usage: git remote add <name> <url>\n".to_string(),
                            1,
                        )
                    }
                };
                match repo.remote(name, url) {
                    Ok(_) => CommandOutput::success(String::new()),
                    Err(e) => CommandOutput::error(format!("git remote add: {}\n", e), 1),
                }
            }
            Some("remove") | Some("rm") => {
                let name = match args.get(1) {
                    Some(n) => *n,
                    None => {
                        return CommandOutput::error(
                            "git remote remove: missing remote name\n".to_string(),
                            1,
                        )
                    }
                };
                match repo.remote_delete(name) {
                    Ok(_) => CommandOutput::success(String::new()),
                    Err(e) => CommandOutput::error(format!("git remote remove: {}\n", e), 1),
                }
            }
            Some("set-url") => {
                let (name, url) = match (args.get(1), args.get(2)) {
                    (Some(n), Some(u)) => (*n, *u),
                    _ => {
                        return CommandOutput::error(
                            "git remote set-url: usage: git remote set-url <name> <url>\n"
                                .to_string(),
                            1,
                        )
                    }
                };
                match repo.remote_set_url(name, url) {
                    Ok(_) => CommandOutput::success(String::new()),
                    Err(e) => CommandOutput::error(format!("git remote set-url: {}\n", e), 1),
                }
            }
            Some(other) => CommandOutput::error(
                format!("git remote: unsupported subcommand '{}'\n", other),
                1,
            ),
        }
    }

    // ───────────────────────────── merge ─────────────────────────────

    fn git_merge(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("merge") {
            Ok(r) => r,
            Err(e) => return e,
        };

        if args.iter().any(|a| *a == "--abort") {
            let head = match repo.head().and_then(|h| h.peel(git2::ObjectType::Commit)) {
                Ok(o) => o,
                Err(e) => return CommandOutput::error(format!("git merge --abort: {}\n", e), 1),
            };
            if let Err(e) = repo.reset(&head, git2::ResetType::Hard, None) {
                return CommandOutput::error(format!("git merge --abort: {}\n", e), 1);
            }
            let _ = repo.cleanup_state();
            return CommandOutput::success(String::new());
        }

        let branch = match args.iter().find(|a| !a.starts_with('-')) {
            Some(b) => *b,
            None => return CommandOutput::error("git merge: missing branch\n".to_string(), 1),
        };

        let obj = match repo.revparse_single(branch) {
            Ok(o) => o,
            Err(e) => {
                return CommandOutput::error(format!("git merge: {}: {}\n", branch, e), 128)
            }
        };
        let annotated = match repo.find_annotated_commit(obj.id()) {
            Ok(a) => a,
            Err(e) => return CommandOutput::error(format!("git merge: {}\n", e), 1),
        };

        let (analysis, _) = match repo.merge_analysis(&[&annotated]) {
            Ok(a) => a,
            Err(e) => return CommandOutput::error(format!("git merge: {}\n", e), 1),
        };

        if analysis.is_up_to_date() {
            return CommandOutput::success("Already up to date.\n".to_string());
        }

        if analysis.is_fast_forward() {
            let branch_name = head_branch_name(&repo);
            let refname = format!("refs/heads/{}", branch_name);
            if let Ok(mut r) = repo.find_reference(&refname) {
                if let Err(e) = r.set_target(obj.id(), "merge: fast-forward") {
                    return CommandOutput::error(format!("git merge: {}\n", e), 1);
                }
            }
            let mut cb = git2::build::CheckoutBuilder::new();
            cb.force();
            let _ = repo.checkout_head(Some(&mut cb));
            return CommandOutput::success(format!(
                "Fast-forward to {}\n",
                &obj.id().to_string()[..7]
            ));
        }

        if let Err(e) = repo.merge(&[&annotated], None, None) {
            return CommandOutput::error(format!("git merge: {}\n", e), 1);
        }
        if repo.index().map_or(false, |idx| idx.has_conflicts()) {
            return CommandOutput::error(
                format!(
                    "CONFLICT (content): merge conflict merging '{}'\nAutomatic merge failed; fix conflicts and then commit the result.\n",
                    branch
                ),
                1,
            );
        }

        let tree_id = match repo.index().and_then(|mut idx| idx.write_tree()) {
            Ok(t) => t,
            Err(e) => return CommandOutput::error(format!("git merge: {}\n", e), 1),
        };
        let tree = match repo.find_tree(tree_id) {
            Ok(t) => t,
            Err(e) => return CommandOutput::error(format!("git merge: {}\n", e), 1),
        };
        let head = match repo.head().and_then(|h| h.peel_to_commit()) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("git merge: {}\n", e), 1),
        };
        let their = match repo.find_commit(obj.id()) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("git merge: {}\n", e), 1),
        };
        let sig = signature_or_default(&repo);
        let msg = format!("Merge branch '{}'", branch);
        match repo.commit(Some("HEAD"), &sig, &sig, &msg, &tree, &[&head, &their]) {
            Ok(_) => {
                let _ = repo.cleanup_state();
                let mut cb = git2::build::CheckoutBuilder::new();
                cb.force();
                let _ = repo.checkout_head(Some(&mut cb));
                CommandOutput::success(format!("Merged branch '{}'\n", branch))
            }
            Err(e) => CommandOutput::error(format!("git merge: {}\n", e), 1),
        }
    }

    // ───────────────────────────── rev-parse ─────────────────────────

    fn git_rev_parse(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("rev-parse") {
            Ok(r) => r,
            Err(e) => return e,
        };

        let abbrev_ref = args.iter().any(|a| *a == "--abbrev-ref");
        let short = args.iter().any(|a| *a == "--short");
        let spec = args
            .iter()
            .find(|a| !a.starts_with('-'))
            .copied()
            .unwrap_or("HEAD");

        if abbrev_ref {
            return CommandOutput::success(format!("{}\n", head_branch_name(&repo)));
        }

        let out = match repo.revparse_single(spec) {
            Ok(obj) => {
                let id = obj.id().to_string();
                if short {
                    CommandOutput::success(format!("{}\n", &id[..7.min(id.len())]))
                } else {
                    CommandOutput::success(format!("{}\n", id))
                }
            }
            Err(e) => CommandOutput::error(format!("git rev-parse: {}: {}\n", spec, e), 128),
        };
        out
    }

    // ───────────────────────────── show ──────────────────────────────

    fn git_show(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("show") {
            Ok(r) => r,
            Err(e) => return e,
        };
        let spec = args
            .iter()
            .find(|a| !a.starts_with('-'))
            .copied()
            .unwrap_or("HEAD");

        let obj = match repo.revparse_single(spec) {
            Ok(o) => o,
            Err(e) => return CommandOutput::error(format!("git show: {}: {}\n", spec, e), 128),
        };
        let commit = match obj.peel_to_commit() {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("git show: {}\n", e), 1),
        };

        let mut output = String::new();
        let author = commit.author();
        output.push_str(&format!("commit {}\n", commit.id()));
        output.push_str(&format!(
            "Author: {} <{}>\n",
            author.name().unwrap_or(""),
            author.email().unwrap_or("")
        ));
        output.push_str(&format!(
            "Date:   {}\n\n",
            format_timestamp(commit.time().seconds())
        ));
        for line in commit.message().unwrap_or("").lines() {
            output.push_str(&format!("    {}\n", line));
        }
        output.push('\n');

        // Diff against the first parent (or the empty tree for the root).
        let commit_tree = match commit.tree() {
            Ok(t) => t,
            Err(e) => return CommandOutput::error(format!("git show: {}\n", e), 1),
        };
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
        let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), None) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("git show: {}\n", e), 1),
        };
        let diff_out = format_git_diff(&diff);
        output.push_str(&diff_out.stdout);
        CommandOutput::success(output)
    }

    // ───────────────────────────── rm / mv ───────────────────────────

    fn git_rm(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("rm") {
            Ok(r) => r,
            Err(e) => return e,
        };
        let cached = args.iter().any(|a| *a == "--cached");
        let paths: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if paths.is_empty() {
            return CommandOutput::error("git rm: missing paths\n".to_string(), 1);
        }
        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => return CommandOutput::error(format!("git rm: {}\n", e), 1),
        };
        let mut output = String::new();
        for p in &paths {
            if let Err(e) = index.remove_path(std::path::Path::new(p)) {
                return CommandOutput::error(format!("git rm: {}: {}\n", p, e), 1);
            }
            if !cached {
                let _ = self.vfs.remove_file(p, &self.cwd);
            }
            output.push_str(&format!("rm '{}'\n", p));
        }
        if let Err(e) = index.write() {
            return CommandOutput::error(format!("git rm: {}\n", e), 1);
        }
        CommandOutput::success(output)
    }

    fn git_mv(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("mv") {
            Ok(r) => r,
            Err(e) => return e,
        };
        let paths: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if paths.len() != 2 {
            return CommandOutput::error(
                "git mv: usage: git mv <source> <destination>\n".to_string(),
                1,
            );
        }
        let (src, dst) = (paths[0], paths[1]);

        // Move on disk (VFS-jailed).
        let src_abs = self.git_repo_path().join(src);
        let dst_abs = self.git_repo_path().join(dst);
        if let Err(e) = std::fs::rename(&src_abs, &dst_abs) {
            return CommandOutput::error(format!("git mv: {}: {}\n", src, e), 1);
        }

        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => return CommandOutput::error(format!("git mv: {}\n", e), 1),
        };
        if let Err(e) = index.remove_path(std::path::Path::new(src)) {
            return CommandOutput::error(format!("git mv: {}: {}\n", src, e), 1);
        }
        if let Err(e) = index.add_path(std::path::Path::new(dst)) {
            return CommandOutput::error(format!("git mv: {}: {}\n", dst, e), 1);
        }
        if let Err(e) = index.write() {
            return CommandOutput::error(format!("git mv: {}\n", e), 1);
        }
        CommandOutput::success(String::new())
    }

    // ───────────────────────────── restore ───────────────────────────

    fn git_restore(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("restore") {
            Ok(r) => r,
            Err(e) => return e,
        };
        let staged = args.iter().any(|a| *a == "--staged");
        let paths: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-') && **a != "--")
            .copied()
            .collect();
        if paths.is_empty() {
            return CommandOutput::error("git restore: missing paths\n".to_string(), 1);
        }

        if staged {
            // Unstage: index entry ← HEAD (same as `git reset -- <paths>`).
            let head_obj = repo.head().ok().and_then(|h| h.peel(git2::ObjectType::Commit).ok());
            return match repo.reset_default(head_obj.as_ref(), &paths) {
                Ok(_) => CommandOutput::success(String::new()),
                Err(e) => CommandOutput::error(format!("git restore --staged: {}\n", e), 1),
            };
        }

        // Working tree ← index.
        let mut cb = git2::build::CheckoutBuilder::new();
        cb.force();
        for p in &paths {
            cb.path(p);
        }
        match repo.checkout_index(None, Some(&mut cb)) {
            Ok(_) => CommandOutput::success(String::new()),
            Err(e) => CommandOutput::error(format!("git restore: {}\n", e), 1),
        }
    }

    // ───────────────────────────── config ────────────────────────────

    fn git_config(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("config") {
            Ok(r) => r,
            Err(e) => return e,
        };
        let filtered: Vec<&str> = args
            .iter()
            .filter(|a| **a != "--global" && **a != "--local")
            .copied()
            .collect();
        let mut config = match repo.config() {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("git config: {}\n", e), 1),
        };
        match filtered.len() {
            1 => match config.get_string(filtered[0]) {
                Ok(v) => CommandOutput::success(format!("{}\n", v)),
                Err(_) => CommandOutput::error(String::new(), 1),
            },
            2 => match config.set_str(filtered[0], filtered[1]) {
                Ok(_) => CommandOutput::success(String::new()),
                Err(e) => CommandOutput::error(format!("git config: {}\n", e), 1),
            },
            _ => CommandOutput::error(
                "git config: usage: git config <key> [value]\n".to_string(),
                1,
            ),
        }
    }

    // ───────────────────────────── clean ─────────────────────────────

    fn git_clean(&self, args: &[&str]) -> CommandOutput {
        let repo = match self.open_repo("clean") {
            Ok(r) => r,
            Err(e) => return e,
        };
        let force = args.iter().any(|a| a.contains('f'));
        let dirs = args.iter().any(|a| a.contains('d'));
        if !force {
            return CommandOutput::error(
                "git clean: refusing to clean without -f\n".to_string(),
                1,
            );
        }

        let mut opts = git2::StatusOptions::new();
        // Do NOT recurse untracked dirs: an untracked directory then shows as
        // a single "dir/" entry, which `clean -d` removes wholesale.
        opts.include_untracked(true).recurse_untracked_dirs(false);
        let statuses = match repo.statuses(Some(&mut opts)) {
            Ok(s) => s,
            Err(e) => return CommandOutput::error(format!("git clean: {}\n", e), 1),
        };

        let mut output = String::new();
        for entry in statuses.iter() {
            if !entry.status().contains(git2::Status::WT_NEW) {
                continue;
            }
            let path = match entry.path() {
                Some(p) => p.trim_end_matches('/').to_string(),
                None => continue,
            };
            let abs = self.git_repo_path().join(&path);
            if abs.is_dir() {
                if dirs {
                    let _ = std::fs::remove_dir_all(&abs);
                    output.push_str(&format!("Removing {}/\n", path));
                }
            } else {
                let _ = std::fs::remove_file(&abs);
                output.push_str(&format!("Removing {}\n", path));
            }
        }
        CommandOutput::success(output)
    }
}

// ═════════════════════════ helpers (module-level) ═════════════════════════

/// Current branch shorthand; falls back to "master" for unborn/empty repos.
#[cfg(feature = "git")]
fn head_branch_name(repo: &git2::Repository) -> String {
    match repo.head() {
        Ok(h) => h.shorthand().unwrap_or("HEAD").to_string(),
        Err(_) => {
            // Unborn branch: read HEAD symbolic target.
            repo.find_reference("HEAD")
                .ok()
                .and_then(|r| r.symbolic_target().map(|s| s.to_string()))
                .map(|s| s.trim_start_matches("refs/heads/").to_string())
                .unwrap_or_else(|| "master".to_string())
        }
    }
}

/// Upstream branch name + (ahead, behind) counts, when configured.
#[cfg(feature = "git")]
fn upstream_info(repo: &git2::Repository) -> Option<(String, usize, usize)> {
    let head = repo.head().ok()?;
    let branch_name = head.shorthand()?;
    let branch = repo.find_branch(branch_name, git2::BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;
    let upstream_name = upstream.name().ok()??.to_string();
    let local_oid = head.target()?;
    let upstream_oid = upstream.get().target()?;
    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid).ok()?;
    Some((upstream_name, ahead, behind))
}

/// Porcelain XY status codes (git status --porcelain).
#[cfg(feature = "git")]
fn porcelain_codes(status: git2::Status) -> (char, char) {
    if status.contains(git2::Status::WT_NEW) && !status.contains(git2::Status::INDEX_NEW) {
        return ('?', '?');
    }
    let x = if status.contains(git2::Status::INDEX_NEW) {
        'A'
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        'M'
    } else if status.contains(git2::Status::INDEX_DELETED) {
        'D'
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        'R'
    } else if status.contains(git2::Status::INDEX_TYPECHANGE) {
        'T'
    } else {
        ' '
    };
    let y = if status.contains(git2::Status::WT_MODIFIED) {
        'M'
    } else if status.contains(git2::Status::WT_DELETED) {
        'D'
    } else if status.contains(git2::Status::WT_RENAMED) {
        'R'
    } else if status.contains(git2::Status::WT_TYPECHANGE) {
        'T'
    } else {
        ' '
    };
    (x, y)
}

/// repo.signature() with a sandbox-safe fallback (mobile devices usually have
/// no global git config).
#[cfg(feature = "git")]
fn signature_or_default(repo: &git2::Repository) -> git2::Signature<'static> {
    repo.signature()
        .or_else(|_| git2::Signature::now("aacode", "aacode@local"))
        .expect("static signature is always valid")
}

/// Prepend an "Authentication failed" hint when the error indicates a
/// credential problem. The AACodeApp GitManager.kt parses those tokens.
#[cfg(feature = "git")]
fn auth_hint(e: &git2::Error) -> String {
    let msg = format!("{}", e);
    let lower = msg.to_lowercase();
    if lower.contains("auth") || msg.contains("403") || msg.contains("401") {
        format!("Authentication failed: {}", msg)
    } else if lower.contains("certificate") || lower.contains("ssl") {
        let diag = SSL_INIT_MSG.get().map(|s| s.as_str()).unwrap_or("ssl not initialized");
        format!("{} [{}]", msg, diag)
    } else {
        msg
    }
}

/// Credential callbacks: token from env (GIT_TOKEN / GIT_PASSWORD /
/// GIT_USERNAME), URL-embedded credentials are handled by libgit2 itself.
#[cfg(feature = "git")]
static SSL_INIT_MSG: std::sync::OnceLock<String> = std::sync::OnceLock::new();

#[cfg(feature = "git")]
fn init_ssl_certs() {
    use std::sync::Once;
    static SSL_CERT_INIT: Once = Once::new();
    SSL_CERT_INIT.call_once(|| {
        match std::env::var("SSL_CERT_FILE") {
            Ok(path) if std::path::Path::new(&path).exists() => {
                let r = unsafe { git2::opts::set_ssl_cert_file(&path) };
                let _ = SSL_INIT_MSG.set(match r {
                    Ok(_) => format!("ssl_cert_file={} ok", path),
                    Err(e) => format!("ssl_cert_file={} err={}", path, e),
                });
            }
            Ok(path) => {
                let _ = SSL_INIT_MSG.set(format!("ssl_cert_file={} missing", path));
            }
            Err(_) => {
                let _ = SSL_INIT_MSG.set("no SSL_CERT_FILE env".to_string());
            }
        }
    });
}

#[cfg(feature = "git")]
fn remote_callbacks<'a>() -> git2::RemoteCallbacks<'a> {
    let mut cb = git2::RemoteCallbacks::new();
    // Escape hatch for environments where OpenSSL cannot reach a usable CA
    // store (e.g. Android). The host app opts in explicitly via env var.
    if std::env::var("FASTSHELL_GIT_ACCEPT_INVALID_CERTS").as_deref() == Ok("1") {
        cb.certificate_check(|_cert, _host| Ok(git2::CertificateCheckStatus::CertificateOk));
    }
    cb.credentials(|_url, username_from_url, allowed| {
        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            let user = std::env::var("GIT_USERNAME")
                .ok()
                .or_else(|| username_from_url.map(|s| s.to_string()))
                .unwrap_or_else(|| "git".to_string());
            if let Ok(token) =
                std::env::var("GIT_TOKEN").or_else(|_| std::env::var("GIT_PASSWORD"))
            {
                if !token.is_empty() {
                    return git2::Cred::userpass_plaintext(&user, &token);
                }
            }
        }
        if allowed.contains(git2::CredentialType::DEFAULT) {
            return git2::Cred::default();
        }
        Err(git2::Error::from_str(
            "authentication required but no credentials available (set GIT_TOKEN or embed in URL)",
        ))
    });
    cb
}

/// Renders a diff as a unified patch. Exit code 0 (matches real `git diff`).
#[cfg(feature = "git")]
fn format_git_diff(diff: &git2::Diff) -> CommandOutput {
    let mut output = String::new();

    let print_result = diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let origin = line.origin();
        let content = std::str::from_utf8(line.content()).unwrap_or("");
        match origin {
            '+' | '-' | ' ' => {
                output.push(origin);
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

    CommandOutput::success(output)
}

/// Formats a commit according to a `--format=`/`--pretty=format:` string.
/// Supports: %H %h %an %ae %s %b %ci %ar %at %d %n and %%.
#[cfg(feature = "git")]
fn format_commit(commit: &git2::Commit, fmt: &str) -> String {
    let full = commit.id().to_string();
    let short: String = full.chars().take(7).collect();
    let author = commit.author();
    let msg = commit.message().unwrap_or("");
    let subject = msg.lines().next().unwrap_or("").to_string();
    let body: String = msg.lines().skip(1).collect::<Vec<_>>().join("\n");
    let secs = commit.time().seconds();
    let offset = commit.time().offset_minutes();

    let mut out = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            let c = chars[i + 1];
            match c {
                'H' => out.push_str(&full),
                'h' => out.push_str(&short),
                'a' if i + 2 < chars.len() => {
                    let c2 = chars[i + 2];
                    match c2 {
                        'n' => out.push_str(author.name().unwrap_or("")),
                        'e' => out.push_str(author.email().unwrap_or("")),
                        'r' => out.push_str(&relative_time(secs)),
                        't' => out.push_str(&secs.to_string()),
                        _ => {
                            out.push('%');
                            out.push('a');
                            out.push(c2);
                        }
                    }
                    i += 3;
                    continue;
                }
                'c' if i + 2 < chars.len() && chars[i + 2] == 'i' => {
                    out.push_str(&iso_time(secs, offset));
                    i += 3;
                    continue;
                }
                's' => out.push_str(&subject),
                'b' => out.push_str(&body),
                'd' => {} // ref decorations — omitted
                'n' => out.push('\n'),
                '%' => out.push('%'),
                _ => {
                    out.push('%');
                    out.push(c);
                }
            }
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// "x seconds/minutes/hours/days/weeks/months/years ago"
#[cfg(feature = "git")]
fn relative_time(commit_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(commit_secs);
    let delta = (now - commit_secs).max(0);
    match delta {
        0..=59 => format!("{} seconds ago", delta),
        60..=3599 => format!("{} minutes ago", delta / 60),
        3600..=86399 => format!("{} hours ago", delta / 3600),
        86400..=604799 => format!("{} days ago", delta / 86400),
        604800..=2591999 => format!("{} weeks ago", delta / 604800),
        2592000..=31535999 => format!("{} months ago", delta / 2592000),
        _ => format!("{} years ago", delta / 31536000),
    }
}

/// ISO-8601-like timestamp: "YYYY-MM-DD HH:MM:SS +ZZZZ"
#[cfg(feature = "git")]
fn iso_time(secs: i64, offset_minutes: i32) -> String {
    let local = secs + (offset_minutes as i64) * 60;
    let (year, month, day, h, m, s) = civil_from_unix(local);
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let om = offset_minutes.abs();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}{:02}{:02}",
        year,
        month,
        day,
        h,
        m,
        s,
        sign,
        om / 60,
        om % 60
    )
}

/// Converts unix seconds to (year, month, day, hour, min, sec).
#[cfg(feature = "git")]
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400);
    let (h, m, s) = ((tod / 3600) as u32, ((tod % 3600) / 60) as u32, (tod % 60) as u32);

    let mut year: i64 = 1970;
    let mut remaining = days;
    loop {
        let dy = if is_leap(year) { 366 } else { 365 };
        if remaining < dy {
            break;
        }
        remaining -= dy;
        year += 1;
    }
    let month_days = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 0usize;
    for (idx, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 {
            month = idx;
            break;
        }
        remaining -= md as i64;
    }
    (year, (month + 1) as u32, (remaining + 1) as u32, h, m, s)
}

#[cfg(feature = "git")]
fn format_timestamp(seconds: i64) -> String {
    let (year, month, day, h, m, s) = civil_from_unix(seconds);
    let month_names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{} {} {:02}:{:02}:{:02} {} +0000",
        month_names[(month - 1) as usize],
        day,
        h,
        m,
        s,
        year
    )
}

#[cfg(feature = "git")]
fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
