# fastshell Integration Guide

End-to-end integration guide for adding fastshell to your Android / iOS / desktop app.

## Quick Start

### 1. Add the library

Copy the pre-built library to your project:

```
# Android
app/src/main/jniLibs/arm64-v8a/libfastshell.so

# iOS
Drag libfastshell.a into Xcode → Build Phases → Link Binary With Libraries

# macOS / Linux
Link libfastshell.dylib / libfastshell-0.2.1.so dynamically
```

### 2. Initialize

```rust
use fastshell::{Fastshell, Config};

let mut sdk = Fastshell::new();

let config = Config {
    sandbox_path: "/path/to/app/sandbox".into(),
    python_enabled: true,
    command_timeout_ms: 30_000,
    allow_subprocess: false,   // mobile: false, desktop: true
    network_ask_permission: true,  // mobile: true, desktop: false
};

sdk.init(config)?;
```

### 3. Execute commands

```rust
// Shell command
let result = sdk.execute("ls -la");
println!("stdout: {}", result.stdout);
println!("exit: {}", result.exit_code);

// Python
let result = sdk.execute_python("print(sum(range(1, 101)))");

// Pipeline (concurrent threads)
let result = sdk.execute("cat file.txt | grep hello | wc -l");

// File API (VFS)
sdk.write_file("hello.txt", "Hello, world!")?;
let content = sdk.read_file("hello.txt")?;
```

### 4. Handle permissions

```rust
let result = sdk.execute("curl http://api.example.com");

if result.needs_permission() {
    // Parse stderr: "PERMISSION_NEEDED:network:api.example.com"
    let resource = parse_permission_resource(&result.stderr);
    // Show native dialog to user
    if user_allowed {
        sdk.set_permission(&resource, true);
        // Retry command
        let result = sdk.execute("curl http://api.example.com");
    }
}
```

## Python Engine (Embedded RustPython)

### How It Works

