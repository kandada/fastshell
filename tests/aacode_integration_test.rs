// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_aacode_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
    })
    .unwrap();
    sdk
}

fn python_available(sdk: &Fastshell) -> bool {
    let rt = sdk.runtime_ref();
    let rt = rt.lock().unwrap();
    rt.python_available()
}

// ═══════════════════════════════════════════════════════
// aacode atomic_tools.py patterns
// These are the exact shell call patterns aacode uses
// via asyncio.create_subprocess_shell()
// ═══════════════════════════════════════════════════════

#[test]
fn aacode_pattern_ls_variants() {
    let sdk = setup();
    sdk.execute("mkdir -p project/src");
    sdk.write_file("project/src/main.py", "print('hello')")
        .unwrap();

    let cmds = vec!["ls", "ls -la", "ls -R", "ls project", "ls project/src"];
    for c in cmds {
        let r = sdk.execute(c);
        assert_eq!(r.exit_code, 0, "aacode '{}' failed: {}", c, r.stderr);
    }
}

#[test]
fn aacode_pattern_cat_file() {
    let sdk = setup();
    sdk.write_file(
        "setup.py",
        "from setuptools import setup\nsetup(name='test')\n",
    )
    .unwrap();
    let r = sdk.execute("cat setup.py");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("setuptools"));
}

#[test]
fn aacode_pattern_grep_search() {
    let sdk = setup();
    sdk.write_file(
        "requirements.txt",
        "openai>=1.0\nanthropic>=0.25\naiohttp\n",
    )
    .unwrap();

    let r = sdk.execute("grep openai requirements.txt");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("openai"));

    let r = sdk.execute("grep -n aiohttp requirements.txt");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("aiohttp"));

    let r = sdk.execute("grep -c openai requirements.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "1");
}

#[test]
fn aacode_pattern_find_files() {
    let sdk = setup();
    sdk.execute("mkdir -p project/tests");
    sdk.write_file("project/main.py", "print(1)").unwrap();
    sdk.write_file("project/tests/test_main.py", "def test(): pass")
        .unwrap();
    sdk.write_file("project/README.md", "# Project").unwrap();

    let r = sdk.execute("find project -name '*.py'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("main.py"));
    assert!(r.stdout.contains("test_main.py"));

    let r = sdk.execute("find project -type f");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("README.md"));
}

#[test]
fn aacode_pattern_mkdir_rm() {
    let sdk = setup();

    let r = sdk.execute("mkdir -p a/b/c");
    assert_eq!(r.exit_code, 0);

    sdk.write_file("a/b/c/test.txt", "data").unwrap();
    assert!(sdk.exists("a/b/c/test.txt"));

    let r = sdk.execute("rm -rf a");
    assert_eq!(r.exit_code, 0);
    assert!(!sdk.exists("a"));
}

