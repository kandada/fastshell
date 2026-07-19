# Python Engine

fastshell provides Python execution through the `PythonEngine` trait with
platform-appropriate backends:

| Backend | Platform | Mechanism |
|---|---|---|
| `SubprocessPython` | Desktop | spawns the system `python3` |
| `RustPythonEngine` | Mobile (+ desktop fallback) | embedded [RustPython](https://github.com/RustPython/RustPython) 0.5.0, pure Rust, feature `python-rustpython` |
| `UnavailableEngine` | — | clear error when no backend exists |

> **History**: the previous backend embedded Chaquopy CPython 3.12 via
> dlopen. It crashed on real Android devices (pthread/bionic TLS faults in
> `unittest`/`threading`) and was removed in 2026-07. RustPython eliminates
> that crash class by construction — pure Rust, in-process, no dlopen/JNI.

## Architecture (RustPython backend)

```
Host App / shell (`python3 ...` command)
  └── fastshell SDK (Rust)
        └── RustPythonEngine (src/python/rustpython.rs)
              ├── worker thread (16 MB stack — deep interpreter recursion)
              ├── fresh Interpreter per execution (clean state, like
              │   `python3 script.py`; trivially thread-safe)
              ├── frozen stdlib (rustpython-pylib, freeze-stdlib feature —
              │   compiled into the binary, no on-disk extraction)
              ├── wrapper script: io.StringIO capture, os.chdir(cwd) +
              │   restore, real __main__ module + sys.argv, SystemExit →
              │   exit code, traceback → stderr
              └── captured stdout/stderr + exit code → ExecutionResult
```

## What works

- `ast` (agent syntax checks), `unittest` (incl. `unittest.main()`)
- `json`, `re`, `os`, `io`, `sys`, `typing`, `traceback`, file I/O in cwd
- big integers, correctly-rounded `int/int` float division
- `python3 -c`, `python3 script.py`, heredoc scripts via the shell

## Limitations

- No C extensions (numpy, pandas...) — true for any embedded interpreter
- No pip (stdlib is frozen; third-party pure-Python could be added to the
  sandbox and imported via `sys.path` if needed)
- Cancellation is cooperative: a runaway script runs until the SDK-level
  command timeout abandons the worker thread

## Licensing

RustPython is MIT; its frozen stdlib is PSF-2.0. RustPython's upstream
`malachite` big-integer dependencies are **LGPL-3.0-only** and are replaced
workspace-wide by clean-room Apache-2.0 shims (`fastshell/num_bigint/`,
`[patch.crates-io]` in the workspace root). See `fastshell/num_bigint/README.md`
(clean-room statement) and `fastshell/vendor/README.md` (upgrade checklist).
`deny.toml` bans the real malachite crates so upgrades cannot silently
reintroduce LGPL code.

## Android specifics

- `libffi.a` (MIT) is vendored at `fastshell/vendor/libffi/aarch64-linux-android/`
  for rustpython-vm's ctypes support (the NDK ships no libffi); linked via
  `.cargo/config.toml` rustflags and `fastshell_c/CMakeLists.txt`.
- The former `libpython3.12.so` + stdlib assets and the
  `System.loadLibrary("python3.12")` call in the app were removed.

## Forcing the embedded engine on desktop

```bash
FASTSHELL_PYTHON=rustpython cargo run ...   # mobile code path on a desktop
```
