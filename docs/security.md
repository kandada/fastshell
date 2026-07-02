# Security Model

fastshell security architecture and best practices.

## VFS Sandbox

All file operations go through a Virtual File System (VFS) rooted at the configured `sandbox_path`. Two layers of path escape prevention:

1. **Component normalization** — `..` components are resolved by popping from a path stack. Paths like `../../../etc/passwd` resolve to the sandbox root (cannot go above).

2. **Symlink canonicalization** — `fs::canonicalize()` resolves real symlinks. If the resolved target lies outside the sandbox root, the operation is rejected with `PathEscape` error.

Operations protected: `read_file`, `write_file`, `list_dir`, `exists`, `is_dir`, `copy`, `rename`, `remove_file`, `remove_dir`.

## Permission System

fastshell uses a **delegated permission model** — it never makes authorization decisions. Instead, it returns special exit codes that the host app handles:

```
Exit code 100 → Permission needed
Exit code 126 → Feature not supported (plugin not registered)
```

The host app is responsible for:
1. Detecting exit_code=100
2. Showing a native permission dialog (with platform-native UI)
3. Calling `set_permission(resource, allowed)` to record the decision
4. Retrying the command

fastshell itself never displays any UI, never requests OS permissions, and never makes policy decisions.

## Subprocess Control

| Platform | Default `allow_subprocess` | Reason |
|----------|---------------------------|--------|
| Android | false | Phantom Process Killer (>32 processes → SIGKILL) |
| iOS | false | `fork()` is forbidden by iOS |
| macOS/Linux | true | Standard desktop environment |

When disabled, unknown commands return "command not found (subprocess disabled)" with exit_code=127. All 180+ built-in commands work regardless.

## Python Sandbox

Embedded CPython executes within fastshell's process. Several sandbox measures:

1. **Shell redirection** — `subprocess.run`, `Popen`, `os.system`, `asyncio.create_subprocess_shell` are all redirected through fastshell's shell dispatcher
2. **File path sandbox** — `builtins.open()` and `os.open()` are hooked to resolve paths within the VFS sandbox root. `..` components are stripped before path construction
3. **os.listdir / os.chdir** — also hooked for sandbox path resolution

**Not sandboxed:** `ctypes.CDLL` (loading native libraries), low-level file descriptors. These are inherent limitations of in-process CPython. For stricter isolation, run Python outside fastshell in a separate process.

## Pipe Execution

Pipeline stages (`cmd1 | cmd2 | cmd3`) run in parallel threads with `mpsc` channels for streaming data. Each stage gets its own `Shell` clone:
- VFS root is shared (PathBuf clone)
- cwd is cloned — `cd` in a pipeline stage does not affect the parent shell
- Permissions map is Arc-shared across all stages

If a pipeline thread panics, the panic is caught via `std::panic::catch_unwind` and all threads are joined before returning an error. No threads are leaked.

## Thread Safety

- `Fastshell` uses `Arc<Mutex<Runtime>>` internally
- All public methods take `&self` (immutable reference), enabling concurrent access
- Permission map uses `Arc<Mutex<HashMap>>` for concurrent reads/writes
- Device plugin uses `Arc<Mutex<Option<Box<dyn DevicePlugin>>>>`
- Mutex poisoning: if a plugin method panics, the poisoned mutex is recovered via `into_inner()`

## Recommendations

### For Production

1. **Set sandbox_path to a dedicated directory** — not shared with other app data
2. **Set command_timeout_ms** — prevent infinite loops from consuming resources
3. **Implement Foreground Service** (Android) — prevent OS from killing the app
4. **Python 代码随 App 打包** — CPython 解释器（vendor/python/，编译期嵌入）、pip 依赖包（app assets → sandbox/site-packages/）、aacode 源码（app assets → sandbox/）均应随 APK/IPA 一同分发。fastshell 的 `execute()` 会自动将 sandbox 和 site-packages 路径注入 `sys.path`，`import` 正常可用。移动端无需也无法使用 `pip install`
5. **Configure cleartext traffic** (Android) and **ATS exceptions** (iOS) — enable HTTP
6. **`CpythonDownloader::ensure_available()` 仅开发调试用** — 生产构建中 CPython 已在编译期嵌入，无需运行时下载

### For Development

1. **Run with `allow_subprocess = true`** on desktop for maximum compatibility
2. **Test with `allow_subprocess = false`** before deploying to mobile
3. **Implement DevicePlugin methods incrementally** — start with the essentials
4. **Monitor exit_code=100** — ensure your permission dialog works end-to-end

## Known Limitations

1. **Python `ctypes.CDLL`** can load system libraries and bypass sandbox
2. **Python `os.open`** with raw file descriptors bypasses path hooks (os-level `open` IS hooked, but low-level `fcntl`/`ioctl` are not)
3. **Subprocess fallthrough** (when enabled) runs commands outside the VFS sandbox
4. **`kill` / `ps`** operate on real host processes (no PID namespace)
5. **No resource quotas** — CPU, memory, disk usage are not limited beyond timeout
6. **No audit logging** — command execution history is not recorded internally
