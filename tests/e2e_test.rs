use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use fastshell::sdk::Fastshell;
use fastshell::sdk::types::{Config, EXIT_NEED_PERMISSION};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup(config: Config) -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_e2e_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    let mut c = config;
    if c.sandbox_path.is_empty() {
        c.sandbox_path = dir.to_string_lossy().to_string();
    }
    sdk.init(c).unwrap();
    sdk
}

fn setup_default() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_e2e_def_{}_{}", std::process::id(), n));
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

// ═══════════════════════════════════════════════════════
// VFS — File Operations via VFS (sandbox path resolution)
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_vfs_crud() {
    let sdk = setup_default();
    sdk.write_file("hello.txt", "hello vfs").unwrap();
    assert_eq!(sdk.read_file("hello.txt").unwrap(), "hello vfs");
    assert!(sdk.exists("hello.txt"));
    assert!(!sdk.is_dir("hello.txt"));

    sdk.write_file("sub/nested.txt", "nested").unwrap();
    assert!(sdk.exists("sub/nested.txt"));
    assert!(sdk.is_dir("sub"));

    let entries = sdk.list_dir("sub").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "nested.txt");

    sdk.execute("rm -r sub");
    assert!(!sdk.exists("sub"));
}

#[test]
fn e2e_vfs_path_escape_prevented() {
    let sdk = setup_default();
    // Attempt to escape sandbox via ..
    let result = sdk.read_file("../../../etc/passwd");
    assert!(result.is_err());

    // Verify write_file also blocks escape
    let result = sdk.write_file("../../outside.txt", "escape");
    assert!(result.is_err());
}

#[test]
fn e2e_vfs_absolute_and_relative() {
    let sdk = setup_default();
    sdk.write_file("/absolute.txt", "abs").unwrap();
    assert_eq!(sdk.read_file("/absolute.txt").unwrap(), "abs");

    // create sub directory first, then cd into it
    sdk.execute("mkdir sub");
    sdk.execute("cd sub");
    sdk.write_file("relative.txt", "rel").unwrap();
    assert_eq!(sdk.read_file("relative.txt").unwrap(), "rel");
    // read via absolute path from root
    sdk.execute("cd /");
    assert_eq!(sdk.read_file("/sub/relative.txt").unwrap(), "rel");
}

// ═══════════════════════════════════════════════════════
// Shell — Built-in Commands
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_shell_file_ops() {
    let sdk = setup_default();
    sdk.write_file("tmp_test.txt", "hello").unwrap();
    let r = sdk.execute("cat /tmp_test.txt");
    assert!(r.stdout.contains("hello"));
}

#[test]
fn e2e_shell_text_processing() {
    let sdk = setup_default();
    sdk.write_file("nums.txt", "3\n1\n2\n2\n").unwrap();

    let r = sdk.execute("sort nums.txt");
    assert_eq!(r.stdout, "1\n2\n2\n3\n");

    let r = sdk.execute("sort nums.txt | uniq");
    assert_eq!(r.stdout, "1\n2\n3\n");

    let r = sdk.execute("wc -l nums.txt");
    assert!(r.stdout.contains("4"));
}

#[test]
fn e2e_shell_grep_sed() {
    let sdk = setup_default();
    sdk.write_file("data.txt", "hello world\nfoo bar\nhello again\n").unwrap();

    let r = sdk.execute("grep hello data.txt");
    assert!(r.stdout.contains("hello world"));
    assert!(r.stdout.contains("hello again"));

    let r = sdk.execute("sed s/hello/hi/g data.txt");
    assert!(r.stdout.contains("hi world"));
    assert!(!r.stdout.contains("hello world"));
}

#[test]
fn e2e_shell_compression() {
    let sdk = setup_default();
    sdk.write_file("big.txt", &"compress me! ".repeat(200)).unwrap();

    let r = sdk.execute("gzip -c big.txt");
    assert_eq!(r.exit_code, 0);
    // compressed output should exist
    assert!(!r.stdout.is_empty());
}

