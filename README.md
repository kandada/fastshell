# fastshell

A lightweight, cross-platform shell runtime SDK for mobile AI agents — providing 180+ Linux-compatible commands, pipelines, glob expansion, Python execution, and built-in SQLite.

## Why

Mobile platforms lack a native Bash environment. AI coding agents rely on shell commands and Python but cannot run on mobile without a compatible runtime. fastshell fills this gap with a self-contained, embeddable SDK.

## Features

- **180+ built-in commands** — `ls`, `grep`, `sed`, `awk`, `jq`, `curl`, `git`, `tar`, `sha256sum`...
- **Built-in SQLite** — `sqlite3 db.sqlite "SELECT ..."`, zero system dependency (bundled via rusqlite)
- **Device integration** — `camera`, `clipboard`, `contacts`, `location`, `notify`, `open`, `say`, `screencapture`... via plugin trait
- **Pipeline support** — True concurrent execution, each stage runs in its own thread with streaming channels
- **Glob expansion** — `ls *.rs`, `cat src/**/*.rs`
- **Regex** — Full regex in `grep` and `sed s///`
- **Python engine** — `python -c '...'` and `.py` script execution
- **Virtual filesystem** — Sandbox isolation, path escape prevention
- **Thread-safe SDK** — `Arc<Mutex<Runtime>>`, timeout enforcement
- **Cross-platform** — Single codebase compiles to Android, iOS, macOS, Linux

## Quick Start

### Rust

```rust
use fastshell::{Fastshell, Config};

let mut sdk = Fastshell::new();
sdk.init(Config {
    sandbox_path: "/tmp/my-sandbox".into(),
    command_timeout_ms: 30_000,
    python_enabled: true,
    allow_subprocess: true,
    network_ask_permission: false,
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

All 180+ built-in commands, pipelines, and globs are available. On desktop, unknown commands fall through to the system shell. On mobile, subprocess fallthrough is **disabled by default** — all execution stays in-process.

### Mobile (FFI)

```c
// Android JNI / iOS C FFI
const char* result = fastshell_init("/data/sandbox");
const char* output = fastshell_execute("ls -la");
fastshell_free_string(output);
```

## API

```rust
pub struct Config {
    pub sandbox_path: String,             // sandbox root path (required)
    pub python_enabled: bool,             // enable Python engine
    pub command_timeout_ms: u64,          // timeout in ms, 0 = no limit
    pub allow_subprocess: bool,           // allow fallthrough to system shell
                                          //   default: true on desktop, false on mobile
    pub network_ask_permission: bool,     // prompt user before network access
                                          //   default: true on mobile, false on desktop
}

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

    // Permission management (mobile)
    pub fn set_permission(&self, resource: &str, allowed: bool);
    pub fn check_permission(&self, resource: &str) -> Option<bool>;
    pub fn clear_permissions(&self);

    // Cancel a running command (for timeout/interrupt)
    pub fn cancel_execution(&self);

    // Device plugin registration
    pub fn register_plugin(&self, plugin: Box<dyn DevicePlugin>);
}

pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    // exit_code == 100 → permission needed (see below)
}
```

## Permission System (Mobile)

fastshell does not make authorization decisions. Instead, it uses a **special exit code** to delegate to the host app:

```
1. Script runs "curl http://example.com"
2. fastshell checks: is "network:example.com" granted?
3. If not → returns { exit_code: 100, stderr: "PERMISSION_NEEDED:network:example.com" }
4. Host app detects exit_code=100 → shows native dialog: "Allow network access to example.com?"
5. User taps "Allow" → host calls sdk.set_permission("network:example.com", true) → retries command
```

**Resource types:**
| Resource | Triggered by |
|----------|-------------|
| `network:<host>` | `curl`, `wget`, `ping`, `ssh`, `nslookup` |
| `network:*` | Grant all network access at once |

**Flow:**
```rust
let result = sdk.execute("curl http://example.com");
if result.exit_code == 100 {
    // parse stderr for resource, show native dialog
    sdk.set_permission("network:example.com", true);
    let result = sdk.execute("curl http://example.com"); // retry
}
```

## Device Plugin

fastshell provides 22 **device capability commands** (`camera`, `clipboard`, `contacts`, `location`, `notify`, `open`...).
These commands **do nothing by default** — the host app must implement the `DevicePlugin` trait and register it.

```
┌──────────────────────────────┐      ┌────────────────────────────┐
│  fastshell SDK                │      │  Host App (Kotlin/Swift)   │
│                                │      │                            │
│  shell: "camera" → command    │      │  impl DevicePlugin {       │
│  checks: plugin registered?   │──→   │    fn take_photo(path) {   │
│  calls: plugin.take_photo()   │      │      // AVCaptureSession   │
│                                │      │      // or CameraX         │
│  returns result to AI agent   │←──   │    }                       │
└──────────────────────────────┘      └────────────────────────────┘
```

**Host app integration:**
```rust
use fastshell::sdk::plugin::DevicePlugin;

struct MyPlugin;
impl DevicePlugin for MyPlugin {
    fn take_photo(&self, output_path: &str) -> Result<(), String> {
        // invoke native camera, save to sandbox
    }
    fn get_clipboard(&self) -> Result<String, String> { ... }
    fn get_location(&self) -> Result<Location, String> { ... }
    // ... implement the methods you need
}

sdk.register_plugin(Box::new(MyPlugin));
```

**Permission model:** Same as network — first call returns `exit_code=100` with `PERMISSION_NEEDED:camera:photo`.
Host app shows native permission dialog, calls `set_permission`, retries.

**Command compatibility:** Common macOS/Linux names are aliased — `pbcopy` / `pbpaste`,
`notify-send`, `xdg-open`, `screencapture`, `say`, `arecord` — AI agents use familiar commands without retraining.

## Mobile Integration Notes

### Single-Process Guarantee

On mobile (`allow_subprocess = false`), fastshell is **100% single-process** — all commands, pipelines,
and Python execution run within the host app's process. No `fork()`, no child processes. This avoids:

- **Android 12+ Phantom Process Killer** — kills apps with >32 total processes (all apps combined)
- **iOS `fork()` prohibition** — iOS forbids process forking entirely

### Pipeline Concurrency

Pipelines now use **true threading** — each stage runs in its own thread with `mpsc` streaming channels:

```
ls -la | grep foo | wc -l
  Thread 1    Thread 2    Thread 3
```

### VFS Root Directory

| Platform | Recommended path |
|----------|-----------------|
| Android | `/data/data/<pkg>/files/fastshell` |
| iOS | `<app>/Documents/fastshell` (NOT `Library/Caches`) |
| Desktop | Any writable path |

### Network Configuration

| Platform | Requirement |
|----------|------------|
| iOS | Add `NSAllowsArbitraryLoads` to `Info.plist`, or configure per-domain exceptions via `NSAppTransportSecurity` |
| Android | Add `android:usesCleartextTraffic="true"` to `AndroidManifest.xml` |

Without these, HTTP requests from `curl`/`wget` will fail silently on mobile.

### Keep-Alive (Android)

Host app should implement a **Foreground Service** with a persistent notification to prevent
Android from killing the process in background. See [dontkillmyapp.com](https://dontkillmyapp.com)
for vendor-specific instructions.

### Subprocess Fallthrough

| Platform | Default | Behavior |
|----------|---------|----------|
| Android / iOS | `allow_subprocess = false` | Unknown commands return "command not found (subprocess disabled)" |
| macOS / Linux | `allow_subprocess = true` | Unknown commands forwarded to system shell |

Built-in commands (`ls`, `grep`, `curl`, `git`, etc.) work everywhere regardless of this setting.

## Build from Source

Requires Rust stable toolchain.

```bash
# 1. One-time setup (installs Rust targets, optional Android NDK)
./scripts/setup.sh

# 2. Build for your target
cargo build --release --target aarch64-apple-darwin        # macOS ARM64
cargo build --release --target x86_64-apple-darwin          # macOS Intel
cargo build --release --target aarch64-apple-ios            # iOS (macOS host)
cargo build --release --target aarch64-linux-android        # Android (needs NDK)

# Linux x86_64 cross-compile (requires cargo-zigbuild)
pip3 install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-gnu

# 3. Run tests (268 tests)
cargo test
```

**Note:** Android target requires Android NDK r27c. `./scripts/setup.sh` will offer to download it.
macOS, iOS, and Linux targets build without NDK.

Linking the library into your project:

```
# macOS / Linux
target/aarch64-apple-darwin/release/libfastshell.dylib

# iOS
target/aarch64-apple-ios/release/libfastshell.a

# Android
target/aarch64-linux-android/release/libfastshell.so
```

Build artifacts per platform (reference):

| Platform | Output File | Format | Approx Size |
|----------|------------|--------|-------------|
| macOS Apple Silicon | `libfastshell.dylib` | dynamic library | ~11 MB |
| macOS Intel | `libfastshell.dylib` | dynamic library | ~13 MB |
| iOS arm64 | `libfastshell.a` | static library | ~47 MB |
| Android arm64 | `libfastshell.so` | shared library (.so) | ~12 MB |
| Linux x86_64 | `libfastshell.so` | shared library (.so) | ~10 MB |

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

### Database
`sqlite3` (built-in, bundled — no system dependency)

### Device (requires plugin)
`camera` `screencapture` `photolib` `record` `arecord` `play` `say` `speech` `contacts` `location` `clipboard` `pbpaste` `pbcopy` `sensor` `notify` `notify-send` `share` `open` `xdg-open` `auth` `battery` `vibrate` `screen` `device`

### Version Control
`git`

## Architecture

```
fastshell/
├── src/
│   ├── vfs/       # Layer 1 — Virtual sandbox filesystem
│   ├── shell/     # Layer 1 — 180+ built-in shell commands (pure Rust)
│   ├── python/    # Layer 1 — Python engine (subprocess / CPython)
│   ├── bridge/    # Layer 2 — Script execution, I/O, pipeline, glob
│   └── sdk/       # Layer 3 — Public API + platform FFI (JNI / C)
├── docs/          # Documentation (API, commands, integration, plugin, security, etc.)
├── tests/         # Integration tests
└── vendor/        # Embedded CPython 3.12 libraries
```

## Design Principles

- **Lightweight** — Pure Rust implementation, no BusyBox dependency, no GPL licensing issues
- **Compatible** — Commands behave identically to Linux; AI agents need no retraining
- **Secure** — VFS sandbox isolation, path escape prevention, command timeout control, single-process on mobile
- **Cross-platform** — Unified API, same Rust core across Android / iOS / macOS / Linux
- **Permission-driven** — Network access requires host app authorization; fastshell never decides on its own

## License

Apache 2.0 © xiefujin (490021684@qq.com)
