use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use fastshell::sdk::Fastshell;
use fastshell::sdk::types::Config;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_fuzz_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
    }).unwrap();
    sdk.execute("mkdir -p sub/nested/deep");
    sdk.write_file("test.txt", "hello world\nline 2\nline 3\n").unwrap();
    sdk.write_file("nums.txt", "5\n3\n8\n1\n9\n2\n7\n4\n6\n").unwrap();
    sdk.write_file("words.txt", "apple\nBanana\nAPPLE\ncherry\ndate\n").unwrap();
    sdk.write_file("json.txt", r#"{"key": "value", "nested": {"x": 1}}"#).unwrap();
    sdk.write_file("multi.txt", "a b c\nd e f\ng h i\n").unwrap();
    sdk
}

// ═══════════════════════════════════════════════════════
// Fuzz — grep with special characters and edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_grep_special_chars() {
    let sdk = setup();
    sdk.write_file("special.txt", "dot.star**\nquestion?mark\n[].brackets\n^dollar\n|pipe\n(open\n)close\n\\backslash\n+plus\n").unwrap();

    let patterns = vec![".", "*", "?", "[", "]", "^", "$", "|", "(", ")", "\\", "+", "dot\\.star"];
    for p in patterns {
        let r = sdk.execute(&format!("grep '{}' special.txt", p.replace("'", "'\\''")));
        assert!(r.exit_code == 0 || r.exit_code == 1, "grep '{}' crashed with code {}: {}", p, r.exit_code, r.stderr);
    }
}

#[test]
fn fuzz_grep_empty_input() {
    let sdk = setup();
    sdk.write_file("empty.txt", "").unwrap();
    let r = sdk.execute("grep anything empty.txt");
    assert!(r.exit_code == 1);
}

#[test]
fn fuzz_grep_unicode() {
    let sdk = setup();
    sdk.write_file("unicode.txt", "café\nnaïve\nüber\n汉字\n日本語\nemoji 😀\n").unwrap();

    let r1 = sdk.execute("grep café unicode.txt");
    assert!(r1.exit_code == 0 || r1.exit_code == 1);

    let r2 = sdk.execute("grep 汉字 unicode.txt");
    assert!(r2.exit_code == 0 || r2.exit_code == 1);
}

#[test]
fn fuzz_grep_very_long_line() {
    let sdk = setup();
    let long = "a".repeat(100_000);
    sdk.write_file("long.txt", &long).unwrap();
    let r = sdk.execute("grep aaaaa long.txt");
    assert!(!r.stderr.contains("panic"), "grep on long line panicked");
    assert!(r.exit_code == 0 || r.exit_code == 1);
}

// ═══════════════════════════════════════════════════════
// Fuzz — sed edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_sed_special_patterns() {
    let sdk = setup();

    let cases = vec![
        ("s/a/b/", "test.txt"),
        ("s/./X/", "test.txt"),
        ("s/^/START:/", "test.txt"),
        ("s/$/:END/", "test.txt"),
        ("s/hello/HELLO/", "test.txt"),
        ("s/nonexistent/replaced/", "test.txt"),
        ("s/[a-z]/X/g", "test.txt"),
        ("s/\\s/_/g", "test.txt"),
    ];

    for (expr, file) in cases {
        let r = sdk.execute(&format!("sed '{}' {}", expr, file));
        assert!(r.exit_code == 0, "sed '{}' failed with code {}: {}", expr, r.exit_code, r.stderr);
    }
}

#[test]
fn fuzz_sed_in_place() {
    let sdk = setup();
    sdk.execute("sed -i 's/hello/hi/' test.txt");
    let content = sdk.read_file("test.txt").unwrap();
    assert!(!content.contains("hello"));
    assert!(content.contains("hi"));
}

// ═══════════════════════════════════════════════════════
// Fuzz — awk edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_awk_basic_patterns() {
    let sdk = setup();

    let r = sdk.execute("awk '{print $1}' multi.txt");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("awk '{print NR, NF}' multi.txt");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("awk 'BEGIN {print \"start\"} {print $0} END {print \"end\"}' test.txt");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("awk '/line/ {print}' test.txt");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("awk '{if(NR==1) print}' test.txt");
    assert_eq!(r.exit_code, 0);
}

