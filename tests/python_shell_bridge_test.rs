use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use fastshell::sdk::Fastshell;
use fastshell::sdk::types::Config;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_pybridge_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config { sandbox_path: dir.to_string_lossy().to_string(), python_enabled: true, allow_subprocess: true, network_ask_permission: false, command_timeout_ms: 0 }).unwrap();
    sdk.write_file("hello.py", "print('hello from python')").unwrap();
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
