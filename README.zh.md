# fastshell

轻量级跨平台 Shell 运行时 SDK，面向移动端 AI Agent——提供 160+ Linux 兼容命令、管道、通配符展开、Python 执行。

## 解决的问题

移动平台缺少原生 Bash 环境。AI 编程 Agent 依赖 shell 命令和 Python，没有兼容的运行时就无法在手机上运行。fastshell 提供了一套自包含、可嵌入的运行时 SDK。

## 特性

- **160+ 内置命令** — `ls`、`grep`、`sed`、`awk`、`jq`、`curl`、`git`、`tar`、`sha256sum`……
- **设备能力集成** — `camera`、`clipboard`、`contacts`、`location`、`notify`、`open`、`say`、`screencapture`……通过插件 trait 接入
- **管道支持** — 真正并发执行，每个阶段独立线程，mpsc channel 流式传递
- **通配符展开** — `ls *.rs`、`cat src/**/*.rs`
- **正则表达式** — `grep` 和 `sed s///` 使用完整正则
- **Python 引擎** — `python -c '...'` 和执行 `.py` 脚本
- **虚拟文件系统** — 沙箱隔离，防止路径逃逸
- **线程安全** — `Arc<Mutex<Runtime>>`，支持超时控制
- **跨平台** — 预编译 macOS (ARM64/Intel)、iOS、Android、Linux x86_64

## 快速开始

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

let result = sdk.execute("echo 你好 | wc -c");
assert_eq!(result.exit_code, 0);
println!("{}", result.stdout);

// 直接文件 API
sdk.write_file("hello.txt", "Hello, world!")?;
assert_eq!(sdk.read_file("hello.txt")?, "Hello, world!");

// Python
let r = sdk.execute_python("print(sum(range(1, 101)))");
// 5050
```

### Python 调用 Shell

在 fastshell 中运行的 Python 代码可以直接使用 `subprocess` 和 `asyncio` 调用 shell 命令，无需额外导入：

```python
import subprocess

# 执行 shell 命令
r = subprocess.run("ls -la", shell=True, capture_output=True, text=True)
print(r.stdout)

# 管道也支持
r = subprocess.run("cat file.txt | grep hello | wc -l", shell=True, capture_output=True, text=True)

# 异步 shell
import asyncio
async def main():
    proc = await asyncio.create_subprocess_shell("curl -s https://api.example.com", stdout=asyncio.subprocess.PIPE)
    data, _ = await proc.communicate()
    print(data.decode())
asyncio.run(main())

# os.system 也可以
import os
ret = os.system("mkdir -p /tmp/work")
```

全部 160+ 内置命令、管道、通配符展开都可以在 Python 中使用。桌面端未知命令会转发给系统 shell；移动端**默认禁用** subprocess fallthrough，所有执行保持在进程内。

### 移动端 (FFI)

```c
// Android JNI / iOS C FFI
const char* result = fastshell_init("/data/sandbox");
const char* output = fastshell_execute("ls -la");
fastshell_free_string(output);
```

## API 参考

```rust
pub struct Config {
    pub sandbox_path: String,             // 沙箱路径（必填）
    pub python_enabled: bool,             // 是否启用 Python
    pub command_timeout_ms: u64,          // 超时（毫秒），0 = 不限
    pub allow_subprocess: bool,           // 允许 subprocess fallthrough
                                          //   桌面端默认 true，移动端默认 false
    pub network_ask_permission: bool,     // 网络请求触发用户授权
                                          //   移动端默认 true，桌面端默认 false
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

    // 权限管理（移动端）
    pub fn set_permission(&self, resource: &str, allowed: bool);
    pub fn check_permission(&self, resource: &str) -> Option<bool>;
    pub fn clear_permissions(&self);

    // 取消正在执行的命令（超时或中断用）
    pub fn cancel_execution(&self);

    // 注册设备插件
    pub fn register_plugin(&self, plugin: Box<dyn DevicePlugin>);
}

pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    // exit_code == 100 → 需要用户授权（见下方权限控制章节）
}
```

## 权限控制（移动端）

fastshell 不自行决定网络访问规则，而是通过**特殊退出码**将决策权交给宿主 App：

```
1. 脚本执行 "curl http://example.com"
2. fastshell 检查 "network:example.com" 是否已授权
3. 未授权 → 返回 { exit_code: 100, stderr: "PERMISSION_NEEDED:network:example.com" }
4. 宿主 App 检测 exit_code=100 → 弹出原生对话框:"允许访问 example.com？"
5. 用户点击"允许" → 宿主调用 sdk.set_permission("network:example.com", true) → 重试命令
```

**资源类型：**
| 资源 | 触发场景 |
|------|---------|
| `network:<host>` | `curl`、`wget`、`ping`、`ssh`、`nslookup` |
| `network:*` | 一次性授权所有网络访问 |

**示例流程：**
```rust
let result = sdk.execute("curl http://example.com");
if result.exit_code == 100 {
    // 解析 stderr 获取资源标识，展示原生对话框
    sdk.set_permission("network:example.com", true);
    let result = sdk.execute("curl http://example.com"); // 重试
}
```

## 设备插件

fastshell 内置 22 个**设备能力命令**（`camera`、`clipboard`、`contacts`、`location`、`notify`、`open`……）。
这些命令**默认不工作** —— 需要宿主 App 实现 `DevicePlugin` trait 并注册。

```
┌──────────────────────────────┐      ┌────────────────────────────┐
│  fastshell SDK                │      │  宿主 App (Kotlin/Swift)   │
│                                │      │                            │
│  shell: "camera" → 命令       │      │  impl DevicePlugin {       │
│  检查: 插件已注册?             │──→   │    fn take_photo(path) {   │
│  调用: plugin.take_photo()    │      │      // AVCaptureSession   │
│                                │      │      // 或 CameraX         │
│  返回结果给 AI Agent           │←──   │    }                       │
└──────────────────────────────┘      └────────────────────────────┘
```

**宿主集成示例：**
```rust
use fastshell::sdk::plugin::DevicePlugin;

struct MyPlugin;
impl DevicePlugin for MyPlugin {
    fn take_photo(&self, output_path: &str) -> Result<(), String> {
        // 调起原生相机，照片存入沙盒指定路径
    }
    fn get_clipboard(&self) -> Result<String, String> { ... }
    fn get_location(&self) -> Result<Location, String> { ... }
    // 按需实现其他方法
}

sdk.register_plugin(Box::new(MyPlugin));
```

**权限模型：** 与网络权限一致 —— 首次调用返回 `exit_code=100` 和 `PERMISSION_NEEDED:camera:photo`。
宿主 App 弹出原生授权对话框，调用 `set_permission` 后重试。

**命令兼容性：** 常用 macOS/Linux 命令名均已别名 —— `pbcopy` / `pbpaste`、
`notify-send`、`xdg-open`、`screencapture`、`say`、`arecord` —— AI Agent 无需学习新命令名。

## 移动端集成注意事项

### 单进程保障

在移动端（`allow_subprocess = false`），fastshell 是 **100% 单进程** ——
所有内置命令、管道、Python 执行都在宿主 App 进程内完成，不产生子进程。这避免了：

- **Android 12+ Phantom Process Killer** — 系统限制所有 App 总共最多 32 个 phantom process，超出直接 SIGKILL
- **iOS 禁止 fork()** — iOS 不允许创建子进程

### 管道并发

管道已改为**真正的线程并发** —— 每个 stage 独立线程，mpsc channel 流式传递数据：

```
ls -la | grep foo | wc -l
  线程1      线程2      线程3
```

### VFS 根目录建议

| 平台 | 推荐路径 |
|------|---------|
| Android | `/data/data/<包名>/files/fastshell` |
| iOS | `<app>/Documents/fastshell`（**不要**用 `Library/Caches`，可能被系统清理） |
| 桌面 | 任意可写路径 |

### 网络配置要求

| 平台 | 要求 |
|------|------|
| iOS | `Info.plist` 添加 `NSAllowsArbitraryLoads`，或通过 `NSAppTransportSecurity` 配置域名白名单 |
| Android | `AndroidManifest.xml` 添加 `android:usesCleartextTraffic="true"` |

不配置的话，`curl`/`wget` 的 HTTP 请求在移动端默认失败。

### 保活建议（Android）

宿主 App 应实现 **Foreground Service** + 常驻通知，防止系统在后台杀进程。
各厂商白名单引导步骤参考 [dontkillmyapp.com](https://dontkillmyapp.com)。

### Subprocess Fallthrough

| 平台 | 默认值 | 行为 |
|------|--------|------|
| Android / iOS | `allow_subprocess = false` | 未知命令返回 "command not found (subprocess disabled)" |
| macOS / Linux | `allow_subprocess = true` | 未知命令转发给系统 shell |

内置命令（`ls`、`grep`、`curl`、`git` 等）不受此设置影响，在所有平台都能正常运行。

## 预编译库

| 平台 | 文件 | 大小 |
|------|------|------|
| macOS Apple Silicon | `dist/aarch64-apple-darwin/libfastshell-0.2.1.dylib` | 8.0 MB |
| macOS Intel | `dist/x86_64-apple-darwin/libfastshell-0.2.1.dylib` | 9.0 MB |
| iOS arm64 | `dist/aarch64-apple-ios/libfastshell-0.2.1.a` | 39 MB |
| Android arm64 | `dist/aarch64-linux-android/libfastshell-0.2.1.so` | 10 MB |
| Linux x86_64 | `dist/x86_64-unknown-linux-gnu/libfastshell-0.2.1.so` | 8.5 MB |

### 集成方式

```bash
# Android
# 将 .so 放到 app/src/main/jniLibs/arm64-v8a/

# iOS
# 将 .a 拖入 Xcode → Build Phases → Link Binary With Libraries

# macOS / Linux
# 直接链接 .dylib / .so
```

## 从源码编译

```bash
cd fastshell

# 环境准备
rustup target add aarch64-apple-darwin x86_64-apple-darwin
rustup target add aarch64-apple-ios aarch64-linux-android
rustup target add x86_64-unknown-linux-gnu

# Android NDK
# 下载 android-ndk-r27c 放到项目根目录

# iOS 最低版本
export IPHONEOS_DEPLOYMENT_TARGET=16.0

# macOS
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# iOS
cargo build --release --target aarch64-apple-ios

# Android
cargo build --release --target aarch64-linux-android

# Linux x86_64（需要 cargo-zigbuild）
pip3 install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-gnu

# 测试
cargo test  # 148 个测试
```

## 命令列表

### 文件操作
`ls` `cd` `pwd` `mkdir` `rm` `cp` `mv` `cat` `find` `touch` `chmod` `file` `stat` `du` `basename` `dirname` `realpath`

### 文本处理
`grep` `sed` `awk` `sort` `uniq` `wc` `head` `tail` `cut` `tr` `diff` `tee` `xargs` `column` `paste` `rev` `comm` `xxd` `printf` `seq` `shuf`

### 网络
`curl` `wget` `ping` `ssh`

### 压缩
`gzip` `gunzip` `tar` `zip` `unzip`

### 加密 / 编码
`base64` `sha256sum` `sha512sum` `md5sum`

### JSON
`jq`

### 系统信息
`ps` `kill` `pgrep` `pkill` `env` `printenv` `date` `sleep` `which` `uname` `hostname` `whoami` `id` `df`

### 控制流程
`true` `false` `test` `expr` `timeout`

### 设备能力（需插件）
`camera` `screencapture` `photolib` `record` `arecord` `play` `say` `speech` `contacts` `location` `clipboard` `pbpaste` `pbcopy` `sensor` `notify` `notify-send` `share` `open` `xdg-open` `auth` `battery` `vibrate` `screen` `device`

### 版本控制
`git`

## 架构

```
fastshell/
├── src/
│   ├── vfs/       # 层1 — 虚拟沙箱文件系统
│   ├── shell/     # 层1 — 160+ 内置命令（纯 Rust 实现）
│   ├── python/    # 层1 — Python 引擎（子进程 / CPython）
│   ├── bridge/    # 层2 — 脚本执行、I/O、管道、通配符
│   └── sdk/       # 层3 — 公共 API + 平台 FFI（JNI / C）
└── dist/          # 各平台预编译库
```

## 设计理念

- **轻量** — 纯 Rust 实现，不依赖 BusyBox，无 GPL 许可证问题
- **兼容** — 命令行为与 Linux 一致，AI Agent 无需额外训练
- **安全** — VFS 沙箱隔离，路径逃逸防护，命令超时控制，移动端单进程运行
- **跨平台** — 统一 API，Android/iOS/macOS/Linux 共享同一 Rust 核心
- **权限驱动** — 网络访问需宿主 App 授权，fastshell 不自行决策

## 许可证

Apache 2.0 © xiefujin (490021684@qq.com)
