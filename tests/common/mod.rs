use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use fastshell::sdk::Fastshell;
use fastshell::sdk::types::Config;

static COMMON_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn unique_dir(prefix: &str) -> PathBuf {
    let n = COMMON_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), n))
}

pub fn setup_sdk() -> Fastshell {
    let dir = unique_dir("fastshell_common");
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
    }).unwrap();
    sdk
}

pub fn setup_sdk_no_subprocess() -> Fastshell {
    let dir = unique_dir("fastshell_common_ns");
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
    }).unwrap();
    sdk
}

pub fn setup_sdk_with_timeout(timeout_ms: u64) -> Fastshell {
    let dir = unique_dir("fastshell_common_to");
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: timeout_ms,
    }).unwrap();
    sdk
}

pub fn populate_vfs(sdk: &Fastshell) {
    sdk.execute("mkdir -p dir1/sub");
    sdk.execute("mkdir -p dir2");
    sdk.write_file("dir1/a.txt", "alpha").unwrap();
    sdk.write_file("dir1/sub/b.txt", "beta").unwrap();
    sdk.write_file("dir2/c.txt", "gamma").unwrap();
    sdk.write_file("root.txt", "i am root").unwrap();
    sdk.write_file("numbers.txt", "1\n2\n3\n4\n5\n").unwrap();
    sdk.write_file("words.txt", "apple\nbanana\nApple\ncherry\ndate\n").unwrap();
    sdk.write_file("data.csv", "name,age\nAlice,30\nBob,25\nCharlie,35\n").unwrap();
}

pub fn assert_cmd_ok(sdk: &Fastshell, cmd: &str) -> String {
    let r = sdk.execute(cmd);
    assert_eq!(r.exit_code, 0, "cmd '{}' failed: stderr={}", cmd, r.stderr);
    r.stdout
}

pub fn assert_cmd_contains(sdk: &Fastshell, cmd: &str, expected: &str) {
    let out = assert_cmd_ok(sdk, cmd);
    assert!(out.contains(expected), "cmd '{}' output does not contain '{}':\n{}", cmd, expected, out);
}

pub fn assert_cmd_stderr_contains(sdk: &Fastshell, cmd: &str, expected: &str) {
    let r = sdk.execute(cmd);
    assert!(r.stderr.contains(expected), "cmd '{}' stderr does not contain '{}':\nstderr={}\nstdout={}", cmd, expected, r.stderr, r.stdout);
}

pub fn assert_cmd_fails(sdk: &Fastshell, cmd: &str) -> String {
    let r = sdk.execute(cmd);
    assert_ne!(r.exit_code, 0, "cmd '{}' should have failed but succeeded", cmd);
    r.stderr
}

pub fn assert_file_content(sdk: &Fastshell, path: &str, expected: &str) {
    let content = sdk.read_file(path).unwrap();
    assert_eq!(content, expected, "file '{}' content mismatch", path);
}

pub fn assert_file_exists(sdk: &Fastshell, path: &str) {
    assert!(sdk.exists(path), "file '{}' should exist", path);
}

pub fn assert_file_not_exists(sdk: &Fastshell, path: &str) {
    assert!(!sdk.exists(path), "file '{}' should NOT exist", path);
}