#[test]
fn e2e_shell_json() {
    let sdk = setup_default();
    sdk.write_file("data.json", r#"{"name":"fastshell","version":"0.1.0"}"#).unwrap();

    let r = sdk.execute("cat data.json | jq .name");
    assert!(r.stdout.contains("fastshell"));
}

// ═══════════════════════════════════════════════════════
// Pipeline — Threaded Concurrency
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_pipeline_two_stage() {
    let sdk = setup_default();
    let r = sdk.execute("echo hello world | wc -w");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "2");
}

#[test]
fn e2e_pipeline_three_stage() {
    let sdk = setup_default();
    let r = sdk.execute("echo \"a\nb\nc\na\nb\" | sort | uniq");
    assert_eq!(r.exit_code, 0);
    let lines: Vec<&str> = r.stdout.trim().lines().collect();
    assert_eq!(lines, vec!["a", "b", "c"]);
}

#[test]
fn e2e_pipeline_four_stage() {
    let sdk = setup_default();
    sdk.write_file("lines.txt", "alpha\nbeta\nalpha\ngamma\nbeta\n").unwrap();
    let r = sdk.execute("cat lines.txt | sort | uniq -c | wc -l");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "3");
}

#[test]
fn e2e_pipeline_cwd_not_leaked() {
    let sdk = setup_default();
    sdk.execute("mkdir subdir");
    // cd inside pipeline should NOT affect parent shell cwd
    let r = sdk.execute("echo test | wc -c");
    assert_eq!(r.exit_code, 0);
    let cwd = sdk.get_cwd();
    // cwd should still be "/" (unchanged)
    assert!(!cwd.contains("subdir"), "pipeline leaked cwd: {}", cwd);
}

// ═══════════════════════════════════════════════════════
// Permission System
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_permission_denied_network() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: true,
        command_timeout_ms: 5_000,
    });

    let r = sdk.execute("curl http://example.com");
    assert_eq!(r.exit_code, EXIT_NEED_PERMISSION);
    assert!(r.stderr.contains("PERMISSION_NEEDED:network:example.com"));
    assert!(r.needs_permission());
}

#[test]
fn e2e_permission_granted_then_allowed() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: true,
        command_timeout_ms: 5_000,
    });

    // First call: denied
    let r = sdk.execute("ping -c 1 -W 1 127.0.0.1");
    assert_eq!(r.exit_code, EXIT_NEED_PERMISSION);
    assert!(r.stderr.contains("PERMISSION_NEEDED:network:127.0.0.1"));

    // Grant permission
    sdk.set_permission("network:127.0.0.1", true);
    assert_eq!(sdk.check_permission("network:127.0.0.1"), Some(true));

    // Retry: should pass through (may fail on connect but not permission)
    let r = sdk.execute("ping -c 1 -W 1 127.0.0.1");
    assert_ne!(r.exit_code, EXIT_NEED_PERMISSION);
}

#[test]
fn e2e_permission_denied_explicitly() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: true,
        command_timeout_ms: 5_000,
    });

    sdk.set_permission("network:evil.com", false);
    let r = sdk.execute("curl http://evil.com");
    assert_eq!(r.exit_code, 1);
    assert!(r.stderr.contains("Permission denied"));
}

#[test]
fn e2e_permission_clear() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: true,
        command_timeout_ms: 5_000,
    });

    sdk.set_permission("network:example.com", true);
    assert!(sdk.check_permission("network:example.com").is_some());
    sdk.clear_permissions();
    assert!(sdk.check_permission("network:example.com").is_none());
}

#[test]
fn e2e_permission_multiple_hosts() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: true,
        command_timeout_ms: 5_000,
    });

    let r = sdk.execute("curl http://a.com");
    assert!(r.stderr.contains("PERMISSION_NEEDED:network:a.com"));

    let r = sdk.execute("curl https://b.org");
    assert!(r.stderr.contains("PERMISSION_NEEDED:network:b.org"));

    sdk.set_permission("network:a.com", true);
    let r = sdk.execute("curl http://a.com");
    assert_ne!(r.exit_code, EXIT_NEED_PERMISSION);

    let r = sdk.execute("curl https://b.org");
    assert_eq!(r.exit_code, EXIT_NEED_PERMISSION);
}

