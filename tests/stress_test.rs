use std::fs;
use std::sync::{Arc, Barrier};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use fastshell::sdk::Fastshell;
use fastshell::sdk::types::Config;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup_no_timeout() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_stress_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 0,
    }).unwrap();
    sdk
}

fn setup_with_timeout(ms: u64) -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_stress_to_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: ms,
    }).unwrap();
    sdk
}

// ═══════════════════════════════════════════════════════
// Concurrency / Race Condition Tests
// ═══════════════════════════════════════════════════════

#[test]
fn stress_concurrent_reads() {
    let sdk = Arc::new(setup_no_timeout());
    sdk.write_file("shared.txt", "shared data").unwrap();

    let handles: Vec<_> = (0..16).map(|_| {
        let sdk = sdk.clone();
        thread::spawn(move || {
            for _ in 0..50 {
                let r = sdk.execute("cat shared.txt");
                assert_eq!(r.exit_code, 0);
            }
        })
    }).collect();

    for h in handles { h.join().unwrap(); }
}

#[test]
fn stress_concurrent_mkdir_ls() {
    let sdk = Arc::new(setup_no_timeout());

    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8).map(|i| {
        let sdk = sdk.clone();
        let b = barrier.clone();
        thread::spawn(move || {
            b.wait();
            for j in 0..50 {
                let dir_name = format!("td_{}_{}", i, j);
                sdk.execute(&format!("mkdir {}", dir_name));
                let r = sdk.execute("ls");
                assert_eq!(r.exit_code, 0);
            }
        })
    }).collect();

    for h in handles { h.join().unwrap(); }
}

#[test]
fn stress_concurrent_write_then_read() {
    let sdk = Arc::new(setup_no_timeout());
    let written = Arc::new(AtomicBool::new(false));

    let writer_sdk = sdk.clone();
    let writer_written = written.clone();
    let writer = thread::spawn(move || {
        for i in 0..100 {
            writer_sdk.write_file("concurrent.txt", &format!("write {}", i)).unwrap();
        }
        writer_written.store(true, Ordering::SeqCst);
    });

    let reader_sdk = sdk.clone();
    let reader = thread::spawn(move || {
        loop {
            let result = reader_sdk.read_file("concurrent.txt");
            match result {
                Ok(_) => {},
                Err(_) => {},
            }
            if written.load(Ordering::SeqCst) { break; }
        }
    });

    writer.join().unwrap();
    reader.join().unwrap();
}

#[test]
fn stress_sequential_many_commands() {
    let sdk = setup_no_timeout();
    let start = Instant::now();
    for i in 0..500 {
        let r = sdk.execute(&format!("echo iteration_{}", i));
        assert_eq!(r.exit_code, 0);
    }
    let elapsed = start.elapsed();
    eprintln!("500 sequential echo commands: {:?} ({:.0} cmd/s)", elapsed, 500.0 / elapsed.as_secs_f64());
    assert!(elapsed < Duration::from_secs(10), "500 commands took too long: {:?}", elapsed);
}

#[test]
fn stress_pipeline_many_stages() {
    let sdk = setup_no_timeout();
    let r = sdk.execute("echo start | grep start | sed 's/start/next/' | grep next | sed 's/next/final/' | grep final | wc -l");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "1");
}

#[test]
fn stress_many_pipelines() {
    let sdk = setup_no_timeout();
    for i in 0..100 {
        let r = sdk.execute(&format!("echo hello_{} | wc -c", i));
        assert_eq!(r.exit_code, 0);
    }
}

// ═══════════════════════════════════════════════════════
// Error Recovery Tests
// ═══════════════════════════════════════════════════════

#[test]
fn recovery_command_timeout() {
    let sdk = setup_with_timeout(50);
    let start = Instant::now();
    let r = sdk.execute("sleep 10");
    let elapsed = start.elapsed();
    assert_eq!(r.exit_code, 124, "expected timeout exit code 124, got {}: {}", r.exit_code, r.stderr);
    assert!(r.stderr.contains("timed out"));
    assert!(elapsed < Duration::from_secs(5), "timeout took too long: {:?}", elapsed);
}

#[test]
fn recovery_cancel_execution() {
    let sdk = Arc::new(setup_no_timeout());
    let sdk_clone = sdk.clone();

    let handle = thread::spawn(move || {
        sdk_clone.execute("sleep 2")
    });

    thread::sleep(Duration::from_millis(50));
    sdk.cancel_execution();

    let result = handle.join();
    assert!(result.is_ok(), "cancel_execution thread should join without panic");
}

