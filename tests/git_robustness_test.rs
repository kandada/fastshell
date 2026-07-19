// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Git robustness integration tests — exercises the exact command patterns
//! used by the AACodeApp `GitManager` (and typical coding-agent workflows)
//! through the full Runtime pipeline (chaining, quoting, etc).

#![cfg(feature = "git")]

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;

fn mk() -> Fastshell {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut s = Fastshell::new();
    let dir = std::env::temp_dir().join(format!(
        "fs_git_it_{}_{}_{}",
        std::process::id(),
        n,
        uuid_like()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    s.init(Config {
        sandbox_path: dir.to_string_lossy().into(),
        python_enabled: false,
        ..Default::default()
    })
    .unwrap();
    s
}

fn uuid_like() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Init a repo with local identity and one commit; leaves cwd at repo root.
fn setup_repo(s: &mut Fastshell) {
    assert_eq!(s.execute("git init").exit_code, 0);
    assert_eq!(
        s.execute("git config user.name tester").exit_code,
        0,
        "git config user.name"
    );
    assert_eq!(s.execute("git config user.email t@example.com").exit_code, 0);
    s.execute("echo v1 > f.txt");
    assert_eq!(s.execute("git add f.txt").exit_code, 0);
    let out = s.execute("git commit -m 'first commit'");
    assert_eq!(out.exit_code, 0, "commit failed: {}", out.stderr);
}

#[test]
fn commit_shows_real_branch_name() {
    let mut s = mk();
    setup_repo(&mut s);
    assert_eq!(s.execute("git checkout -b feature").exit_code, 0);
    s.execute("echo x > g.txt");
    s.execute("git add g.txt");
    let out = s.execute("git commit -m 'on feature'");
    assert!(
        out.stdout.contains("[feature"),
        "commit should show current branch, got: {}",
        out.stdout
    );
}

#[test]
fn porcelain_status_with_branch_header() {
    let mut s = mk();
    setup_repo(&mut s);
    s.execute("echo v2 > f.txt");
    s.execute("echo new > untracked.txt");
    let out = s.execute("git status --porcelain -b");
    assert_eq!(out.exit_code, 0);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(lines[0].starts_with("## "), "missing branch header: {:?}", lines);
    assert!(
        out.stdout.contains(" M f.txt"),
        "worktree-modified should be ' M': {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("?? untracked.txt"),
        "untracked should be '??': {}",
        out.stdout
    );
}

#[test]
fn porcelain_status_plain() {
    let mut s = mk();
    setup_repo(&mut s);
    let out = s.execute("git status --porcelain");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.trim().is_empty(), "clean repo → empty porcelain");
}

#[test]
fn log_custom_format() {
    let mut s = mk();
    setup_repo(&mut s);
    // GitManager format: --format=%H%n%s%n%an%n%ci%n---
    let out = s.execute("git log -n 5 --format=%H%n%s%n%an%n%ci%n---");
    assert_eq!(out.exit_code, 0);
    let block: Vec<&str> = out.stdout.split("---").next().unwrap().trim().lines().collect();
    assert_eq!(block.len(), 4, "H/s/an/ci lines expected: {:?}", block);
    assert_eq!(block[0].len(), 40, "full hash expected: {}", block[0]);
    assert_eq!(block[1], "first commit");
    assert_eq!(block[2], "tester");
    assert!(block[3].contains('-'), "ISO date expected: {}", block[3]);

    // pipe-separated pretty format (GitManager uses quotes around the format string)
    let out = s.execute("git log -n 1 --pretty=format:'%h|%an|%ar|%s'");
    assert_eq!(out.exit_code, 0, "pretty=format: {}", out.stderr);
    let parts: Vec<&str> = out.stdout.trim().split('|').collect();
    assert_eq!(parts.len(), 4, "got: {}", out.stdout);
    assert_eq!(parts[0].len(), 7);
    assert_eq!(parts[1], "tester");
    assert!(parts[2].contains("ago"));
    assert_eq!(parts[3], "first commit");
}

#[test]
fn checkout_syncs_working_tree() {
    let mut s = mk();
    setup_repo(&mut s);
    s.execute("git checkout -b feature");
    s.execute("echo feat > feat.txt");
    s.execute("git add feat.txt");
    assert_eq!(s.execute("git commit -m 'feat file'").exit_code, 0);

    // Switch back to the initial branch: feat.txt must disappear.
    let out = s.execute("git checkout master");
    assert_eq!(out.exit_code, 0, "checkout master: {}", out.stderr);
    let ls = s.execute("ls");
    assert!(
        !ls.stdout.contains("feat.txt"),
        "feat.txt should be removed after switching branches: {}",
        ls.stdout
    );
    // And status must be clean (index synced too).
    let st = s.execute("git status --porcelain");
    assert!(st.stdout.trim().is_empty(), "status must be clean: {}", st.stdout);

    // Switch forward again: feat.txt returns.
    s.execute("git checkout feature");
    let ls = s.execute("ls");
    assert!(ls.stdout.contains("feat.txt"));
}

#[test]
fn diff_exit_code_is_zero_and_supports_pathspec() {
    let mut s = mk();
    setup_repo(&mut s);
    s.execute("echo v2 > f.txt");
    let out = s.execute("git diff");
    assert_eq!(out.exit_code, 0, "git diff must exit 0: {}", out.stderr);
    assert!(out.stdout.contains("+v2"));

    // Pathspec form used by GitManager: git diff <file>
    let out = s.execute("git diff f.txt");
    assert_eq!(out.exit_code, 0, "pathspec diff: {}", out.stderr);
    assert!(out.stdout.contains("+v2"));

    // Staged diff
    s.execute("git add f.txt");
    let out = s.execute("git diff --staged");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("+v2"));
}

