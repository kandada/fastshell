# FFI Reference

Platform-specific Foreign Function Interface for C, JNI (Android), and iOS.

## Common C FFI

Available on all desktop platforms (macOS, Linux):

```c
// Initialize with sandbox path
// Returns JSON: {"stdout":"","stderr":"","exit_code":0} on success
const char* fastshell_init(const char* sandbox_path);

// Execute a shell command
// Returns JSON: {"stdout":"...","stderr":"...","exit_code":0}
const char* fastshell_execute(const char* command);

// Set permission for a resource
// resource format: "network:example.com", "camera:photo"
// allowed: 0 = deny, 1 = allow
void fastshell_set_permission(const char* resource, uint8_t allowed);

// Cancel currently executing command
void fastshell_cancel_execution(void);

// Free a string returned by init/execute
void fastshell_free_string(char* ptr);
```

**Example usage (C):**
```c
const char* result = fastshell_init("/tmp/sandbox");
// Parse JSON result...
fastshell_free_string(result);

const char* output = fastshell_execute("ls -la");
printf("%s\n", output);
fastshell_free_string(output);

fastshell_set_permission("network:example.com", 1);
```

## Android (JNI)

```java
// In com/fastshell/Sdk.java
public class Sdk {
    static { System.loadLibrary("fastshell"); }

    // Native methods
    public static native String nativeInit(String sandboxPath);
    public static native String nativeExecute(String command);
    public static native String nativeGetCwd();
    public static native void nativeSetPermission(String resource, boolean allowed);
    public static native void nativeCancelExecution();
}
```

**Example (Kotlin):**
```kotlin
// Initialize
val result = Sdk.nativeInit("/data/data/com.example/files/fastshell")
val json = JSONObject(result)
if (json.getInt("exit_code") != 0) {
    throw Exception("Fastshell init failed: ${json.getString("stderr")}")
}

// Execute
val output = Sdk.nativeExecute("ls -la")
val outJson = JSONObject(output)
println(outJson.getString("stdout"))

// Handle permission
if (outJson.getInt("exit_code") == 100) {
    val stderr = outJson.getString("stderr")
    // stderr: "PERMISSION_NEEDED:network:example.com"
    showDialog { allowed ->
        Sdk.nativeSetPermission("network:example.com", allowed)
        retry()
    }
}

Sdk.nativeCancelExecution() // cancel if timeout
```

## iOS C FFI

```c
// iOS-specific function names
const char* fastshell_ios_init(const char* sandbox_path);
const char* fastshell_ios_execute(const char* command);
void fastshell_ios_set_permission(const char* resource, uint8_t allowed);
void fastshell_ios_cancel_execution(void);
void fastshell_free_string(char* ptr);
```

**Example (Swift):**
```swift
// Bridge header: #include "fastshell_ffi.h"

let result = fastshell_ios_init(strdup(sandboxPath))
if let json = String(cString: result!) {
    // Parse JSON...
}
fastshell_free_string(result)

let output = fastshell_ios_execute(strdup("echo hello"))
print(String(cString: output!))
fastshell_free_string(output)
```

## Return Value Format

All init/execute functions return a C string containing JSON:

```json
{
    "stdout": "command output here\n",
    "stderr": "",
    "exit_code": 0
}
```

**The caller MUST call `fastshell_free_string()`** on every returned pointer to avoid memory leaks.

## Permission Flow (FFI)

```
1. fastshell_execute("curl http://api.example.com")
   → {"exit_code":100, "stderr":"PERMISSION_NEEDED:network:api.example.com"}

2. Host parses exit_code=100, extracts resource from stderr
   → Shows native dialog

3. User approves → fastshell_set_permission("network:api.example.com", 1)

4. fastshell_execute("curl http://api.example.com")
   → {"exit_code":0, "stdout":"...response body..."}
```

## Thread Safety (FFI)

The FFI functions use a global `OnceLock<Mutex<Fastshell>>` singleton. Only one fastshell instance exists per process. All calls are internally synchronized via the mutex.

Concurrent calls from multiple threads are safe — they will queue behind the mutex.
