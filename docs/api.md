# fastshell API Reference

Complete Rust SDK API reference for fastshell v0.2.2.

## Core Types

### `Config`

```rust
pub struct Config {
    /// VFS sandbox root directory (required, must be non-empty)
    /// - Android: /data/data/<pkg>/files/fastshell
    /// - iOS: <app>/Documents/fastshell
    /// - Desktop: any writable path
    pub sandbox_path: String,

    /// Enable embedded CPython engine. When true, fastshell detects
    /// and loads CPython 3.12 from vendor/ or system path.
    /// Default: true
    pub python_enabled: bool,

    /// Command execution timeout in milliseconds. 0 = no timeout.
    /// When timeout triggers, returns exit_code=124 and "command timed out".
    /// Default: 30_000 (30s)
    pub command_timeout_ms: u64,

    /// Allow unknown commands to fall through to system shell via fork/exec.
    /// **Desktop default: true** — system shell available.
    /// **Mobile default: false** — avoids fork (Android phantom process killer, iOS prohibition).
    /// Built-in 180+ commands always work regardless of this setting.
    pub allow_subprocess: bool,

    /// Enable network permission checking. When true, curl/wget/ping/ssh
    /// return exit_code=100 the first time a host is accessed, asking the
    /// host app to authorize before proceeding.
    /// **Mobile default: true, Desktop default: false**
    pub network_ask_permission: bool,
}
```

### `CommandResult`

```rust
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    // exit_code == 0   → success
    // exit_code == 100 → permission needed (see Permission System below)
    // exit_code == 126 → feature not supported (plugin not registered)
    // exit_code == 127 → command not found
    // exit_code == 124 → command timed out
}
```

### `SdkInfo`

```rust
pub struct SdkInfo {
    pub version: String,          // e.g. "0.2.2"
    pub platform: String,         // e.g. "macos", "linux", "android", "ios"
    pub python_available: bool,   // whether CPython engine is loaded
    pub sandbox_path: String,     // current VFS root
    pub allow_subprocess: bool,   // whether subprocess fallthrough is enabled
}
```

### `FileEntry`

```rust
pub struct FileEntry {
    pub name: String,             // file/directory name
    pub path: String,             // full VFS path (relative to cwd)
    pub is_dir: bool,
    pub size: u64,                // bytes
}
```

## Public API

### Lifecycle

```rust
impl Fastshell {
    /// Create a new uninitialized instance. Call `init()` before use.
    pub fn new() -> Self;

    /// Initialize with configuration. Creates VFS sandbox, loads Python engine.
    /// Must be called before any other method.
    /// Returns error if sandbox_path is empty or VFS creation fails.
    pub fn init(&mut self, config: Config) -> Result<(), String>;

    /// Returns true if `init()` has been called successfully.
    pub fn is_initialized(&self) -> bool;

    /// Shut down, clear VFS root directory, release resources.
    pub fn shutdown(&mut self);
}
```

### Command Execution

```rust
impl Fastshell {
    /// Execute a shell command string. Supports pipelines, globs, built-in commands.
    /// Returns CommandResult with stdout, stderr, exit_code.
    ///
    /// Examples:
    ///   sdk.execute("ls -la")
    ///   sdk.execute("cat file.txt | grep hello | wc -l")
    ///   sdk.execute("echo *.rs")
    pub fn execute(&self, command: &str) -> CommandResult;

    /// Execute Python code directly via the embedded CPython engine.
    /// Python can use subprocess, asyncio, os.system — all redirected to fastshell.
    pub fn execute_python(&self, code: &str) -> CommandResult;

    /// Execute a Python script file from VFS.
    pub fn execute_python_script(&self, script_path: &str) -> CommandResult;

    /// Cancel the currently executing command (sets AtomicBool flag).
    /// Useful for interrupt/timeout from host app.
    pub fn cancel_execution(&self);
}
```

### File System API (VFS)