// ═══════════════════════════════════════════════════════
// Fuzz — sort edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_sort_variants() {
    let sdk = setup();

    let cases = vec![
        "sort -n nums.txt",
        "sort -nr nums.txt",
        "sort -u words.txt",
        "sort -r words.txt",
        "sort -f words.txt",
    ];

    for cmd in cases {
        let r = sdk.execute(cmd);
        assert_eq!(r.exit_code, 0, "sort cmd '{}' failed: stderr={}", cmd, r.stderr);
    }
}

#[test]
fn fuzz_sort_empty_file() {
    let sdk = setup();
    sdk.write_file("empty_for_sort.txt", "").unwrap();
    let r = sdk.execute("sort empty_for_sort.txt");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.is_empty());
}

// ═══════════════════════════════════════════════════════
// Fuzz — find edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_find_variants() {
    let sdk = setup();

    let cases = vec![
        "find . -name '*.txt'",
        "find . -type f",
        "find . -type d",
        "find sub -name '*.rst'",
    ];

    for cmd in cases {
        let r = sdk.execute(cmd);
        assert_eq!(r.exit_code, 0, "find cmd '{}' failed: stderr={}", cmd, r.stderr);
    }
}

// ═══════════════════════════════════════════════════════
// Fuzz — path escape attempts
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_path_escape_variants() {
    let sdk = setup();

    let escape_attempts = vec![
        "../../../etc/passwd",
        "sub/../../etc/passwd",
        "./../../././etc/passwd",
    ];

    for path in escape_attempts {
        let r = sdk.read_file(path);
        assert!(r.is_err(), "Path '{}' should be blocked but got: {:?}", path, r);

        let r = sdk.write_file(path, "escape");
        assert!(r.is_err(), "Write to '{}' should be blocked", path);
    }
}

// ═══════════════════════════════════════════════════════
// Fuzz — JSON (jq) edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_jq_basic() {
    let sdk = setup();
    let r = sdk.execute("cat json.txt | jq .key");
    assert_eq!(r.exit_code, 0, "jq .key failed: stderr={}", r.stderr);

    let r = sdk.execute("cat json.txt | jq .nested");
    assert_eq!(r.exit_code, 0, "jq .nested failed: stderr={}", r.stderr);

    let r = sdk.execute("cat json.txt | jq .");
    assert_eq!(r.exit_code, 0, "jq . failed: stderr={}", r.stderr);
}

// ═══════════════════════════════════════════════════════
// Fuzz — base64 edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_base64_roundtrip() {
    let sdk = setup();

    let inputs = vec!["hello", "", "a", "ab", "abc", "hello world!", "test\nmulti\nline"];

    for input in inputs {
        let encoded = sdk.execute(&format!("echo -n '{}' | base64", input));
        if encoded.exit_code != 0 { continue; }
        let decoded = sdk.execute(&format!("echo -n '{}' | base64 -d", encoded.stdout.trim()));
        if decoded.exit_code != 0 { continue; }
        assert_eq!(decoded.stdout.trim(), input, "base64 roundtrip failed for '{}'", input);
    }
}

// ═══════════════════════════════════════════════════════
// Fuzz — pipeline with empty stages
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_pipeline_edge_cases() {
    let sdk = setup();

    let r = sdk.execute("echo hello | grep nonexistent");
    assert_eq!(r.exit_code, 1);

    let r = sdk.execute("echo a | cat | cat | cat");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "a");
}

// ═══════════════════════════════════════════════════════
// Fuzz — sed with capture groups and addresses
// ═══════════════════════════════════════════════════════

#[test]
fn fuzz_sed_capture_groups() {
    let sdk = setup();
    sdk.write_file("captures.txt", "first second\napple banana\n").unwrap();

    let r = sdk.execute("sed 's/\\(\\w\\+\\) \\(\\w\\+\\)/\\2 \\1/' captures.txt");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("sed -n '2p' captures.txt");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("apple banana"));
}

// ═══════════════════════════════════════════════════════
// Python Sandbox Bypass Tests
// ═══════════════════════════════════════════════════════