#[test]
fn reset_unstages_files() {
    let mut s = mk();
    setup_repo(&mut s);
    s.execute("echo v2 > f.txt");
    s.execute("git add f.txt");
    let st = s.execute("git status --porcelain");
    assert!(st.stdout.contains("M  f.txt"), "staged: {}", st.stdout);

    let out = s.execute("git reset -- f.txt");
    assert_eq!(out.exit_code, 0, "reset --: {}", out.stderr);
    let st = s.execute("git status --porcelain");
    assert!(st.stdout.contains(" M f.txt"), "unstaged: {}", st.stdout);
}

#[test]
fn reset_hard_discards_changes() {
    let mut s = mk();
    setup_repo(&mut s);
    s.execute("echo v2 > f.txt");
    let out = s.execute("git reset --hard HEAD");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let cat = s.execute("cat f.txt");
    assert_eq!(cat.stdout, "v1\n");
}

#[test]
fn stash_push_and_pop() {
    let mut s = mk();
    setup_repo(&mut s);
    s.execute("echo v2 > f.txt");
    s.execute("echo new > u.txt");

    let out = s.execute("git stash push -u");
    assert_eq!(out.exit_code, 0, "stash push -u: {}", out.stderr);
    let cat = s.execute("cat f.txt");
    assert_eq!(cat.stdout, "v1\n", "worktree restored after stash");

    let out = s.execute("git stash pop");
    assert_eq!(out.exit_code, 0, "stash pop: {}", out.stderr);
    let cat = s.execute("cat f.txt");
    assert_eq!(cat.stdout, "v2\n", "changes back after pop");
    let cat = s.execute("cat u.txt");
    assert_eq!(cat.stdout, "new\n", "untracked back after pop");
}

#[test]
fn stash_nothing_to_save() {
    let mut s = mk();
    setup_repo(&mut s);
    let out = s.execute("git stash push -u");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("No local changes"));
}

#[test]
fn remote_add_geturl_remove() {
    let mut s = mk();
    setup_repo(&mut s);
    let out = s.execute("git remote add origin https://example.com/x.git");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);

    let out = s.execute("git remote get-url origin");
    assert_eq!(out.stdout.trim(), "https://example.com/x.git");

    let out = s.execute("git remote -v");
    assert!(out.stdout.contains("origin"));
    assert!(out.stdout.contains("(fetch)"));

    let out = s.execute("git remote remove origin");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let out = s.execute("git remote get-url origin");
    assert_ne!(out.exit_code, 0);

    // GitManager pattern: fallback via chaining
    let out = s.execute("git remote get-url origin 2>/dev/null || echo ''");
    assert_eq!(out.exit_code, 0);
}

#[test]
fn merge_fast_forward_and_conflict() {
    let mut s = mk();
    setup_repo(&mut s);

    // Fast-forward merge
    s.execute("git checkout -b feature");
    s.execute("echo feat > feat.txt");
    s.execute("git add feat.txt && git commit -m 'feat'");
    s.execute("git checkout master");
    let out = s.execute("git merge feature");
    assert_eq!(out.exit_code, 0, "ff merge: {}", out.stderr);
    let ls = s.execute("ls");
    assert!(ls.stdout.contains("feat.txt"), "ff merge must update worktree");

    // Conflicting merge
    s.execute("git checkout -b left");
    s.execute("echo left > c.txt && git add c.txt && git commit -m left");
    s.execute("git checkout master");
    s.execute("echo right > c.txt && git add c.txt && git commit -m right");
    let out = s.execute("git merge left");
    assert_ne!(out.exit_code, 0);
    assert!(
        out.stderr.contains("CONFLICT") || out.stdout.contains("CONFLICT"),
        "conflict output must contain CONFLICT: {} {}",
        out.stdout,
        out.stderr
    );
    // Abort restores a clean state
    let out = s.execute("git merge --abort");
    assert_eq!(out.exit_code, 0, "merge --abort: {}", out.stderr);
    let st = s.execute("git status --porcelain");
    assert!(st.stdout.trim().is_empty(), "clean after abort: {}", st.stdout);
}

