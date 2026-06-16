# fastshell

轻量级跨平台 Shell 运行时 SDK，面向移动端 AI Agent——提供 75+ Linux 兼容命令、管道、通配符展开、Python 执行。

## 解决的问题

移动平台缺少原生 Bash 环境。AI 编程 Agent 依赖 shell 命令和 Python，没有兼容的运行时就无法在手机上运行。fastshell 提供了一套自包含、可嵌入的运行时 SDK。

## 特性

- **75+ 内置命令** — `ls`、`grep`、`sed`、`awk`、`jq`、`curl`、`git`、`tar`、`sha256sum`……
- **管道支持** — `cat file | grep pattern | wc -l` 正常工作
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

全部 160+ 内置命令、管道、通配符展开都可以在 Python 中使用。不在内置列表的命令会转发给系统 shell。

### 移动端 (FFI)

```c
// Android JNI / iOS C FFI
const char* result = fastshell_init("/data/sandbox");
const char* output = fastshell_execute("ls -la");
fastshell_free_string(output);
```

## API 参考

```rust
impl Fastshell {
    pub fn new() -> Self;
    pub fn init(&mut self, config: Config) -> Result<(), String>;
    pub fn execute(&self, command: &str) -> CommandResult;
    pub fn execute_python(&self, code: &str) -> CommandResult;
    pub fn execute_python_script(&self, script_path: &str) -> CommandResult;
    pub fn get_cwd(&self) -> String;                           // 获取当前工作目录
    pub fn read_file(&self, path: &str) -> Result<String, String>;  // 直接读文件
    pub fn write_file(&self, path: &str, content: &str) -> Result<(), String>;
    pub fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, String>;
    pub fn exists(&self, path: &str) -> bool;
    pub fn is_dir(&self, path: &str) -> bool;
    pub fn set_env(&mut self, key: &str, value: &str);         // 设置环境变量
    pub fn get_env(&self, key: &str) -> Option<String>;
    pub fn get_info(&self) -> SdkInfo;
    pub fn config(&self) -> &Config;
    pub fn vfs_root(&self) -> String;
    pub fn shutdown(&mut self);                                // 清理沙箱
}
```

### Config

```rust
pub struct Config {
    pub sandbox_path: String,       // 沙箱路径（必填）
    pub python_enabled: bool,       // 是否启用 Python
    pub command_timeout_ms: u64,    // 命令超时毫秒，0 表示不超时
}
```

## 预编译库

| 平台 | 文件 | 大小 |
|------|------|------|
| macOS Apple Silicon | `dist/aarch64-apple-darwin/libfastshell-0.1.0.dylib` | 7.8 MB |
| macOS Intel | `dist/x86_64-apple-darwin/libfastshell-0.1.0.dylib` | 8.6 MB |
| iOS arm64 | `dist/aarch64-apple-ios/libfastshell-0.1.0.a` | 38 MB |
| Android arm64 | `dist/aarch64-linux-android/libfastshell-0.1.0.so` | 9.6 MB |
| Linux x86_64 | `dist/x86_64-unknown-linux-gnu/libfastshell-0.1.0.so` | 8.0 MB |

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
cargo test  # 115 个测试
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

### 版本控制
`git`

## 架构

```
fastshell/
├── src/
│   ├── vfs/       # 层1 — 虚拟沙箱文件系统
│   ├── shell/     # 层1 — 75+ 内置命令（纯 Rust 实现）
│   ├── python/    # 层1 — Python 引擎（子进程 / CPython）
│   ├── bridge/    # 层2 — 脚本执行、I/O、管道、通配符
│   └── sdk/       # 层3 — 公共 API + 平台 FFI（JNI / C）
└── dist/          # 各平台预编译库
```

## 设计理念

- **轻量** — 纯 Rust 实现，不依赖 BusyBox，无 GPL 许可证问题
- **兼容** — 命令行为与 Linux 一致，AI Agent 无需额外训练
- **安全** — VFS 沙箱隔离，路径逃逸防护，命令超时控制
- **跨平台** — 统一 API，Android/iOS/macOS/Linux 共享同一 Rust 核心

## 许可证

Apache 2.0 © xiefujin (490021684@qq.com)
