# fastshell

A lightweight, cross-platform shell runtime SDK for mobile AI agents — providing 75+ Linux-compatible commands, pipelines, glob expansion, and Python execution.

## Why

Mobile platforms lack a native Bash environment. AI coding agents rely on shell commands and Python but cannot run on mobile without a compatible runtime. fastshell fills this gap with a self-contained, embeddable SDK.

## Features

- **160+ built-in commands** — `ls`, `grep`, `sed`, `awk`, `jq`, `curl`, `git`, `tar`, `sha256sum`...
- **Pipeline support** — `cat file | grep pattern | wc -l` works as expected
- **Glob expansion** — `ls *.rs`, `cat src/**/*.rs`
- **Regex** — Full regex in `grep` and `sed s///`
- **Python engine** — `python -c '...'` and `.py` script execution
- **Virtual filesystem** — Sandbox isolation, path escape prevention
- **Thread-safe SDK** — `Arc<Mutex<Runtime>>`, timeout enforcement
- **Cross-platform** — Pre-built for macOS (ARM64/Intel), iOS, Android, Linux x86_64

## Quick Start

### Rust

```rust
use fastshell::{Fastshell, Config};

let mut sdk = Fastshell::new();
sdk.init(Config {
    sandbox_path: "/tmp/my-sandbox".into(),
    command_timeout_ms: 30_000,
    python_enabled: true,
})?;

let result = sdk.execute("echo hello | grep h | wc -c");
assert_eq!(result.exit_code, 0);
println!("{}", result.stdout);

// Direct file API
sdk.write_file("hello.txt", "Hello, world!")?;
assert_eq!(sdk.read_file("hello.txt")?, "Hello, world!");

// Python
let r = sdk.execute_python("print(sum(range(1, 101)))");
// 5050
```

### Python calling Shell

Python code running in fastshell can use `subprocess` and `asyncio` to call shell commands — no special imports needed:

```python
import subprocess

# Run a shell command
r = subprocess.run("ls -la", shell=True, capture_output=True, text=True)
print(r.stdout)

# Pipelines work
r = subprocess.run("cat file.txt | grep hello | wc -l", shell=True, capture_output=True, text=True)

# Async shell
import asyncio
async def main():
    proc = await asyncio.create_subprocess_shell("curl -s https://api.example.com", stdout=asyncio.subprocess.PIPE)
    data, _ = await proc.communicate()
    print(data.decode())
asyncio.run(main())

# os.system also works
import os
ret = os.system("mkdir -p /tmp/work")
```

All 160+ built-in commands, pipelines, and globs are available. Commands not built-in fall through to the system shell.

### Mobile (FFI)

```c
// Android JNI / iOS C FFI
const char* result = fastshell_init("/data/sandbox");
const char* output = fastshell_execute("ls -la");
fastshell_free_string(output);
```

## API

```rust
impl Fastshell {
    pub fn new() -> Self;
    pub fn init(&mut self, config: Config) -> Result<(), String>;
    pub fn execute(&self, command: &str) -> CommandResult;
    pub fn execute_python(&self, code: &str) -> CommandResult;
    pub fn execute_python_script(&self, script_path: &str) -> CommandResult;
    pub fn get_cwd(&self) -> String;
    pub fn read_file(&self, path: &str) -> Result<String, String>;
    pub fn write_file(&self, path: &str, content: &str) -> Result<(), String>;
    pub fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, String>;
    pub fn exists(&self, path: &str) -> bool;
    pub fn is_dir(&self, path: &str) -> bool;
    pub fn set_env(&mut self, key: &str, value: &str);
    pub fn get_env(&self, key: &str) -> Option<String>;
    pub fn get_info(&self) -> SdkInfo;
    pub fn config(&self) -> &Config;
    pub fn vfs_root(&self) -> String;
    pub fn shutdown(&mut self);
}
```

## Pre-built Libraries

| Platform | File | Size |
|----------|------|------|
| macOS Apple Silicon | `dist/aarch64-apple-darwin/libfastshell-0.1.0.dylib` | 7.8 MB |
| macOS Intel | `dist/x86_64-apple-darwin/libfastshell-0.1.0.dylib` | 8.6 MB |
| iOS arm64 | `dist/aarch64-apple-ios/libfastshell-0.1.0.a` | 38 MB |
| Android arm64 | `dist/aarch64-linux-android/libfastshell-0.1.0.so` | 9.6 MB |
| Linux x86_64 | `dist/x86_64-unknown-linux-gnu/libfastshell-0.1.0.so` | 8.0 MB |

## Build from Source

```bash
cd fastshell

# Prerequisites
rustup target add aarch64-apple-darwin x86_64-apple-darwin
rustup target add aarch64-apple-ios aarch64-linux-android
rustup target add x86_64-unknown-linux-gnu

# Android NDK
# Download android-ndk-r27c and place at project root

# macOS
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# iOS
cargo build --release --target aarch64-apple-ios

# Android
cargo build --release --target aarch64-linux-android

# Linux x86_64 (via zigbuild)
pip3 install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-gnu

# Tests
cargo test  # 115 tests
```

## Commands

### File Operations
`ls` `cd` `pwd` `mkdir` `rm` `cp` `mv` `cat` `find` `touch` `chmod` `file` `stat` `du` `basename` `dirname` `realpath`

### Text Processing
`grep` `sed` `awk` `sort` `uniq` `wc` `head` `tail` `cut` `tr` `diff` `tee` `xargs` `column` `paste` `rev` `comm` `xxd` `printf` `seq` `shuf`

### Network
`curl` `wget` `ping` `ssh`

### Compression
`gzip` `gunzip` `tar` `zip` `unzip`

### Crypto / Encoding
`base64` `sha256sum` `sha512sum` `md5sum`

### JSON
`jq`

### System
`ps` `kill` `pgrep` `pkill` `env` `printenv` `date` `sleep` `which` `uname` `hostname` `whoami` `id` `df`

### Control Flow
`true` `false` `test` `expr` `timeout`

### Version Control
`git`

## Architecture

```
fastshell/
├── src/
│   ├── vfs/       # Layer 1 — Virtual sandbox filesystem
│   ├── shell/     # Layer 1 — 160+ built-in shell commands (pure Rust)
│   ├── python/    # Layer 1 — Python engine (subprocess / CPython)
│   ├── bridge/    # Layer 2 — Script execution, I/O, pipeline, glob
│   └── sdk/       # Layer 3 — Public API + platform FFI (JNI / C)
└── dist/          # Pre-built libraries per platform
```

## Design Principles

- **Lightweight** — Pure Rust implementation, no BusyBox dependency, no GPL licensing issues
- **Compatible** — Commands behave identically to Linux; AI agents need no retraining
- **Secure** — VFS sandbox isolation, path escape prevention, command timeout control
- **Cross-platform** — Unified API, same Rust core across Android / iOS / macOS / Linux

## License

Apache 2.0 © xiefujin (490021684@qq.com)
