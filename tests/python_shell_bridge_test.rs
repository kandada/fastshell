// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_pybridge_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 0,
    })
    .unwrap();
    sdk.write_file("hello.py", "print('hello from python')")
        .unwrap();
    sdk
}

#[test]
fn test_python_subprocess_run() {
    let sdk = setup();
    let r = sdk.execute_python("import subprocess; r = subprocess.run('echo hello', shell=True, capture_output=True, text=True); print(r.stdout.strip())");
    assert!(r.is_success());
    assert!(r.stdout.contains("hello"));
}

#[test]
fn test_python_subprocess_pipe() {
    let sdk = setup();
    let r = sdk.execute_python(
        "import subprocess; r = subprocess.run('echo hello | wc -w', shell=True, capture_output=True, text=True); print(r.stdout.strip())"
    );
    assert!(r.is_success());
    assert_eq!(r.stdout.trim(), "1");
}

#[test]
fn test_python_create_subprocess_shell() {
    let sdk = setup();
    let code = r#"
import asyncio
async def main():
    proc = await asyncio.create_subprocess_shell(
        'echo async_test',
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await proc.communicate()
    print(stdout.decode().strip())
asyncio.run(main())
"#;
    let r = sdk.execute_python(code);
    assert!(r.is_success());
    assert!(r.stdout.contains("async_test"));
}

#[test]
fn test_python_file_operations() {
    let sdk = setup();
    let code = r#"
import subprocess
subprocess.run('mkdir work', shell=True)
subprocess.run('cd work && touch test.txt', shell=True)
subprocess.run('cd work && echo hello world > test.txt', shell=True)
r = subprocess.run('cd work && cat test.txt', shell=True, capture_output=True, text=True)
print(r.stdout.strip())
"#;
    let r = sdk.execute_python(code);
    assert!(r.is_success());
    assert!(r.stdout.contains("hello world"));
}

#[test]
fn test_python_exit_code() {
    let sdk = setup();
    let code = r#"
import subprocess
r = subprocess.run('ls /nonexistent', shell=True, capture_output=True, text=True)
print('EXIT:' + str(r.returncode))
"#;
    let r = sdk.execute_python(code);
    assert!(r.is_success());
    assert!(r.stdout.contains("EXIT:"));
    assert!(!r.stdout.contains("EXIT:0"));
}

#[test]
fn test_python_script_file() {
    let sdk = setup();
    sdk.write_file("test_script.py", "import subprocess; r = subprocess.run('ls', shell=True, capture_output=True, text=True); print(r.stdout.strip())").unwrap();
    let r = sdk.execute_python("exec(open('test_script.py').read())");
    assert!(r.is_success());
}

#[test]
fn test_python_os_system() {
    let sdk = setup();
    let code = r#"
import os
ret = os.system('echo from_os_system')
print('RET:' + str(ret))
"#;
    let r = sdk.execute_python(code);
    assert!(r.is_success());
    assert!(r.stdout.contains("from_os_system"));
    assert!(r.stdout.contains("RET:0"));
}

#[test]
fn test_python_site_packages_import() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } {
        return;
    }

    let root = sdk.vfs_root();
    let site_path = std::path::Path::new(&root).join("python/site-packages");
    std::fs::create_dir_all(&site_path).unwrap();
    let test_mod = site_path.join("myprebuilt.py");
    std::fs::write(&test_mod, "def greet(): return 'bundled_hello'\n").unwrap();

    // Diagnostic: print what sys.path looks like
    let diag = sdk.execute_python("import sys, os; print('ROOT', repr(os.environ.get('FASTSHELL_ROOT'))); print('PATH', sys.path)");
    eprintln!("DIAG: stdout={:?} stderr={:?}", diag.stdout, diag.stderr);

    let code = r#"
import myprebuilt
print(myprebuilt.greet())
"#;
    let r = sdk.execute_python(code);
    assert!(
        r.is_success(),
        "import myprebuilt failed: stdout={:?} stderr={:?}",
        r.stdout,
        r.stderr
    );
    assert!(
        r.stdout.contains("bundled_hello"),
        "stdout={:?} stderr={:?}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn test_python_sandbox_root_import() {
    let sdk = setup();
    if {
        let rt = sdk.runtime_ref();
        let rt = rt.lock().unwrap();
        !rt.python_available()
    } {
        return;
    }

    // Simulate aacode code bundled at sandbox root level
    let root = sdk.vfs_root();
    let aacode_dir = std::path::Path::new(&root).join("aacode");
    std::fs::create_dir_all(&aacode_dir).unwrap();
    let init_file = aacode_dir.join("__init__.py");
    std::fs::write(&init_file, "").unwrap();
    let main_file = aacode_dir.join("main.py");
    std::fs::write(&main_file, "def version(): return '1.0.0'\n").unwrap();

    let code = r#"
from aacode.main import version
print(version())
"#;
    let r = sdk.execute_python(code);
    assert!(
        r.is_success(),
        "import aacode.main failed: stdout={:?} stderr={:?}",
        r.stdout,
        r.stderr
    );
    assert!(
        r.stdout.contains("1.0.0"),
        "stdout={:?} stderr={:?}",
        r.stdout,
        r.stderr
    );
}
