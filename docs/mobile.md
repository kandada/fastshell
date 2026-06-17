# Mobile Integration Notes

Platform-specific requirements and best practices for Android and iOS.

## Single-Process Guarantee

On mobile (`allow_subprocess = false`), fastshell is **100% single-process**. All built-in commands, pipelines, and Python execution run within the host app's process. This avoids:

- **Android 12+ Phantom Process Killer** — kills apps exceeding 32 total phantom processes across all apps combined (SIGKILL signal 9)
- **iOS fork() prohibition** — iOS forbids process creation (`fork`/`exec`)

## Android

### Network (HTTP)

Android 9+ (API 28) blocks cleartext HTTP by default. Required configuration:

**AndroidManifest.xml:**
```xml
<application android:usesCleartextTraffic="true">
```

Or use `network_security_config.xml` for per-domain control.

### Keep-Alive (Foreground Service)

To prevent Android from killing the app in background, implement a Foreground Service:

```kotlin
class FastshellService : Service() {
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("fastshell running")
            .setSmallIcon(R.drawable.ic_terminal)
            .build()
        startForeground(1, notification)
        // Initialize and run fastshell here
        return START_STICKY
    }
}
```

**Manifest:**
```xml
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC" />
<service android:name=".FastshellService"
         android:foregroundServiceType="dataSync" />
```

### Vendor-Specific (Kill Prevention)

Many manufacturers (Xiaomi, Huawei, Samsung, Oppo) have aggressive battery optimization that kills background apps. Guide users to:
1. Disable battery optimization for your app
2. Enable "Auto-start" (Xiaomi, Huawei)
3. Lock app in recent tasks

See [dontkillmyapp.com](https://dontkillmyapp.com) for vendor-specific instructions.

### VFS Root

```kotlin
val sandboxPath = applicationContext.filesDir.resolve("fastshell").absolutePath
// → /data/data/<package>/files/fastshell
```

### APK Size

The `.so` file for arm64 is ~10MB. This adds directly to the APK size.

## iOS

### Network (ATS)

iOS App Transport Security blocks cleartext HTTP. Required in **Info.plist**:

```xml
<key>NSAppTransportSecurity</key>
<dict>
    <key>NSAllowsArbitraryLoads</key>
    <true/>
</dict>
```

Or configure per-domain exceptions for better security.

### Background Execution

iOS limits background execution severely:
- **~30 seconds** of background time after app is suspended
- No reliable way to keep running indefinitely (by design)
- Limited background modes: audio, location, VoIP, background fetch, remote notifications

**For fastshell: accept "foreground-only" execution model on iOS.** The agent interacts with fastshell while the app is active. If the app goes to background, the command may be paused/killed.

### VFS Root

```swift
let documentsPath = FileManager.default
    .urls(for: .documentDirectory, in: .userDomainMask)[0]
    .appendingPathComponent("fastshell")
// → <app>/Documents/fastshell
```

**Do NOT use:**
- `NSTemporaryDirectory()` — may be purged at any time
- `Library/Caches/` — may be purged by system when storage is low

### App Store Review

Interpreted code execution (Python) is allowed if all scripts and the interpreter are bundled with the app (not downloaded at runtime). fastshell's CPython is embedded at build time, meeting this requirement.

Downloading Python packages at runtime (pip) is a gray area — bundle needed libraries at build time instead.

### Binary Size

The `.a` static library for arm64 is ~39MB. This adds directly to the app binary.

## Desktop (macOS / Linux)

No platform restrictions. `allow_subprocess = true` by default, all features available without configuration.

## Common Issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| `curl` fails with TLS error on iOS | ATS blocks HTTP | Add `NSAllowsArbitraryLoads` to Info.plist |
| `curl` fails with connection refused on Android | Cleartext HTTP blocked | Add `usesCleartextTraffic="true"` to Manifest |
| App killed in background (Android) | Battery optimization | Foreground Service + disable battery optimization |
| Commands fail when app is backgrounded (iOS) | iOS background limit | Accept foreground-only execution model |
| "command not found (subprocess disabled)" | Blocked unknown commands | Enable `allow_subprocess` (desktop) or use built-in alternatives |
| Python `import` fails for third-party libs | Library not bundled | Place library files under sandbox/python/site-packages/ |