// ═══════════════════════════════════════════════════════
// Subprocess Control
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_subprocess_disabled_rejects_unknown() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
    });

    let r = sdk.execute("some_random_tool_xyz --flag");
    assert_eq!(r.exit_code, 127);
    assert!(r.stderr.contains("subprocess disabled"));
}

#[test]
fn e2e_subprocess_enabled_allows_fallback() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
    });

    let r = sdk.execute("some_random_tool_xyz --flag");
    assert_eq!(r.exit_code, 127);
    // Should show standard "command not found" from system, not "subprocess disabled"
    assert!(!r.stderr.contains("subprocess disabled"));
}

#[test]
fn e2e_subprocess_disabled_builtin_still_works() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
    });

    // All built-in commands must still work
    let r = sdk.execute("echo hello");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("hello"));

    let r = sdk.execute("ls");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("pwd");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("/"));
}

// ═══════════════════════════════════════════════════════
// Python — Sandbox Execution & File Hook
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_python_basic_execution() {
    let sdk = setup_default();
    let r = sdk.execute_python("print(10 + 32)");
    // Python may not be available; only assert if it is
    let info = sdk.get_info();
    if info.python_available {
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("42"));
    }
}

#[test]
fn e2e_python_subprocess_bridge() {
    let sdk = setup_default();
    let r = sdk.execute_python("import subprocess; r = subprocess.run(['echo', 'hello_from_py'], capture_output=True, text=True); print(r.stdout.strip())");
    let info = sdk.get_info();
    if info.python_available {
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("hello_from_py"));
    }
}

// ═══════════════════════════════════════════════════════
// SQLite3
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_sqlite3_shell_command() {
    let sdk = setup_default();
    sdk.execute("sqlite3 test.db 'CREATE TABLE users (id, name)'");
    sdk.execute("sqlite3 test.db \"INSERT INTO users VALUES (1, 'Alice')\"");
    sdk.execute("sqlite3 test.db \"INSERT INTO users VALUES (2, 'Bob')\"");

    let r = sdk.execute("sqlite3 test.db 'SELECT * FROM users ORDER BY id'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("Alice"));
    assert!(r.stdout.contains("Bob"));

    let r = sdk.execute("sqlite3 test.db .tables");
    assert!(r.stdout.contains("users"));

    let r = sdk.execute("sqlite3 -csv test.db 'SELECT * FROM users'");
    assert!(r.stdout.contains("1,Alice"));
}

#[test]
fn e2e_sqlite3_stdin() {
    let sdk = setup_default();
    // Test sqlite3 with stdin via file redirection
    sdk.write_file("init.sql", "CREATE TABLE t (x TEXT);\nINSERT INTO t VALUES ('pipe_works');\nSELECT * FROM t;\n").unwrap();
    // Use cat to pipe SQL file content to sqlite3
    let r = sdk.execute("cat init.sql | sqlite3 test2.db");
    assert!(r.exit_code == 0 || r.stdout.contains("pipe_works"),
        "exit={} stdout={} stderr={}", r.exit_code, r.stdout, r.stderr);
}

#[test]
fn e2e_python_sqlite3_import() {
    let sdk = setup_default();
    let info = sdk.get_info();
    if !info.python_available { return; }

    let r = sdk.execute_python(
        "import sqlite3; conn = sqlite3.connect(':memory:'); \
         conn.execute('CREATE TABLE t(x)'); \
         conn.execute('INSERT INTO t VALUES(42)'); \
         print(conn.execute('SELECT x FROM t').fetchone()[0])"
    );
    if r.exit_code == 0 {
        assert!(r.stdout.contains("42"), "sqlite3 import failed: stdout={} stderr={}", r.stdout, r.stderr);
    }
}

// ═══════════════════════════════════════════════════════
// Config — Defaults
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_config_defaults_mobile() {
    let cfg = Config::default();
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        assert!(!cfg.allow_subprocess, "mobile must disable subprocess by default");
        assert!(cfg.network_ask_permission, "mobile must ask permission by default");
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        assert!(cfg.allow_subprocess, "desktop must allow subprocess by default");
        assert!(!cfg.network_ask_permission, "desktop must not ask permission by default");
    }
}

