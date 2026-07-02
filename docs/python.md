# Python Engine

fastshell embeds CPython 3.12 for running Python code within the sandbox.

CPython is embedded at **compile time** via `include_bytes!()`. The library ships inside the app binary — no network download on first launch. See the [integration guide](integration.md#cpython-embedding-production-build) for the build process.

## Architecture

```
Host App (Kotlin/Swift)
  └── fastshell SDK (Rust)
        ├── register_shell_execute()  ← registers C ABI bridge
        ├── register_shell_free()     ← registers C ABI bridge
        └── CpythonEngine
              ├── extract_bundled()   ← decompress .so.gz from binary to sandbox
              ├── libloading::Library::new() ← dlopen
              ├── Py_Initialize()     ← start CPython VM
              ├── inject PYTHON_WRAPPER ← hooks: subprocess, open, os, asyncio
              └── PyRun_SimpleString()← execute user Python code
                    │
                    ▼
              CPython VM (in-process)
                ├── subprocess.run("ls") → _hooked_run() → _fs_run()
                │     → ctypes → fastshell_python_shell_exec() in Rust
                │     → fastshell Shell engine (no fork, no /bin/sh)
                │
                ├── builtins.open("file") → _sandboxed_open()
                │     → _resolve_sandbox_path() → VFS sandbox root + file
                │     → _real_open() on real filesystem (within sandbox)
                │
                ├── os.chdir("dir") → _sandboxed_chdir()
                │     → tracks cwd, resolves within sandbox
                │
                └── os.listdir("dir") → _sandboxed_listdir()
                      → resolves within sandbox, returns entries
```

## How Shell Becomes Python → Shell → Python

When Python code calls `subprocess.run("ls -la", shell=True)`:

```
Python: subprocess.run("ls -la")
  → _hooked_run() intercepts the call (wrapper injected at init)
  → _fs_run("ls -la") marshals the command string
  → ctypes calls fastshell_python_shell_exec() via C ABI
  → fastshell_shell_exec_c() executes through Fastshell SDK
  → Shell dispatch matches "ls" → cmd_ls() built-in implementation
  → Returns JSON: {"stdout": "file1 file2", "stderr": "", "returncode": 0}
  → Python receives CompletedProcess object with stdout/stderr/returncode
```

No subprocess is created. The shell command runs in-process through fastshell's pure Rust implementation.

## Python Version

CPython 3.12 is embedded via platform-specific shared libraries, gzip-compressed and baked into the binary at compile time:

| Platform | Library | Size (compressed) | Bundled? |
|----------|---------|-------------------|----------|
| macOS ARM64 | `libpython3.12.dylib` | ~2 MB | Yes |
| macOS Intel | `libpython3.12.dylib` | ~2 MB | Yes |
| iOS ARM64 | `libpython3.12.a` | — | Planned |
| Android ARM64 | `libpython3.12.so` | — | Placeholder (needs cross-compile) |
| Linux x86_64 | `libpython3.12.so` | — | Placeholder (needs cross-compile) |

> **Note:** Android and Linux targets have 0-byte placeholder files in `vendor/python/`. These prevent compile errors but result in a clear error at runtime: "CPython appears to be a placeholder". Production builds must replace the placeholders with real cross-compiled CPython libraries. See [integration.md](integration.md#cpython-embedding-production-build) for build steps.

If the bundled library is not found (platform not supported, or library not placed in vendor/), fastshell falls back to:
- System `python3` command (desktop only)
- Clear error message with build instructions (mobile)

## Python Wrapper

Injected at the first `execute()` call (once per process, via `OnceLock`). The wrapper:

1. **Hooks shell execution** — `subprocess.run`, `subprocess.Popen`, `os.system`, `asyncio.create_subprocess_shell`
2. **Hooks file access** — `builtins.open`, `os.open`, `os.listdir` resolve paths within VFS sandbox root
3. **Hooks directory operations** — `os.chdir` tracks cwd within sandbox
4. **Sets environment variables** — `FASTSHELL_ROOT`, `FASTSHELL_CWD` for path resolution

### File Path Sandbox

```python
# User code: open("file.txt")
# Hook resolves: <sandbox_root>/file.txt

# User code: open("/etc/passwd")
# Leading / stripped → "etc/passwd"
# Hook resolves: <sandbox_root>/etc/passwd (within sandbox, not real /etc/)

# User code: open("../../../etc/passwd")
# Hook strips .. components → "etc/passwd"
# Hook resolves: <sandbox_root>/etc/passwd (escape prevented)
```

`..` path traversal is blocked at the Python hook level by stripping parent components before path construction, in addition to Rust-level VFS path escape prevention.

## Using Python in Shell

```bash
# Inline code
python -c "print(1 + 2)"

# Execute script
python /scripts/analyze.py

# Python calling shell commands (the key use case for AI agents)
python -c "
import subprocess
r = subprocess.run('ls -la | grep .rs', shell=True, capture_output=True, text=True)
print(r.stdout)
"
```

## Using Shell from Python Code

```python
import subprocess
import asyncio
import os

# Shell commands (all intercepted, no subprocess)
result = subprocess.run("git status", shell=True, capture_output=True, text=True)
print(result.stdout)

# Pipelines (concurrent threads in fastshell)
result = subprocess.run("find . -name '*.rs' | xargs wc -l", shell=True)

# Async
async def run():
    proc = await asyncio.create_subprocess_shell(
        "curl -s https://api.github.com/repos/rust-lang/rust",
        stdout=asyncio.subprocess.PIPE
    )
    data, _ = await proc.communicate()
    return data.decode()

# File operations (all sandboxed to VFS root)
with open("/data/output.txt", "w") as f:
    f.write("hello")

content = open("/data/input.txt").read()

# os operations (all sandboxed)
files = os.listdir("/data")
os.chdir("/data")
```

## Python Libraries

### Bundled Standard Library

CPython 3.12 stdlib is available. Common modules that work:
- `os`, `sys`, `json`, `re`, `math`, `datetime`, `collections`, `itertools`, `functools`, `typing`
- `subprocess`, `asyncio` (redirected to fastshell, not real subprocess)
- `http`, `urllib`, `socket` (uses system network through CPython)

### Third-Party Libraries

> **移动平台：第三方库建议在构建阶段随 App 打包。**
>
> 移动端没有 `pip` 环境，且应用商店审核期望主要功能代码随 App 一同分发。推荐流程：开发机 `pip install` → 复制 `.py` 文件到 App assets → 随 APK/IPA 打包 → App 首次启动时拷贝到 sandbox。fastshell 的 `execute()` 会自动将 `sandbox/python/site-packages/` 加入 `sys.path`，`import` 即可正常工作。
> 详见 [integration.md](integration.md#python-third-party-libraries)。

Place `.py` files or packages under `<sandbox>/python/site-packages/`. fastshell inserts this path into `sys.path` during initialization.

```
sandbox/
├── python/
│   ├── lib/              ← extracted libpython3.12.{so,dylib} (from compile-time embed)
│   └── site-packages/    ← pre-installed pip packages (from App assets)
│       ├── openai/
│       │   ├── __init__.py
│       │   └── ...
│       ├── anthropic/
│       ├── aiohttp/
│       └── ...
│
├── aacode/               ← aacode Python source (from App assets)
│   ├── core/
│   ├── tools/
│   └── ...
```

Then Python code can `import openai` or `from aacode.core.main_agent import MainAgent` as usual — all from local files, no network required.

**C extension libraries** (numpy, pandas) require cross-compilation with the same toolchain used for fastshell's CPython. Place the compiled `.so`/`.dylib` in `site-packages/` alongside pure Python packages. Same bundling process — build-time only, no runtime download.

## Limitations

- **No pip** — install libraries by placing files manually
- **No virtualenv** — single global Python environment
- **No ctypes safety** — `ctypes.CDLL(None)` can load system libraries and bypass sandbox
- **GIL** — CPython's Global Interpreter Lock applies; Python code is single-threaded
- **Memory** — CPython runtime adds ~5-10MB memory overhead
- **`sqlite3` module** — works on macOS/Linux if system CPython has `_sqlite3` compiled in (most do). On mobile (Android/iOS), requires CPython to be cross-compiled with `--with-sqlite3`. As an alternative, fastshell has a built-in `sqlite3` shell command available on all platforms.
- **Re-initialization** — CPython cannot be re-initialized after `Py_Finalize()`. The library handle is leaked (`Box::leak`) intentionally to keep the VM alive for the process lifetime.

## Troubleshooting

**"CPython not available" on mobile:**
```
→ The CPython .so was not embedded at compile time.
→ Add libpython3.12.so.gz to vendor/python/<target-triple>/
→ Rebuild the app.
→ Development shortcut: call CpythonDownloader::ensure_available()
```

**"CPython library not loaded" on desktop:**
```
→ Both system python3 and embedded CPython are missing.
→ Install Python 3.12 via homebrew/apt, OR
→ Add the .dylib/.so.gz to vendor/python/ and rebuild.
```