#[test]
fn sandbox_python_os_popen_bypass() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let code = r#"
import os
try:
    p = os.popen('echo "popen_works"')
    out = p.read()
    print(out.strip())
except Exception as e:
    print('ERROR:' + str(e))
"#;
    let r = sdk.execute_python(code);
    eprintln!("os.popen result: stdout={:?} stderr={:?}", r.stdout, r.stderr);
    // os.popen should either be hooked or fail gracefully
    assert!(r.exit_code == 0 || r.stderr.contains("ERROR") || r.stderr.contains("AttributeError"),
        "os.popen should be handled: code={} stdout={:?} stderr={:?}", r.exit_code, r.stdout, r.stderr);
}

#[test]
fn sandbox_python_ctypes_bypass() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let code = r#"
try:
    import ctypes
    libc = ctypes.CDLL(None)
    print('ctypes_loaded')
except Exception as e:
    print('ERROR:' + str(e))
"#;
    let r = sdk.execute_python(code);
    assert!(r.exit_code == 0,
        "ctypes should at least not panic: code={} stdout={:?} stderr={:?}", r.exit_code, r.stdout, r.stderr);
}

#[test]
fn sandbox_python_exec_bypass() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let code = r#"
code = "print('from_exec')"
exec(code)
"#;
    let r = sdk.execute_python(code);
    assert!(r.exit_code == 0);
    assert!(r.stdout.contains("from_exec"));
}

#[test]
fn sandbox_python_eval_bypass() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let r = sdk.execute_python("print(eval('1+2'))");
    assert!(r.exit_code == 0);
    assert!(r.stdout.contains("3"));
}

#[test]
fn sandbox_python_importlib_bypass() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let code = r#"
import importlib
try:
    os = importlib.import_module('os')
    print(os.name)
except Exception as e:
    print('ERROR:' + str(e))
"#;
    let r = sdk.execute_python(code);
    assert!(r.exit_code == 0);
}

#[test]
fn sandbox_python_subprocess_via_shlex() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let code = r#"
import subprocess
r = subprocess.run(['echo', 'shlex_test'], capture_output=True, text=True)
print(r.stdout.strip())
"#;
    let r = sdk.execute_python(code);
    assert!(r.exit_code == 0);
    // list form of subprocess.run should also work
    assert!(r.stdout.contains("shlex_test") || r.stderr.contains("ERROR"),
        "subprocess.run with list args: stdout={:?} stderr={:?}", r.stdout, r.stderr);
}

#[test]
fn sandbox_python_threading_subprocess() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let code = r#"
import threading, subprocess

def worker(name):
    r = subprocess.run(f'echo thread_{name}', shell=True, capture_output=True, text=True)
    print(f'{name}:{r.stdout.strip()}')

threads = []
for i in range(3):
    t = threading.Thread(target=worker, args=(i,))
    threads.append(t)
    t.start()
for t in threads:
    t.join()
print('done')
"#;
    let r = sdk.execute_python(code);
    assert!(r.exit_code == 0);
}

#[test]
fn sandbox_vfs_isolation_python() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let code = r#"
import os
try:
    with open('../../../etc/passwd', 'r') as f:
        print('ESCAPED:' + f.read())
except Exception as e:
    print('BLOCKED:' + type(e).__name__)
"#;
    let r = sdk.execute_python(code);
    assert!(r.exit_code == 0);
    assert!(!r.stdout.contains("ESCAPED"), "VFS path escape should be blocked in Python: {}", r.stdout);
}

#[test]
fn sandbox_python_nested_subprocess() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let code = r#"
import subprocess
r = subprocess.run('echo level1 | grep level1', shell=True, capture_output=True, text=True)
print('L1:' + r.stdout.strip())
r2 = subprocess.run(f'echo {r.stdout.strip()}_extended', shell=True, capture_output=True, text=True)
print('L2:' + r2.stdout.strip())
"#;
    let r = sdk.execute_python(code);
    assert!(r.exit_code == 0);
}

#[test]
fn sandbox_shutdown_during_python_execution() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } { return; }

    let mut sdk = sdk;
    let r = sdk.execute_python("print('before_shutdown')");
    assert_eq!(r.exit_code, 0);

    sdk.shutdown();
    assert!(!sdk.is_initialized());
}
