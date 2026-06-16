# fastshell 预编译产物

**直接下载使用，不需要 Rust、Cargo、NDK。**

## 全部平台

| 平台 | 文件 | 大小 |
|---|---|---|
| macOS Apple Silicon | `aarch64-apple-darwin/libfastshell-0.1.0.dylib` | 7.8M |
| macOS Intel | `x86_64-apple-darwin/libfastshell-0.1.0.dylib` | 8.6M |
| iOS arm64 | `aarch64-apple-ios/libfastshell-0.1.0.a` | 38M |
| Android arm64 | `aarch64-linux-android/libfastshell-0.1.0.so` | 9.6M |
| Linux x86_64 | `x86_64-unknown-linux-gnu/libfastshell-0.1.0.so` | 8.0M |

## 集成方式

```bash
# Android
app/src/main/jniLibs/arm64-v8a/libfastshell-0.1.0.so

# iOS
拖入 Xcode → Build Phases → Link Binary With Libraries

# macOS / Linux
直接链接 .dylib / .so
```

## 构建方式

```bash
# Android NDK 文件：项目根目录 android-ndk-r27c.zip (879MB, gitignored)
# 如不存在，从 https://developer.android.com/ndk/downloads 下载放入项目根目录

# 一键安装编译环境
./scripts/setup.sh

# macOS
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# iOS
cargo build --release --target aarch64-apple-ios

# Android（setup.sh 已自动配置 linker）
cargo build --release --target aarch64-linux-android

# Linux x86_64（通过 zig 交叉编译，不依赖系统 gcc）
pip3 install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-gnu
```
