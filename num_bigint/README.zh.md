# fastshell/num_bigint — malachite 净室替身（Clean-Room Shims）

> English version: [README.md](README.md)

本目录包含 **三个第一方原创 crate**（Apache-2.0），通过 workspace 根
`Cargo.toml` 的 `[patch.crates-io]` 全局替换 RustPython 依赖的同名
LGPL-3.0-only crate：

| 目录 | 替换的 crate | 实现方式 |
|---|---|---|
| `malachite-bigint/` | malachite-bigint (LGPL-3.0) | 转发 **num-bigint**（MIT/Apache-2.0） |
| `malachite-q/` | malachite-q (LGPL-3.0) | 基于 num-bigint 自研 `Rational`（含正确舍入的大整数→f64 除法） |
| `malachite-base/` | malachite-base (LGPL-3.0) | 自研 `RoundingMode` / `RoundingInto` / `PrimitiveFloat` 微型 trait |

## ⚠️ 包名与协议说明

这三个 crate 的 **package name 与原 LGPL crate 相同**（`[patch.crates-io]`
的机制要求同名同版本），但**代码是本仓库的原创作品，协议为 Apache-2.0**，
与 Mikhail Hogrefe 的 malachite 项目没有任何代码关系。任何协议审计工具
如按包名匹配到 "malachite-*"，请以本目录源码头部的版权声明和本 README 为准。

## 净室声明（Clean-Room Statement）

本目录代码的实现过程遵守以下规则，本声明本身构成合规证据：

1. **从未阅读、复制或参考 malachite 家族任何源码**（其为 LGPL-3.0-only）。
2. API 形状的两个合法来源：
   * **num-bigint / num-rational 的公开 API**（MIT/Apache-2.0）——
     malachite-bigint 官方自述即 "a drop-in num-bigint replacement"，
     该 API 的原产地本就是 num-bigint；
   * **RustPython 的调用点**（MIT）—— 需要哪些函数、签名如何，
     以 RustPython 源码（`rustpython-common/src/int.rs` 等）与
     **编译器报错**为准（编译驱动开发）。
3. 算法实现为原创：`malachite-q` 的 `rounding_into`（大整数比值到 f64
   的正确舍入）按 IEEE-754 round-to-nearest-ties-to-even 规则与
   CPython `long_true_divide` 的公开注释思路独立实现，并有单元测试
   覆盖（超大比值、溢出饱和、round-trip、与原生除法一致性）。

## 新增 API 的规则（未来维护者必读）

升级 RustPython 后若编译器报告 shim 缺少某 API：

1. 打开**报错处的 RustPython 源文件**（MIT），确认调用方式与期望语义；
2. 若 num-bigint / num-rational 有对应实现 → 直接转发；
3. 若没有 → 依据调用点语义 + 公开标准（IEEE-754 / CPython 文档）原创实现；
4. **任何情况下都不得打开 malachite 源码"参考一下"** —— 一次接触即污染
   净室属性；
5. 补一条单元测试，并在 PR/commit 信息中注明 API 来源依据。

## 验证命令

```bash
# 替换已生效：malachite-nz（真 malachite 的核心）必须不在依赖图中
cargo tree -i malachite-nz          # 期望: error: nothing depends on / not found

# 三个 shim 自身的单元测试
cargo test -p malachite-base -p malachite-bigint -p malachite-q

# 端到端（RustPython 跑在 shim 上）
cargo test -p fastshell --features python-rustpython --lib rustpython
```
