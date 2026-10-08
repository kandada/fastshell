// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Coverage smoke for the lower-coverage command modules
//! (`extra.rs`, `final_batch.rs`, `hashsum.rs`, `sys_more.rs`, `misc_utils.rs`,
//! `fs_more.rs`, `xargs.rs`, `touch.rs`, `mkdir.rs`, `export.rs`, `wget.rs`,
//! `ssh.rs`). System/network commands are invoked with safe arguments and only
//! required to return cleanly (no panic); `wget`/`curl` are exercised against a
//! local HTTP server.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::io::{Read, Write};
use std::net::TcpListener;

static DIR_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn sandbox_dir() -> std::path::PathBuf {
    let seq = DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_extra_smoke_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha\nbeta\ngamma\n").unwrap();
    std::fs::write(dir.join("f.txt"), "hello\n").unwrap();
    dir
}

/// Fresh SDK per command: a blocking command cannot poison later ones.
fn new_sdk(dir: &std::path::Path) -> Fastshell {
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 200,
        ..Default::default()
    })
    .unwrap();
    s
}

fn ok(s: &Fastshell, cmd: &str) -> String {
    let r = s.execute(cmd);
    assert!(
        (0..=255).contains(&r.exit_code),
        "cmd {cmd:?} bad exit {}: {}",
        r.exit_code,
        r.stderr
    );
    r.stdout
}

/// Serves one fixed body for any request; returns the port.
fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut s = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let body = b"hello-from-server";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.write_all(body);
            let _ = s.flush();
        }
    });
    port
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn system_and_network_commands_smoke() {
    let dir = sandbox_dir();
    let port = serve();
    let url = format!("http://127.0.0.1:{port}/");
    let mut cmds: Vec<String> = [
        // extra.rs
        "renice 0",
        "nohup echo hi",
        "chroot",
        "mkfifo /fifo1",
        "install f.txt /inst.txt",
        "shred f.txt",
        "fallocate -l 16 /fall.bin",
        "ifconfig",
        "netstat",
        "patch --help",
        "mknod",
        "mount",
        "umount",
        // final_batch.rs
        "hostid",
        "echo '1+1' | bc",
        "iostat",
        "vmstat",
        "lsblk",
        "lsof",
        "hdparm",
        "smartctl",
        "blkid",
        "lsusb",
        "ss",
        "ip",
        "ethtool",
        "showmount",
        // hashsum.rs
        "sha256sum a.txt",
        "sha512sum a.txt",
        "md5sum a.txt f.txt",
        // sys_more.rs
        "sha1sum a.txt",
        "sum a.txt",
        "pidof sh",
        "nproc",
        "tty",
        "clear",
        "sync",
        "nice echo hi",
        "groups",
        "dd if=a.txt",
        "od a.txt",
        "uptime",
        "free",
        // misc_utils.rs / fs_more.rs / xargs.rs / touch.rs / mkdir.rs
        "expr 6 \\* 7",
        "split -l 1 a.txt part; ls part* | sort",
        "comm a.txt a.txt",
        "xxd a.txt",
        "mktemp",
        "tac a.txt",
        "printf 'x\\ny\\n' | xargs echo",
        "touch new.txt; ls new.txt",
        "mkdir -p d1/d2; ls -d d1/d2",
        "export X=1; echo $X",
        // ssh.rs
        "ssh",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    cmds.push(format!("wget -O dl.txt {url}; cat dl.txt"));
    cmds.push(format!("curl -s {url}"));
    for cmd in &cmds {
        let s = new_sdk(&dir);
        ok(&s, cmd);
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn wget_and_curl_fetch_local_http() {
    let dir = sandbox_dir();
    // This test performs a real HTTP roundtrip, so give it a longer budget
    // than the blocking-command smoke above.
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
        ..Default::default()
    })
    .unwrap();
    let port = serve();
    let url = format!("http://127.0.0.1:{port}/");
    let out = ok(&s, &format!("wget -O dl.txt {url}; cat dl.txt"));
    assert!(out.contains("hello-from-server"), "{out}");
    let out = ok(&s, &format!("curl -s {url}"));
    assert!(out.contains("hello-from-server"), "{out}");
}