#[test]
fn aacode_pattern_cp_mv() {
    let sdk = setup();
    sdk.write_file("original.txt", "original content").unwrap();

    let r = sdk.execute("cp original.txt copied.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(sdk.read_file("copied.txt").unwrap(), "original content");

    let r = sdk.execute("mv copied.txt renamed.txt");
    assert_eq!(r.exit_code, 0);
    assert!(!sdk.exists("copied.txt"));
    assert_eq!(sdk.read_file("renamed.txt").unwrap(), "original content");
}

#[test]
fn aacode_pattern_wc_head_tail() {
    let sdk = setup();
    let lines: Vec<String> = (1..=100).map(|i| format!("line {}", i)).collect();
    sdk.write_file("hundred.txt", &lines.join("\n")).unwrap();

    let r = sdk.execute("wc -l hundred.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "100 hundred.txt");

    let r = sdk.execute("head -n 3 hundred.txt");
    assert_eq!(r.exit_code, 0);
    let head_lines: Vec<&str> = r.stdout.trim().split('\n').collect();
    assert_eq!(head_lines.len(), 3);

    let r = sdk.execute("tail -n 3 hundred.txt");
    assert_eq!(r.exit_code, 0);
    let tail_lines: Vec<&str> = r.stdout.trim().split('\n').collect();
    assert_eq!(tail_lines.len(), 3);
}

#[test]
fn aacode_pattern_sed_replace() {
    let sdk = setup();
    sdk.write_file(
        "config.py",
        "DEBUG = True\nPORT = 8080\nHOST = 'localhost'\n",
    )
    .unwrap();

    let r = sdk.execute("sed 's/True/False/' config.py");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("DEBUG = False"));

    let r = sdk.execute("sed -i 's/8080/9090/' config.py");
    assert_eq!(r.exit_code, 0);
    let content = sdk.read_file("config.py").unwrap();
    assert!(content.contains("9090"));
    assert!(!content.contains("8080"));
}

#[test]
fn aacode_pattern_awk_processing() {
    let sdk = setup();
    sdk.write_file(
        "data.csv",
        "name,age,city\nAlice,30,NYC\nBob,25,LA\nCharlie,35,SF\n",
    )
    .unwrap();

    let r = sdk.execute("awk -F, '{print $1}' data.csv");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("name"));
    assert!(r.stdout.contains("Alice"));

    let r = sdk.execute("awk -F, 'NR>1 {print $2}' data.csv");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("30"));
}

#[test]
fn aacode_pattern_sort_uniq() {
    let sdk = setup();
    sdk.write_file("items.txt", "apple\nbanana\nApple\napple\ncherry\nBanana\n")
        .unwrap();

    let r = sdk.execute("sort items.txt | uniq");
    assert_eq!(r.exit_code, 0);
    let lines: Vec<&str> = r.stdout.trim().split('\n').collect();
    assert!(lines.contains(&"apple"));

    let r = sdk.execute("sort items.txt | uniq -c");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("apple"));
}

#[test]
fn aacode_pattern_diff_files() {
    let sdk = setup();
    sdk.write_file("v1.py", "x = 1\ny = 2\n").unwrap();
    sdk.write_file("v2.py", "x = 1\ny = 3\nz = 4\n").unwrap();

    let r = sdk.execute("diff v1.py v2.py");
    assert_eq!(r.exit_code, 1);
    assert!(!r.stdout.is_empty());
}

#[test]
fn aacode_pattern_chmod_executable() {
    let sdk = setup();
    sdk.write_file("run.sh", "#!/bin/bash\necho hello\n")
        .unwrap();

    let r = sdk.execute("chmod +x run.sh");
    assert_eq!(r.exit_code, 0);
}

#[test]
fn aacode_pattern_complex_pipeline() {
    let sdk = setup();
    let lines: Vec<String> = (1..=50)
        .map(|i| format!("log: event_{} status=ok", i % 3))
        .collect();
    sdk.write_file("app.log", &lines.join("\n")).unwrap();

    let r = sdk.execute("cat app.log | grep 'status=ok' | wc -l");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("grep event_1 app.log | wc -l");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("cat app.log | sed 's/status=ok/OK/' | grep OK | head -n 5 | wc -l");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "5");
}

#[test]
fn aacode_pattern_jq_json() {
    let sdk = setup();
    sdk.write_file(
        "response.json",
        r#"{"key": "Hello", "nested": {"value": 42}}"#,
    )
    .unwrap();

    let r = sdk.execute("cat response.json | jq .key");
    assert_eq!(r.exit_code, 0, "jq failed: stderr={}", r.stderr);
    assert!(r.stdout.contains("Hello"), "jq output was: {}", r.stdout);

    let r = sdk.execute("cat response.json | jq '.nested'");
    assert_eq!(r.exit_code, 0, "jq .nested failed: stderr={}", r.stderr);
    assert!(
        r.stdout.contains("42"),
        "jq .nested output was: {}",
        r.stdout
    );
}