All file APIs go through VFS with path escape prevention.

```rust
impl Fastshell {
    /// Read file contents as UTF-8 string. Returns error if path escapes sandbox.
    pub fn read_file(&self, path: &str) -> Result<String, String>;

    /// Write string content to file. Creates parent directories as needed.
    pub fn write_file(&self, path: &str, content: &str) -> Result<(), String>;

    /// List directory entries sorted by name.
    pub fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, String>;

    /// Check if path exists within sandbox.
    pub fn exists(&self, path: &str) -> bool;

    /// Check if path is a directory.
    pub fn is_dir(&self, path: &str) -> bool;

    /// Get current working directory (virtual path starting with /).
    pub fn get_cwd(&self) -> String;

    /// Get host filesystem path to VFS root directory.
    pub fn vfs_root(&self) -> String;
}
```

### Permission Management

```rust
impl Fastshell {
    /// Grant or deny a permission resource.
    /// After granting, subsequent commands for that resource proceed normally.
    ///
    /// Resource format: "<type>:<identifier>"
    ///   network:example.com   — network access to specific host
    ///   camera:photo          — camera capture
    ///   location:gps          — GPS location access
    ///   contacts:read         — contact list reading
    ///   microphone:record     — audio recording
    pub fn set_permission(&self, resource: &str, allowed: bool);

    /// Check if a permission has been granted/denied.
    /// Returns None if no decision has been made yet (first call).
    pub fn check_permission(&self, resource: &str) -> Option<bool>;

    /// Clear all stored permissions (reset to first-call state).
    pub fn clear_permissions(&self);
}
```

### Device Plugin

```rust
impl Fastshell {
    /// Register a device plugin implementation.
    /// Once registered, 22 device commands (camera, clipboard, location, ...)
    /// become available. See plugin.md for full trait reference.
    ///
    /// Example:
    ///   sdk.register_plugin(Box::new(MyPlugin));
    pub fn register_plugin(&self, plugin: Box<dyn DevicePlugin>);
}
```

### Environment & Info

```rust
impl Fastshell {
    /// Set an environment variable (visible to shell and Python).
    pub fn set_env(&mut self, key: &str, value: &str);

    /// Get an environment variable.
    pub fn get_env(&self, key: &str) -> Option<String>;

    /// Get SDK metadata.
    pub fn get_info(&self) -> SdkInfo;

    /// Get current configuration reference.
    pub fn config(&self) -> &Config;

    /// Get the internal runtime Arc reference (advanced).
    pub fn runtime_ref(&self) -> Arc<Mutex<Runtime>>;
}
```

## Special Exit Codes

| Code | Meaning | stderr format |
|------|---------|---------------|
| 0 | Success | empty |
| 100 | Permission needed | `PERMISSION_NEEDED:<type>:<resource>` |
| 124 | Command timed out | `command timed out` |
| 126 | Feature not supported | `<feature>: not supported (plugin not registered)` |
| 127 | Command not found | `<cmd>: command not found (subprocess disabled)` |

## Permission Flow

```
1. AI agent calls: sdk.execute("curl http://example.com")

2. fastshell checks: is "network:example.com" in permissions?
   → Not found → returns { exit_code: 100, stderr: "PERMISSION_NEEDED:network:example.com" }

3. Host app parses stderr:
   - type: "network"
   - resource: "example.com"
   → Shows native dialog: "Allow network access to example.com?"

4. User taps Allow → host calls: sdk.set_permission("network:example.com", true)

5. Host retries: sdk.execute("curl http://example.com")
   → Permission found, allowed → proceeds normally
```

## Thread Safety

- `Fastshell` uses `Arc<Mutex<Runtime>>` internally
- All execute methods are `&self` (immutable reference) — safe to share across threads
- Pipeline stages run in parallel threads with shell clones (each with isolated cwd)
- Permission map uses `Arc<Mutex<HashMap>>` — concurrent reads/writes are safe