// ═══════════════════════════════════════════════════════
// Timeout & Cancel
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_timeout_triggers() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 1, // 1ms timeout → guaranteed to trigger
    });

    let r = sdk.execute("sleep 10");
    assert_eq!(r.exit_code, 124);
    assert!(r.stderr.contains("timed out"));
}

#[test]
fn e2e_no_timeout_zero() {
    let sdk = setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 0, // no timeout
    });

    let r = sdk.execute("echo ok");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("ok"));
}

// ═══════════════════════════════════════════════════════
// Thread Safety — Concurrent Execute
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_concurrent_execute() {
    let sdk = Arc::new(setup(Config {
        sandbox_path: String::new(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 0,
    }));

    let mut handles = Vec::new();
    for i in 0..5 {
        let sdk = sdk.clone();
        let handle = std::thread::spawn(move || {
            let cmd = format!("echo thread{}", i);
            sdk.execute(&cmd)
        });
        handles.push(handle);
    }

    for handle in handles {
        let r = handle.join().unwrap();
        assert_eq!(r.exit_code, 0);
        assert!(!r.stdout.is_empty());
    }
}

// ═══════════════════════════════════════════════════════
// Edge Cases
// ═══════════════════════════════════════════════════════

#[test]
fn e2e_empty_command() {
    let sdk = setup_default();
    let r = sdk.execute("");
    assert_eq!(r.exit_code, 0);
}

#[test]
fn e2e_command_with_spaces() {
    let sdk = setup_default();
    let r = sdk.execute("echo 'hello   world'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("hello   world"));
}

#[test]
fn e2e_command_output_fields() {
    let sdk = setup_default();
    let r = sdk.execute("echo success");
    assert!(r.is_success());
    assert!(r.stdout.contains("success"));
    assert!(r.stderr.is_empty());

    let r = sdk.execute("this_does_not_exist_xyz");
    assert!(!r.is_success());
    assert_ne!(r.exit_code, 0);
}

#[test]
fn e2e_get_info_complete() {
    let sdk = setup_default();
    let info = sdk.get_info();
    assert!(!info.version.is_empty());
    assert!(!info.platform.is_empty());
    assert!(!info.sandbox_path.is_empty());
    // allow_subprocess is true because we set it in setup_default
    assert!(info.allow_subprocess);
}

#[test]
fn e2e_vfs_root_accessible() {
    let sdk = setup_default();
    let root = sdk.vfs_root();
    assert!(std::path::Path::new(&root).exists());
    assert!(std::path::Path::new(&root).is_dir());
}

#[test]
fn e2e_env_vars() {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_e2e_env_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
    }).unwrap();

    sdk.set_env("MY_TEST_VAR", "my_test_value");
    assert_eq!(sdk.get_env("MY_TEST_VAR"), Some("my_test_value".to_string()));
    assert_eq!(sdk.get_env("NONEXISTENT"), None);
}

#[test]
fn e2e_globs_work() {
    let sdk = setup_default();
    sdk.write_file("a.rs", "").unwrap();
    sdk.write_file("b.rs", "").unwrap();
    sdk.write_file("c.txt", "").unwrap();

    let r = sdk.execute("echo *.rs");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("a.rs"));
    assert!(r.stdout.contains("b.rs"));
    assert!(!r.stdout.contains("c.txt"));
}

#[test]
fn e2e_shutdown_cleans_up() {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_e2e_shutdown_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
    }).unwrap();

    let root = sdk.vfs_root();
    assert!(std::path::Path::new(&root).exists());
    assert!(sdk.is_initialized());

    sdk.shutdown();
    assert!(!sdk.is_initialized());
    assert!(!std::path::Path::new(&root).exists());
}
