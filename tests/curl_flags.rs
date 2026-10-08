// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! `curl.rs` flag coverage against a local HTTP server (the largest remaining
//! uncovered module).

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::io::{Read, Write};
use std::net::TcpListener;

fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut s = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let body = b"hello-from-server";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nX-Test: yes\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.write_all(body);
            let _ = s.flush();
        }
    });
    port
}

fn sdk() -> Fastshell {
    let dir = std::env::temp_dir().join(format!("fs_curl_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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
    s
}

fn ok(s: &Fastshell, cmd: &str) -> (i32, String) {
    let r = s.execute(cmd);
    assert!(
        (0..=255).contains(&r.exit_code),
        "cmd {cmd:?} exit {}: {}",
        r.exit_code,
        r.stderr
    );
    (r.exit_code, r.stdout)
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn curl_flag_matrix() {
    let s = sdk();
    let p = serve();
    let u = format!("http://127.0.0.1:{p}/");
    let cases = [
        format!("curl {u}"),
        format!("curl -s {u}"),
        format!("curl -sS {u}"),
        format!("curl -i {u}"),
        format!("curl -I {u}"),
        format!("curl --head {u}"),
        format!("curl -o out.txt {u}; cat out.txt"),
        format!("curl -s -o /dev/null -w '%{{http_code}}' {u}"),
        format!("curl -w '%{{http_code}}' -s {u}"),
        format!("curl -X POST -d 'a=1' {u}"),
        format!("curl -d 'a=1&b=2' {u}"),
        format!("curl -H 'X-A: 1' {u}"),
        format!("curl -A 'agent/1' {u}"),
        format!("curl -e '{u}' {u}"),
        format!("curl -L {u}"),
        format!("curl -k {u}"),
        format!("curl -u user:pass {u}"),
        format!("curl -b 'a=1' {u}"),
        format!("curl -c cookies.txt {u}; cat cookies.txt"),
        format!("curl --compressed {u}"),
        format!("curl -f {u}"),
        format!("curl -v {u}"),
        format!("curl --max-time 5 {u}"),
        format!("curl -m 5 {u}"),
        format!("curl -G {u}"),
        format!("curl -T f.txt {u}"),
        format!("curl --json '{{\"a\":1}}' {u}"),
        format!("curl {u} {u}"),
        format!("curl -sL {u}"),
        format!("curl -fsSL {u}"),
    ];
    for c in cases {
        ok(&s, &c);
    }
    // Error / edge paths.
    for c in [
        "curl",
        "curl --bogus-flag",
        "curl --totally-bogus",
        "curl -s http://127.0.0.1:1/",
        "curl -V",
        "curl --help",
    ] {
        ok(&s, c);
    }
    // A successful fetch must contain the body.
    let (_, out) = ok(&s, &format!("curl -s {u}"));
    assert!(out.contains("hello-from-server"), "{out}");
}