Python support is provided by **embedded [RustPython](https://github.com/RustPython/RustPython) 0.5.0**
(feature `python-rustpython`) — a Python 3 interpreter written in pure Rust that
compiles directly into the fastshell library:

```
rustpython-vm + rustpython-stdlib + rustpython-pylib (freeze-stdlib)
    ▼ compiled into the Rust binary — no libpython, no dlopen, no assets
RustPythonEngine (src/python/rustpython.rs)
    ▼ fresh interpreter per execution on a 16 MB-stack worker thread
    ▼ frozen CPython stdlib (ast / unittest / json / re / os ...)
    ▼ stdout/stderr captured, cwd sandboxed, SystemExit → exit code
```

> The previous integration embedded Chaquopy CPython 3.12 via dlopen. It
> crashed on real Android devices (pthread/bionic TLS in unittest/threading)
> and was removed in 2026-07. See `vendor/README.md` for the full history,
> licensing notes (clean-room malachite shims) and the upgrade checklist.

### Build

```bash
# Desktop / CI
cargo build --release --features git,python-rustpython

# Android (static libffi for ctypes is bundled at vendor/libffi/)
cargo build --release --target aarch64-linux-android --lib -p aacode-rs
```

On desktop the system `python3` is preferred at runtime; set
`FASTSHELL_PYTHON=rustpython` to force the embedded engine.


## Platform-Specific Configuration

### Android

**AndroidManifest.xml:**
```xml
<!-- Required for HTTP requests -->
<application android:usesCleartextTraffic="true">
    ...
</application>

<!-- Optional: Foreground Service for background execution -->
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<service android:name=".FastshellService"
         android:foregroundServiceType="dataSync" />
```

**VFS root directory:**
```kotlin
val sandboxPath = context.filesDir.resolve("fastshell").absolutePath
```

### iOS

**Info.plist (for HTTP requests):**
```xml
<key>NSAppTransportSecurity</key>
<dict>
    <key>NSAllowsArbitraryLoads</key>
    <true/>
</dict>
```

**VFS root directory:**
```swift
let sandboxPath = FileManager.default
    .urls(for: .documentDirectory, in: .userDomainMask)[0]
    .appendingPathComponent("fastshell").path
// NOTE: Use Documents/, NOT Library/Caches/ (which may be purged by iOS)
```

### Desktop (macOS / Linux)

No special configuration needed. Any writable path works.

## Device Plugin Integration

Register device capabilities to enable camera, clipboard, contacts, etc:

```rust
use fastshell::sdk::plugin::DevicePlugin;

struct MyAppPlugin;
impl DevicePlugin for MyAppPlugin {
    fn take_photo(&self, output_path: &str) -> Result<(), String> {
        // iOS: AVCaptureSession
        // Android: CameraX or Intent
        // Save photo to VFS sandbox at output_path
    }
    fn get_clipboard(&self) -> Result<String, String> {
        // iOS: UIPasteboard.general.string
        // Android: ClipboardManager
    }
    fn get_location(&self) -> Result<Location, String> {
        // iOS: CLLocationManager
        // Android: FusedLocationProviderClient
    }
    // ... implement other methods as needed
}

sdk.register_plugin(Box::new(MyAppPlugin));
```

See `plugin.md` for the complete 30-method trait reference.

## Python Third-Party Libraries

To use Python libraries like `requests`, `rich`, `jinja2`:

> **移动平台注意事项：Python 代码应在构建阶段打包进 APK/IPA。**
> 移动端没有 pip 环境，且应用商店审核要求主要功能代码随 App 一同分发。建议在开发机上预安装依赖，将 `.py` 文件作为 App assets 随 APK/IPA 一起打包。App 首次启动时拷贝到 sandbox，fastshell 会自动将 `sandbox/python/site-packages/` 加入 `sys.path`，`import` 即正常可用。

### Build-time Bundling Pipeline

```
开发机                           App 构建                          用户手机
  │                                │                                │
  ├─ pip install openai            │                                │
  │    (下载到 site-packages/)      │                                │
  │                                │                                │
  ├─ 复制 .py 文件到 assets/       │                                │
  │                                │                                │
  │                                ├─ APK 打包 ──────────────────→  │
  │                                │   assets 随 APK                │
  │                                │   嵌入到安装包中                │
  │                                │                                │
  │                                │                                ├─ App 启动
  │                                │                                │   复制 assets 到 sandbox
  │                                │                                │   python/site-packages/
  │                                │                                │
  │                                │                                ├─ CPython 从 sandbox
  │                                │                                │   sys.path 加载
  │                                │                                │   import openai ✓
```

### 1. Pure Python libraries

Pre-install on dev machine, then copy to app assets:

```bash
# 1. 在开发机上 pip install
pip install openai anthropic aiohttp -t ./site-packages/

# 2. 将 site-packages/ 放入 Android 项目的 assets/ 目录
cp -r site-packages/ android/app/src/main/assets/python/site-packages/

# 3. App 启动时复制到 sandbox（Kotlin 示例）
fun copyPythonLibs(context: Context, sandboxPath: String) {
    val dest = File(sandboxPath, "python/site-packages")
    dest.mkdirs()
    context.assets.list("python/site-packages")?.forEach { name ->
        context.assets.open("python/site-packages/$name").use { input ->
            File(dest, name).outputStream().use { output ->
                input.copyTo(output)
            }
        }
    }
}
```

Fastshell's CPython engine finds them via `sys.path`:
```
<sandbox>/python/site-packages/
├── openai/
│   ├── __init__.py
│   └── ...
├── anthropic/
└── aiohttp/
```

### 2. C extension libraries (numpy, pillow)

Cross-compile `.so`/`.dylib` using the same NDK/Xcode toolchain as fastshell, place alongside pure Python packages in assets. Same bundling process — just `.so` files instead of `.py` files.

**Never call `pip install` on the user's device.** This will:
- Fail on Android (no pip available)
- Cause App Store rejection on iOS
- Require network access for what should be offline functionality
- Add unpredictable delays on first launch

## AI Agent Integration Pattern

```
┌─────────────┐     execute("ls")     ┌──────────────┐
│  AI Agent    │ ──────────────────→  │  fastshell     │
│  (aacode/    │ ←──────────────────  │  SDK           │
│   fastclaw)  │   CommandResult      │               │
└──────┬───────┘                      └───────┬───────┘
       │                                      │
       │ parse stdout/stderr                  │ VFS sandbox
       │ generate next command                │ built-in commands
       ▼                                      ▼
    LLM Model                              File System + Network
```

## Error Handling

```rust
let result = sdk.execute(command);

match result.exit_code {
    0   => { /* success, use result.stdout */ }
    100 => {
        // Permission needed — show dialog, retry
        let resource = extract_from_stderr(&result.stderr);
        show_permission_dialog(resource);
    }
    124 => { /* timeout — inform agent or retry */ }
    126 => { /* feature not supported — plugin not registered */ }
    127 => { /* command not found — agent made a mistake */ }
    _   => { /* command failed — pass stderr to agent */ }
}
```

## Thread Safety

`Fastshell` is thread-safe. Multiple threads can call `execute()` concurrently:

```rust
let sdk = Arc::new(sdk);

for i in 0..10 {
    let sdk = sdk.clone();
    thread::spawn(move || {
        sdk.execute(&format!("echo thread {}", i));
    });
}
```

Pipeline stages also run in parallel threads automatically.

## Shutdown

```rust
sdk.shutdown();  // cleans up VFS root directory
```
