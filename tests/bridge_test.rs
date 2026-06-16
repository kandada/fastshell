use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup_dir() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir()
        .join(format!("fastshell_int_bridge_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn setup_runtime() -> fastshell::bridge::Runtime {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    let shell = fastshell::shell::Shell::new(vfs);
    let python: Option<Box<dyn fastshell::python::PythonEngine>> =
        Some(Box::new(fastshell::python::SubprocessPython::new()));
    fastshell::bridge::Runtime::new(shell, python)
}

#[test]
fn test_runtime_shell_commands() {
    let mut rt = setup_runtime();

    let result = rt.execute("echo bridge_test");
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("bridge_test"));

    let result = rt.execute("pwd");
    assert_eq!(result.stdout.trim(), "/");
}

#[test]
fn test_runtime_file_operations() {
    let mut rt = setup_runtime();

    rt.execute("mkdir test");
    rt.execute("cd test");
    rt.execute("touch data.txt");

    let result = rt.execute("ls");
    assert!(result.stdout.contains("data.txt"));
}

#[test]
fn test_runtime_python_code() {
    let mut rt = setup_runtime();
    let result = rt.execute_python_code("print(42)");

    if rt.python_available() {
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("42"));
    }
}

#[test]
fn test_runtime_empty_command() {
    let mut rt = setup_runtime();
    let result = rt.execute("");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_fs_bridge_operations() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    let bridge = fastshell::bridge::FsBridge::new(vfs);

    bridge.write_file("/int_test.txt", "fs bridge").unwrap();
    assert_eq!(bridge.read_file("/int_test.txt").unwrap(), "fs bridge");
    assert!(bridge.exists("/int_test.txt"));
    assert!(!bridge.exists("/nope.txt"));

    bridge.remove_file("/int_test.txt").unwrap();
    assert!(!bridge.exists("/int_test.txt"));
}
