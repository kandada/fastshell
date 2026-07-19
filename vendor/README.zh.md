# fastshell/vendor — 第三方运行时说明

> English version: [README.md](README.md)

> 本目录曾存放 Chaquopy CPython 资产（libpython3.12 + stdlib，~85MB）。
> **2026-07 起已全部移除**：真机上嵌入式 CPython 在 `unittest`/`threading`
> 场景崩溃（pthread/bionic TLS 不兼容，dlopen 进程内崩溃会带崩整个 App），
> Python 能力由 **RustPython**（纯 Rust，进程内安全）替代。

## 当前 Python 方案：RustPython

| 项 | 值 |
|---|---|
| 引入方式 | crates.io，**精确锁定 `=0.5.0`**（fastshell/Cargo.toml） |
| Feature | `python-rustpython`（fastshell 可选 feature，aacode-rs 默认启用） |
| 组成 | `rustpython-vm` + `rustpython-stdlib` + `rustpython-pylib`(freeze-stdlib) |
| stdlib | CPython 纯 Python 标准库**冻结进二进制**，无磁盘解压、无 PYTHONHOME |
| 引擎实现 | `fastshell/src/python/rustpython.rs`（每次执行全新解释器，16MB 大栈工作线程） |
| 平台 | Android (aarch64) / iOS (aarch64) / macOS / Linux —— 纯 Rust 交叉编译 |

## 协议构成与风险规避（重要）

RustPython 本体是 **MIT**，冻结 stdlib 是 **PSF-2.0**（均为宽松协议）。
但其上游依赖 malachite 家族（malachite-base / malachite-bigint /
malachite-q）是 **LGPL-3.0-only** —— 静态链接进闭源 App 会触发 LGPL
的重链接义务。

**规避方式**：workspace 根 `Cargo.toml` 的 `[patch.crates-io]` 将三个
malachite crate 全局替换为我们自研的净室实现（Apache-2.0，转发到
MIT/Apache 的 num-bigint），位于 `fastshell/num_bigint/`。
详见 `fastshell/num_bigint/README.md` 的净室声明。

**最终依赖树中不存在任何 LGPL/GPL 强制项**，闭源 App 仅需随发行附
第三方署名（Apache-2.0 / MIT / PSF-2.0 / Unicode-3.0 等声明文本）。

## 升级 RustPython 的流程（防止 LGPL 回流，必须逐条执行）

1. 修改 `fastshell/Cargo.toml` 中三个 `rustpython-*` 的固定版本号
2. 若新版本对 malachite 的调用面变化，编译会在 shim 上报错 →
   **只按编译错误、参照 RustPython（MIT）调用点补齐 shim，绝不阅读
   malachite 源码**（见 num_bigint/README.md 净室规则）
3. 验证替换生效（必须为空/报 does not depend）：
   ```bash
   cargo tree -i malachite-nz
   ```
4. 协议全树扫描（不得出现 LGPL/GPL 强制项）：
   ```bash
   cargo license --features git,python-rustpython | grep -iE 'GPL|MPL'
   ```
5. 回归：`cargo test -p fastshell --features git,python-rustpython`
   （重点 `python::rustpython` 模块：ast / unittest / bigint 用例）
6. Android 交叉编译验证：
   ```bash
   cargo build --release --target aarch64-linux-android --lib -p aacode-rs
   ```

## 历史备注

* 旧的 CPython 集成代码（`src/python/cpython.rs`，dlopen + 解压 + JNI
  流式回调，约 1100 行）已随资产一并删除；`PythonEngine` trait 抽象保留，
  桌面端仍优先使用系统 `python3`（SubprocessPython）。
* Android App 侧的 `libpython3.12.so` 与 `System.loadLibrary("python3.12")`
  已同步移除（见 AACodeApp INTEGRATION.md）。