#[test]
fn aacode_pattern_base64_encode() {
    let sdk = setup();
    let r = sdk.execute("echo -n 'test_api_key_12345' | base64");
    assert_eq!(r.exit_code, 0);
    let encoded = r.stdout.trim().to_string();
    assert!(!encoded.is_empty());
}

#[test]
fn aacode_pattern_curl_fetch() {
    let sdk = setup();
    sdk.set_permission("network:example.com", true);

    let r = sdk.execute("curl -s http://example.com");
    assert_eq!(r.exit_code, 0);
}

#[test]
fn aacode_pattern_python_subprocess() {
    let sdk = setup();
    if !python_available(&sdk) {
        return;
    }

    let code = r#"
import subprocess
r = subprocess.run('ls -la', shell=True, capture_output=True, text=True)
print('FILES:' + str(len(r.stdout.splitlines())))
r2 = subprocess.run('echo hello | wc -c', shell=True, capture_output=True, text=True)
print('WC:' + r2.stdout.strip())
"#;
    let r = sdk.execute_python(code);
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("FILES:"));
    assert!(r.stdout.contains("WC:"));
}

#[test]
fn aacode_pattern_python_async_subprocess() {
    let sdk = setup();
    if !python_available(&sdk) {
        return;
    }

    let code = r#"
import asyncio
async def main():
    proc = await asyncio.create_subprocess_shell(
        'echo "async_output"',
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await proc.communicate()
    print(stdout.decode().strip())
asyncio.run(main())
"#;
    let r = sdk.execute_python(code);
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("async_output"));
}

#[test]
fn aacode_pattern_python_file_workflow() {
    let sdk = setup();
    if !python_available(&sdk) {
        return;
    }

    let code = r#"
import subprocess
subprocess.run('mkdir work', shell=True)
subprocess.run('cd work && echo "line1\nline2\nline3" > data.txt', shell=True)
r = subprocess.run('cd work && cat data.txt | wc -l', shell=True, capture_output=True, text=True)
print('LINES:' + r.stdout.strip())
r = subprocess.run('cd work && grep line data.txt', shell=True, capture_output=True, text=True)
print('GREP:' + str(len(r.stdout.splitlines())))
"#;
    let r = sdk.execute_python(code);
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("LINES:3"));
    assert!(r.stdout.contains("GREP:3"));
}

#[test]
fn aacode_pattern_python_env_and_config() {
    let sdk = setup();
    if !python_available(&sdk) {
        return;
    }

    let code = r#"
import os, subprocess
os.environ['MY_TEST_VAR'] = 'test_value'
r = subprocess.run('echo $MY_TEST_VAR', shell=True, capture_output=True, text=True)
print('ENV:' + r.stdout.strip())
"#;
    let r = sdk.execute_python(code);
    assert_eq!(r.exit_code, 0);
}

#[test]
fn aacode_pattern_echo_and_redirect() {
    let sdk = setup();

    sdk.write_file("agent_init.py", "from main import MainAgent\n")
        .unwrap();
    let content = sdk.read_file("agent_init.py").unwrap();
    assert!(content.contains("from main import MainAgent"));
}

#[test]
fn aacode_pattern_touch_date() {
    let sdk = setup();

    let r = sdk.execute("touch placeholder.txt");
    assert_eq!(r.exit_code, 0);
    assert!(sdk.exists("placeholder.txt"));

    let r = sdk.execute("date '+%Y-%m-%d'");
    assert_eq!(r.exit_code, 0);
    assert!(!r.stdout.trim().is_empty());
}

#[test]
fn aacode_pattern_tar_create_list() {
    let sdk = setup();
    sdk.write_file("a.txt", "a").unwrap();
    sdk.write_file("b.txt", "b").unwrap();

    let r = sdk.execute("tar -cf archive.tar a.txt b.txt");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("tar -tf archive.tar");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("a.txt"));
    assert!(r.stdout.contains("b.txt"));
}

