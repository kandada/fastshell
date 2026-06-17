# fastshell Integration Guide

End-to-end integration guide for adding fastshell to your Android / iOS / desktop app.

## Quick Start

### 1. Add the library

Copy the pre-built library to your project:

```
# Android
app/src/main/jniLibs/arm64-v8a/libfastshell-0.2.1.so

# iOS
Drag libfastshell-0.2.1.a into Xcode → Build Phases → Link Binary With Libraries

# macOS / Linux
Link libfastshell-0.2.1.dylib / libfastshell-0.2.1.so dynamically
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

1. **Pure Python libraries** — Place `.py` files under:
   ```
   <sandbox>/python/site-packages/
   ```
   Fastshell's CPython engine will find them via `sys.path`.

2. **C extension libraries** (numpy, pillow) — Cross-compile `.so`/`.dylib` using the same NDK/Xcode toolchain as fastshell, place alongside.

No `pip install` needed. Bundle required libraries at build time.

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
