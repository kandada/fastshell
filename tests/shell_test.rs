use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup_dir() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir()
        .join(format!("fastshell_int_shell_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn test_shell_basic_commands() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    let mut shell = fastshell::shell::Shell::new(vfs);

    let out = shell.execute("pwd", &[], None);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout.trim(), "/");

    let out = shell.execute("echo", &["hello", "shell"], None);
    assert_eq!(out.stdout.trim(), "hello shell");
}

#[test]
fn test_shell_file_workflow() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    let mut shell = fastshell::shell::Shell::new(vfs);

    shell.execute("mkdir", &["work"], None);
    shell.execute("cd", &["work"], None);
    assert_eq!(shell.cwd, "/work");

    shell.execute("touch", &["file.txt"], None);
    let out = shell.execute("ls", &[], None);
    assert!(out.stdout.contains("file.txt"));

    shell.execute("rm", &["file.txt"], None);
    let out = shell.execute("ls", &[], None);
    assert!(!out.stdout.contains("file.txt"));
}

#[test]
fn test_shell_grep_and_cat() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    let mut shell = fastshell::shell::Shell::new(vfs);

    let _ = shell.vfs.write("/text.txt", "", "line one\nline two\nLINE THREE\n");
    let out = shell.execute("cat", &["text.txt"], None);
    assert!(out.stdout.contains("line one"));
    assert!(out.stdout.contains("line two"));

    let out = shell.execute("grep", &["line", "text.txt"], None);
    assert_eq!(out.stdout.lines().count(), 2);

    let out = shell.execute("grep", &["-i", "line", "text.txt"], None);
    assert_eq!(out.stdout.lines().count(), 3);
}

#[test]
fn test_shell_error_handling() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    let mut shell = fastshell::shell::Shell::new(vfs);

    let out = shell.execute("cat", &["nonexistent.txt"], None);
    assert_ne!(out.exit_code, 0);

    let out = shell.execute("nonexistent_cmd", &[], None);
    assert_ne!(out.exit_code, 0);
}