#[test]
fn aacode_pattern_test_conditions() {
    let sdk = setup();
    sdk.write_file("exists.txt", "content").unwrap();

    let r = sdk.execute("test -f exists.txt");
    assert_eq!(r.exit_code, 0, "test -f exists.txt should succeed");

    let r = sdk.execute("test -f nonexists.txt");
    assert_eq!(r.exit_code, 1, "test -f nonexists.txt should fail");

    let r = sdk.execute("test -d .");
    assert_eq!(r.exit_code, 0, "test -d . should succeed");
}

#[test]
fn aacode_pattern_env_path_resolution() {
    let sdk = setup();
    sdk.execute("mkdir -p project");
    sdk.execute("cd project");
    let r = sdk.execute("pwd");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("project"), "pwd output was: {}", r.stdout);
}

#[test]
fn aacode_pattern_python_pip_simulate() {
    let sdk = setup();
    if !python_available(&sdk) {
        return;
    }

    let code = r#"
import sys
print('Python', sys.version_info[:2])
import subprocess
r = subprocess.run('which python3', shell=True, capture_output=True, text=True)
print('WHICH:' + r.stdout.strip())
"#;
    let r = sdk.execute_python(code);
    assert_eq!(r.exit_code, 0);
}

#[test]
fn aacode_pattern_bulk_git_like_operations() {
    let sdk = setup();
    sdk.execute("mkdir -p repo/src");
    sdk.write_file("repo/src/main.py", "def main(): pass\n")
        .unwrap();
    sdk.write_file("repo/README.md", "# Repo\n").unwrap();

    let r = sdk.execute("ls repo/src");
    assert_eq!(r.exit_code, 0, "ls failed: stderr={}", r.stderr);
    assert!(r.stdout.contains("main.py"), "stdout was: {}", r.stdout);

    let r = sdk.execute("ls repo");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("README.md"), "stdout was: {}", r.stdout);

    let r = sdk.execute("cat repo/src/main.py");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("def main"));

    let r = sdk.execute("grep def repo/src/main.py");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("def main"));
}

#[test]
fn aacode_pattern_large_codebase_scenario() {
    let sdk = setup();

    let project_structure = [
        "project/__init__.py",
        "project/main.py",
        "project/core/__init__.py",
        "project/core/agent.py",
        "project/core/react_loop.py",
        "project/utils/__init__.py",
        "project/utils/helpers.py",
        "project/tests/__init__.py",
        "project/tests/test_agent.py",
        "project/config.yaml",
    ];

    for path in &project_structure {
        let dir = std::path::Path::new(path).parent().unwrap();
        let dir_str = dir.to_string_lossy();
        if !dir_str.is_empty() {
            sdk.execute(&format!("mkdir -p {}", dir_str));
        }
        sdk.write_file(path, &format!("# {}", path)).unwrap();
    }

    let r = sdk.execute("find project -name '*.py' | wc -l");
    assert_eq!(r.exit_code, 0);
    let count: i32 = r.stdout.trim().parse().unwrap_or(0);
    assert!(
        count >= 8,
        "expected >=8 .py files, got {}: stdout={}",
        count,
        r.stdout
    );

    let r = sdk.execute("find project -type d | wc -l");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("grep -r 'def' project --include='*.py' | wc -l");
    assert!(r.exit_code == 0 || r.exit_code == 1);

    let r = sdk.execute("find project | head -20");
    assert!(r.exit_code == 0 || r.exit_code == 1 || r.exit_code == 2);
}

#[test]
fn aacode_pattern_xargs_exec_pattern() {
    let sdk = setup();
    sdk.write_file("files.txt", "a.txt\nb.txt\nc.txt\n")
        .unwrap();

    let r = sdk.execute("cat files.txt | xargs touch");
    assert_eq!(r.exit_code, 0);
    assert!(sdk.exists("a.txt"));
    assert!(sdk.exists("b.txt"));
    assert!(sdk.exists("c.txt"));
}