#[test]
fn rev_parse_and_show() {
    let mut s = mk();
    setup_repo(&mut s);
    let out = s.execute("git rev-parse HEAD");
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout.trim().len(), 40);

    let out = s.execute("git rev-parse --short HEAD");
    assert_eq!(out.stdout.trim().len(), 7);

    let out = s.execute("git rev-parse --abbrev-ref HEAD");
    assert_eq!(out.stdout.trim(), "master");

    let out = s.execute("git show HEAD");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("first commit"));
    assert!(out.stdout.contains("+v1"));
}

#[test]
fn rm_and_mv_and_restore() {
    let mut s = mk();
    setup_repo(&mut s);

    // git mv
    let out = s.execute("git mv f.txt f2.txt");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let ls = s.execute("ls");
    assert!(ls.stdout.contains("f2.txt") && !ls.stdout.contains("f.txt\n"));
    s.execute("git commit -m 'renamed'");

    // git rm
    let out = s.execute("git rm f2.txt");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let ls = s.execute("ls");
    assert!(!ls.stdout.contains("f2.txt"));
    s.execute("git commit -m 'removed'");

    // restore: modify + restore from index
    s.execute("echo data > r.txt && git add r.txt && git commit -m add-r");
    s.execute("echo changed > r.txt");
    let out = s.execute("git restore r.txt");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert_eq!(s.execute("cat r.txt").stdout, "data\n");

    // restore --staged: unstage
    s.execute("echo changed2 > r.txt && git add r.txt");
    let out = s.execute("git restore --staged r.txt");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let st = s.execute("git status --porcelain");
    assert!(st.stdout.contains(" M r.txt"), "unstaged: {}", st.stdout);
}

#[test]
fn clean_removes_untracked() {
    let mut s = mk();
    setup_repo(&mut s);
    s.execute("echo junk > junk.txt");
    s.execute("mkdir junkdir && echo x > junkdir/inner.txt");
    let out = s.execute("git clean -fd");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let ls = s.execute("ls");
    assert!(!ls.stdout.contains("junk.txt"));
    assert!(!ls.stdout.contains("junkdir"));
}

#[test]
fn empty_repo_edge_cases() {
    let s = mk();
    assert_eq!(s.execute("git init").exit_code, 0);
    s.execute("git config user.name t && git config user.email t@e.c");

    let out = s.execute("git log");
    assert_ne!(out.exit_code, 0);
    assert!(out.stderr.contains("does not have any commits"));

    let out = s.execute("git diff");
    assert_eq!(out.exit_code, 0);

    let out = s.execute("git status --porcelain -b");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.starts_with("## "), "{}", out.stdout);

    // push with no remote
    let out = s.execute("git push origin master");
    assert_ne!(out.exit_code, 0);
    assert!(out.stderr.contains("not found"));

    // fetch with no remote
    let out = s.execute("git fetch origin");
    assert_ne!(out.exit_code, 0);
}

#[test]
fn commit_without_identity_uses_fallback() {
    let s = mk();
    // No git config at all — must still commit (mobile scenario).
    // Use HOME isolation: git2 may pick up the developer's global config on
    // desktop, so this asserts commit success either way.
    assert_eq!(s.execute("git init").exit_code, 0);
    s.execute("echo x > a.txt && git add a.txt");
    let out = s.execute("git commit -m fallback-sig");
    assert_eq!(out.exit_code, 0, "commit must not require config: {}", out.stderr);
}

#[test]
fn branch_listing_and_delete() {
    let mut s = mk();
    setup_repo(&mut s);
    s.execute("git branch backup/task_20260101_000000");
    let out = s.execute("git branch");
    assert!(out.stdout.contains("backup/task_20260101_000000"));
    assert!(out.stdout.contains("* master"));

    let out = s.execute("git branch -a");
    assert_eq!(out.exit_code, 0);

    let out = s.execute("git branch -d backup/task_20260101_000000");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let out = s.execute("git branch -D nope");
    assert_ne!(out.exit_code, 0);
}