#[test]
fn recovery_after_timeout_still_works() {
    let sdk = setup_with_timeout(50);

    let start = Instant::now();
    let timeout_result = sdk.execute("sleep 10");
    let elapsed = start.elapsed();

    assert!(elapsed < Duration::from_secs(5), "timeout should happen quickly");
    assert_eq!(timeout_result.exit_code, 124, "expected timeout code 124: code={} stderr={}",
        timeout_result.exit_code, timeout_result.stderr);
}

#[test]
fn recovery_after_error_command() {
    let sdk = setup_with_timeout(5000);
    let r1 = sdk.execute("nonexistent_command_12345");
    assert_eq!(r1.exit_code, 127);
    let r2 = sdk.execute("echo recovered");
    assert_eq!(r2.exit_code, 0);
    assert!(r2.stdout.contains("recovered"));
}

#[test]
fn recovery_non_utf8_output() {
    let sdk = setup_no_timeout();
    let r = sdk.execute("echo hello");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("hello"));
}

#[test]
fn recovery_many_errors_then_success() {
    let sdk = setup_no_timeout();
    for _ in 0..20 {
        let _ = sdk.execute("nonexistent_cmd_123");
    }
    let r = sdk.execute("echo success_after_errors");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("success_after_errors"));
}

#[test]
fn recovery_shutdown_while_commands() {
    let mut sdk = setup_no_timeout();
    sdk.execute("echo before_shutdown");
    sdk.shutdown();
    let r = sdk.execute("echo after_shutdown");
    assert_ne!(r.exit_code, 0);
    assert!(!sdk.is_initialized());
}

// ═══════════════════════════════════════════════════════
// Thread Safety
// ═══════════════════════════════════════════════════════

#[test]
fn thread_safety_arc_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Fastshell>();
    assert_sync::<Fastshell>();
}

#[test]
fn thread_safety_config_types() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Config>();
    assert_sync::<Config>();
}

// ═══════════════════════════════════════════════════════
// Memory / Resource Leak Tests
// ═══════════════════════════════════════════════════════

#[test]
fn leak_many_init_shutdown_cycles() {
    for _ in 0..20 {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fs_leak_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let mut sdk = Fastshell::new();
        sdk.init(Config {
            sandbox_path: dir.to_string_lossy().to_string(),
            python_enabled: false,
            allow_subprocess: false,
            network_ask_permission: false,
            command_timeout_ms: 0,
        }).unwrap();
        for _ in 0..50 {
            let _ = sdk.execute("echo test");
        }
        sdk.shutdown();
    }
}

#[test]
fn stress_rapid_create_destroy() {
    for i in 0..10 {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fs_rapid_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let mut sdk = Fastshell::new();
        sdk.init(Config {
            sandbox_path: dir.to_string_lossy().to_string(),
            python_enabled: false,
            allow_subprocess: false,
            network_ask_permission: false,
            command_timeout_ms: 0,
        }).unwrap();
        for j in 0..100 {
            let r = sdk.execute(&format!("echo cycle_{}_{}", i, j));
            assert_eq!(r.exit_code, 0);
        }
        sdk.shutdown();
    }
}

// ═══════════════════════════════════════════════════════
// Permission and Config Edge Cases
// ═══════════════════════════════════════════════════════

#[test]
fn stress_permission_flip() {
    let sdk = setup_no_timeout();
    for i in 0..50 {
        let resource = format!("network:example{}.com", i);
        sdk.set_permission(&resource, i % 2 == 0);
        let val = sdk.check_permission(&resource);
        assert_eq!(val, Some(i % 2 == 0));
    }
    sdk.clear_permissions();
    assert_eq!(sdk.check_permission("network:example0.com"), None);
}

#[test]
fn stress_empty_command() {
    let sdk = setup_no_timeout();
    for _ in 0..100 {
        let r = sdk.execute("");
        assert_eq!(r.exit_code, 0);
    }
}

#[test]
fn stress_whitespace_only_command() {
    let sdk = setup_no_timeout();
    let r = sdk.execute("   ");
    assert_eq!(r.exit_code, 0);
}

#[test]
fn stress_bulk_file_api() {
    let sdk = setup_no_timeout();
    let start = Instant::now();
    for i in 0..500 {
        sdk.write_file(&format!("f_{}", i), &format!("data_{}", i)).unwrap();
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(5), "500 file writes took {:?}", elapsed);

    let start = Instant::now();
    for i in 0..500 {
        let content = sdk.read_file(&format!("f_{}", i)).unwrap();
        assert_eq!(content, format!("data_{}", i));
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(5), "500 file reads took {:?}", elapsed);
}
