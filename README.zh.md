# fastshell

轻量级跨平台 Shell 运行时 SDK，面向移动端 AI Agent——提供 180+ Linux 兼容命令、管道、通配符展开、Python 执行、内置 SQLite。

## 解决的问题

移动平台缺少原生 Bash 环境。AI 编程 Agent 依赖 shell 命令和 Python，没有兼容的运行时就无法在手机上运行。fastshell 提供了一套自包含、可嵌入的运行时 SDK。

## 特性

- **180+ 内置命令** — `ls`、`grep`、`sed`、`awk`、`jq`、`curl`、`git`、`tar`、`sha256sum`……
- **内置 SQLite** — `sqlite3 db.sqlite "SELECT ..."`，零系统依赖（rusqlite bundled 静态链接）
- **设备能力集成** — `camera`、`clipboard`、`contacts`、`location`、`notify`、`open`、`say`、`screencapture`……通过插件 trait 接入
- **管道支持** — 真正并发执行，每个阶段独立线程，mpsc channel 流式传递
- **通配符展开** — `ls *.rs`、`cat src/**/*.rs`
- **正则表达式** — `grep` 和 `sed s///` 使用完整正则
- **Python 引擎** — 内嵌 [RustPython](https://github.com/RustPython/RustPython)（MIT，纯 Rust，feature `python-rustpython`）：支持 `ast`/`unittest`/冻结标准库，无 dlopen/JNI/C-TLS 崩溃面。桌面端优先使用系统 `python3`。
- **虚拟文件系统** — 沙箱隔离，防止路径逃逸
- **线程安全** — `Arc<Mutex<Runtime>>`，支持超时控制
- **跨平台** — 统一代码库编译 Android / iOS / macOS / Linux

## 真实应用

[**aacode**](https://github.com/kandada/aacode) —— 移动端 AI 编程助手，运行在 fastshell 之上：
- 整个 shell 沙箱、180+ 命令、内置 Python、设备桥接、原生 C-ABI agent 接口均由 fastshell 提供。
- 可通过 [**APK 下载**](https://github.com/kandada/aacode/raw/main/mobile_app/aacode-v1.7.24-arm64.apk) 安装。Google Play 等应用市场还在上架流程中。
- 可通过  [**APK 下载**](https://github.com/kandada/aacode/raw/main/mobile_app/aacode-v1.7.24-arm64.apk) 安装。Google Play等应用市场还在上架流程中。

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

### 内嵌 RustPython（feature `python-rustpython`）

移动端内嵌 **[RustPython](https://github.com/RustPython/RustPython) 0.5.0** —— 纯 Rust 实现的
Python 3 解释器，直接编译进静态库：没有 `libpython.so`、没有 dlopen、没有磁盘
解压（CPython 纯 Python 标准库通过 `freeze-stdlib` 冻结进二进制）。

> 旧的 Chaquopy CPython 集成已移除：进程内嵌入式 CPython 在真机上崩溃
> （pthread/bionic TLS，`unittest`/`threading` 场景）。RustPython 从架构上
> 消灭了这一整类崩溃。

开箱即用：`ast` 语法检查、`unittest`、`json`、`re`、`os`、沙箱内文件读写、
大整数与正确的浮点除法语义。不支持 C 扩展（numpy 等，任何嵌入方案皆然）。

协议说明：RustPython 上游的 LGPL malachite 依赖已被净室 Apache-2.0 替身替换
（见 `num_bigint/README.md` 与 `vendor/README.md`），最终依赖树全部为宽松协议，
闭源 App 可放心静态链接。

桌面端优先使用系统 `python3`；设置 `FASTSHELL_PYTHON=rustpython` 可强制使用
内嵌引擎（用于在桌面上测试移动端代码路径）。

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

全部 180+ 内置命令、管道、通配符展开都可以在 Python 中使用。桌面端未知命令会转发给系统 shell；移动端**默认禁用** subprocess fallthrough，所有执行保持在进程内。

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

## 从源码编译

需要 Rust 稳定版工具链。

```bash
# 1. 编译目标平台（加 --features python-rustpython 启用内嵌 Python）
cargo build --release --target aarch64-apple-darwin        # macOS ARM64
cargo build --release --target x86_64-apple-darwin          # macOS Intel
cargo build --release --target aarch64-apple-ios            # iOS（需 macOS 宿主机）
cargo build --release --target aarch64-linux-android        # Android（需 NDK；libffi 已随仓库提供，见 vendor/README.md）

# Linux x86_64 交叉编译（需要 cargo-zigbuild）
pip3 install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-gnu

# 2. 运行测试（fastshell 约 500 个；含 aacode-rs 全 workspace 共 687 个）
cargo test --features git,python-rustpython
```

**注意：** Android 目标需要 Android NDK r27c。Python 由内嵌 RustPython 提供，
无需任何外部 CPython 资产。

产物直接链接到你的项目：

```
# macOS / Linux
target/aarch64-apple-darwin/release/libfastshell.dylib

# iOS
target/aarch64-apple-ios/release/libfastshell.a

# Android
target/aarch64-linux-android/release/libfastshell.so
```

编译产物参考（按 `git,python-rustpython` 全 feature 实测；不开则更小）：

| 平台 | 输出文件 | 格式 | 约大小 |
|------|---------|------|--------|
| macOS Apple Silicon | `libfastshell.dylib` | 动态库 | ~14 MB |
| iOS arm64 | `libfastshell.a` | 静态库（链接期可裁剪） | ~90 MB `.a` |
| Android arm64 | `libaacode_rs.a` → `libfastshell_jni.so` | 静态库 + NDK CMake 链接（推荐，见 `fastshell_c/`） | ~218 MB `.a` → **~48 MB `.so`** |
| Linux x86_64 | `libfastshell.so` | 动态库 (.so) | ~14 MB |

移动端推荐 **staticlib + 宿主侧 C/CMake 链接** 路线（Android 见 `fastshell_c/`）：
链接期死代码消除（`--gc-sections`）可大幅缩小最终 `.so`，且单个 `.a` 同时
包含 fastshell + aacode-rs agent + RustPython + git2。

## 命令列表

### 文件操作
`ls` `cd` `pwd` `mkdir` `rm` `cp` `mv` `cat` `find` `tree` `touch` `chmod` `file` `stat` `du` `basename` `dirname` `realpath`

### 文本处理
`grep`（`egrep`/`fgrep`）`rg` `sed` `awk` `sort` `uniq` `wc` `head` `tail` `cut` `tr` `diff` `tee` `xargs` `column` `paste` `rev` `comm` `xxd` `printf` `seq` `shuf`

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

### 数据库
`sqlite3`（内置，bundled，无系统依赖）

### 设备能力（需插件）
`camera` `screencapture` `photolib` `record` `arecord` `play` `say` `speech` `contacts` `location` `clipboard` `pbpaste` `pbcopy` `sensor` `notify` `notify-send` `share` `open` `xdg-open` `auth` `battery` `vibrate` `screen` `device`

### 版本控制
`git`

## 架构

```
fastshell/
├── src/
│   ├── vfs/       # 层1 — 虚拟沙箱文件系统
│   ├── shell/     # 层1 — 180+ 内置命令（纯 Rust 实现）
│   ├── python/    # 层1 — Python 引擎（子进程 / CPython）
│   ├── bridge/    # 层2 — 脚本执行、I/O、管道、通配符
│   └── sdk/       # 层3 — 公共 API + 平台 FFI（JNI / C）
├── docs/          # 文档（API、命令、集成、插件、安全等）
├── tests/         # 集成测试
└── vendor/        # 内嵌 CPython 3.12 库
```

## 设计理念

- **轻量** — 纯 Rust 实现，不依赖 BusyBox，无 GPL 许可证问题
- **兼容** — 命令行为与 Linux 一致，AI Agent 无需额外训练
- **安全** — VFS 沙箱隔离，路径逃逸防护，命令超时控制，移动端单进程运行
- **跨平台** — 统一 API，Android/iOS/macOS/Linux 共享同一 Rust 核心
- **权限驱动** — 网络访问需宿主 App 授权，fastshell 不自行决策

## 许可证

Apache 2.0 © xiefujin (490021684@qq.com)