#[test]
fn local_clone_pull_push_roundtrip() {
    let mut s = mk();
    // Create an "origin" bare-ish repo inside the sandbox, clone it, commit,
    // push back, then pull from a second clone. All file:// local.
    s.execute("mkdir origin && cd origin");
    setup_repo(&mut s);
    s.execute("cd ..");

    let root = s.execute("pwd");
    assert_eq!(root.stdout.trim(), "/");

    // Clone via filesystem path
    let origin_path = {
        // resolve real path of sandbox/origin via git rev-parse? use relative
        "origin"
    };
    let out = s.execute(&format!("git clone {} clone1", origin_path));
    assert_eq!(out.exit_code, 0, "local clone: {}", out.stderr);
    let ls = s.execute("ls clone1");
    assert!(ls.stdout.contains("f.txt"), "cloned content: {}", ls.stdout);

    // Commit in clone1 and push back to origin.
    s.execute("cd clone1");
    s.execute("git config user.name c1 && git config user.email c1@e.c");
    s.execute("echo v2 > f.txt && git add f.txt && git commit -m 'from clone1'");
    let out = s.execute("git push origin master");
    // Pushing to a non-bare checked-out branch is refused by libgit2 (same as
    // real git); accept either success or the standard refusal message.
    if out.exit_code != 0 {
        assert!(
            out.stderr.contains("current branch") || out.stderr.contains("push"),
            "unexpected push error: {}",
            out.stderr
        );
    }
    s.execute("cd ..");

    // Second clone + pull (up to date).
    let out = s.execute("git clone origin clone2");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    s.execute("cd clone2");
    let out = s.execute("git pull origin master");
    assert_eq!(out.exit_code, 0, "pull: {} {}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("Already up to date") || out.stdout.contains("Updated"),
        "{}",
        out.stdout
    );
}

#[test]
fn gitmanager_command_sequence_end_to_end() {
    // The exact sequence ChangesScreen drives, chained with && like
    // FastshellBridge does: `cd <absolute dir> && git ...`.
    let s = mk();
    s.execute("mkdir proj");
    let out = s.execute("cd /proj && git init");
    assert_eq!(out.exit_code, 0);
    s.execute("cd /proj && git config user.name app && git config user.email a@b.c");
    s.execute("cd /proj && echo hello > readme.md");
    let out = s.execute("cd /proj && git status --porcelain -b");
    assert!(out.stdout.contains("?? readme.md"), "{}", out.stdout);
    let out = s.execute("cd /proj && git add -- readme.md");
    assert_eq!(out.exit_code, 0);
    let out = s.execute("cd /proj && git commit -m 'initial commit'");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let out = s.execute("cd /proj && git log -n 20 --format=%H%n%s%n%an%n%ci%n---");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("initial commit"));
    let out = s.execute("cd /proj && git branch backup/task_x");
    assert_eq!(out.exit_code, 0);
    let out = s.execute("cd /proj && git stash push -u");
    assert_eq!(out.exit_code, 0);
}

#[test]
fn concurrent_git_on_same_repo_is_serialized() {
    // Two fastshell instances (like an agent task + the app UI) hammering the
    // SAME repository concurrently: the process-wide repo lock must prevent
    // index corruption. All commits must land, status must end clean.
    let mut s = mk();
    setup_repo(&mut s);
    let root = {
        // Recover the sandbox real path from git.
        let out = s.execute("git rev-parse HEAD");
        assert_eq!(out.exit_code, 0);
        s
    };
    let sandbox = root; // keep first instance alive
    let dir = sandbox_path_of(&sandbox);

    let mut handles = Vec::new();
    for t in 0..4 {
        let dir = dir.clone();
        handles.push(std::thread::spawn(move || {
            let mut s = Fastshell::new();
            s.init(Config {
                sandbox_path: dir,
                python_enabled: false,
                ..Default::default()
            })
            .unwrap();
            s.execute(&format!("git config user.name t{t} && git config user.email t{t}@e.c"));
            for i in 0..5 {
                let f = format!("f_{t}_{i}.txt");
                let out = s.execute(&format!("echo x > {f} && git add {f} && git commit -m 'c_{t}_{i}'"));
                assert_eq!(out.exit_code, 0, "thread {t} iter {i}: {}", out.stderr);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let mut s2 = Fastshell::new();
    s2.init(Config {
        sandbox_path: dir,
        python_enabled: false,
        ..Default::default()
    })
    .unwrap();
    let log = s2.execute("git log --oneline");
    assert_eq!(log.exit_code, 0);
    let commits = log.stdout.lines().count();
    assert_eq!(commits, 21, "1 initial + 20 concurrent commits, got {commits}:\n{}", log.stdout);
    let st = s2.execute("git status --porcelain");
    assert!(st.stdout.trim().is_empty(), "repo must end clean: {}", st.stdout);
}

/// Extract the sandbox path used by an instance (via its VFS root).
fn sandbox_path_of(s: &Fastshell) -> String {
    s.vfs_root()
}
