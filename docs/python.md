# Python Engine

fastshell embeds CPython 3.12 for running Python code within the sandbox.

## Architecture

```
fastshell Rust code
  └── cpython.rs
        ├── libloading → loads libpython3.12.dylib/.so/.a
        ├── Py_Initialize() → starts CPython interpreter
        ├── PYTHON_WRAPPER → injected Python hooks
        │     ├── hooks subprocess.run/Popen/os.system
        │     ├── hooks builtins.open/os.open → VFS sandbox
        │     ├── hooks os.chdir/os.listdir → VFS sandbox
        │     └── redirects all shell calls back to fastshell
        └── PyRun_SimpleString() → executes user code
```

## How Shell Becomes Python

When Python code calls `subprocess.run("ls -la", shell=True)`:

```
Python: subprocess.run("ls -la")
  → _hooked_run() intercepts the call
  → _fs_run("ls -la") calls fastshell_python_shell_exec()  
  → fastshell_shell_exec_c() executes through SDK
  → Shell dispatch matches "ls" → cmd_ls() built-in implementation
  → Returns stdout/stderr/exit_code as JSON
  → Python receives CompletedProcess object
```

No subprocess is created. The shell command runs in-process through fastshell's Rust implementation.

## Python Version

CPython 3.12 is embedded via platform-specific shared libraries:

| Platform | Library | Bundled? |
|----------|---------|----------|
| macOS ARM64 | `libpython3.12.dylib` | Yes (gzip compressed in vendor/) |
| macOS Intel | `libpython3.12.dylib` | Yes |
| iOS | `libpython3.12.a` | Yes (static link) |
| Android | `libpython3.12.so` | Yes |
| Linux x86_64 | `libpython3.12.so` | Yes |

If the bundled library is not found, fastshell falls back to searching the system path.

## Python Wrapper

Injected at the first `execute()` call. The wrapper:

1. **Hooks shell execution** — `subprocess.run`, `subprocess.Popen`, `os.system`, `asyncio.create_subprocess_shell`
2. **Hooks file access** — `builtins.open`, `os.open` resolve paths within VFS sandbox root
3. **Hooks directory operations** — `os.chdir`, `os.listdir` resolve paths within sandbox
4. **Sets environment variables** — `FASTSHELL_ROOT`, `FASTSHELL_CWD`

### File Path Sandbox

```python
# User code: open("file.txt")
# Hook resolves: /path/to/sandbox/root/file.txt

# User code: open("/etc/passwd")  
# Hook resolves: /path/to/sandbox/root/etc/passwd (within sandbox)

# User code: open("../../../etc/passwd")
# Hook strips .. components → /path/to/sandbox/root/etc/passwd
```

`..` path traversal is blocked at the hook level by stripping parent components before path construction.

## Using Python in Shell

```bash
# Inline code
python -c "print(1 + 2)"

# Execute script
python /scripts/analyze.py

# Python calling shell commands
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

# Shell commands
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

# File operations (sandboxed)
with open("/data/output.txt", "w") as f:
    f.write("hello")

content = open("/data/input.txt").read()

# os operations (sandboxed)
files = os.listdir("/data")
os.chdir("/data")
```

## Python Libraries

### Bundled Standard Library

CPython 3.12 stdlib is available. Common modules that work:
- `os`, `sys`, `json`, `re`, `math`, `datetime`, `collections`, `itertools`, `functools`, `typing`
- `subprocess`, `asyncio` (redirected to fastshell)
- `http`, `urllib`, `socket` (uses system network through CPython)

### Third-Party Libraries

Place `.py` files or packages under `<sandbox>/python/site-packages/`. fastshell inserts this path into `sys.path` during initialization.

```
sandbox/
├── python/
│   └── site-packages/
│       ├── requests/
│       │   ├── __init__.py
│       │   └── ...
│       └── rich/
│           ├── __init__.py
│           └── ...
```

Then Python code can `import requests` or `import rich` as usual.

**C extension libraries** (numpy, pandas) require cross-compilation with the same toolchain used for fastshell's CPython.

## Limitations

- **No pip** — install libraries by placing files manually
- **No virtualenv** — single global Python environment
- **No ctypes safety** — `ctypes.CDLL(None)` can load system libraries and bypass sandbox
- **GIL** — CPython's Global Interpreter Lock applies; Python code is single-threaded
- **Memory** — CPython runtime adds ~5-10MB memory overhead
