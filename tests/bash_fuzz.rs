// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 语法差分模糊测试：按固定种子从 shell 构造文法生成程序，在 **fastshell** 与
//! **真实 bash** 上分别执行并比较 stdout + 退出码。手写用例只覆盖"已想到的"，
//! 随机生成可发现"未知的未知"。种子固定 → 失败可复现。
//!
//! 合法的平台/版本差异写入 `tests/known_divergences.txt`（子串匹配）后跳过。
//! 无 bash 时自动 SKIP。种子数用环境变量 `FB_FUZZ_SEEDS` 覆盖（默认 250）。

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;

mod common;

/// Reset the generator's free variables on both sides between programs.
const RESET: &str = "unset v; unset x; unset n; unset i; unset a; unset e; set --; ";

/// SplitMix64 — 小、快、确定。
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn n(&mut self, m: usize) -> usize {
        (self.next() as usize) % m
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.n(xs.len())]
    }
}

fn setup() -> (Fastshell, std::path::PathBuf) {
    let fs_dir = std::env::temp_dir().join(format!("fs_fuzz_fs_{}", std::process::id()));
    let sh_dir = std::env::temp_dir().join(format!("fs_fuzz_sh_{}", std::process::id()));
    for d in [&fs_dir, &sh_dir] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).unwrap();
    }
    for (name, body) in [
        ("a.txt", "alpha\n"),
        ("b.txt", "beta\n"),
        ("c.log", "gamma\n"),
        ("nums.txt", "10\n2\n33\n"),
        ("csv.txt", "a,1\nb,2\nc,3\n"),
        ("dup.txt", "x\nx\ny\n"),
    ] {
        fs::write(sh_dir.join(name), body).unwrap();
    }
    fs::create_dir_all(sh_dir.join("sub")).unwrap();
    fs::write(sh_dir.join("sub/d.txt"), "delta\n").unwrap();

    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: fs_dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 15_000,
        ..Default::default()
    })
    .unwrap();
    for (name, body) in [
        ("a.txt", "alpha\n"),
        ("b.txt", "beta\n"),
        ("c.log", "gamma\n"),
        ("nums.txt", "10\n2\n33\n"),
        ("csv.txt", "a,1\nb,2\nc,3\n"),
        ("dup.txt", "x\nx\ny\n"),
    ] {
        sdk.write_file(name, body).unwrap();
    }
    sdk.execute("mkdir -p sub");
    sdk.write_file("sub/d.txt", "delta\n").unwrap();
    (sdk, sh_dir)
}

/// Generates one shell program from the grammar, deterministically from `rng`.
fn gen_program(r: &mut Rng) -> String {
    const WORDS: &[&str] = &["a", "b", "abc", "x", "hello", "1", "2", "0"];
    // Variable names only (never `0`, which is the script-name parameter).
    const VARS: &[&str] = &["v", "x", "n", "i"];
    const SPACED: &[&str] = &["\"x y\"", "'a b'", "\"\"", "''"];
    const FILES: &[&str] = &["a.txt", "b.txt", "c.log", "nums.txt", "csv.txt", "dup.txt"];

    let w = r.pick(WORDS);
    let f = r.pick(FILES);
    let n = r.n(4) + 1;

    // Atoms that are safe in any position.
    let atom = |r: &mut Rng| -> String {
        match r.n(6) {
            0 => r.pick(WORDS).to_string(),
            1 => r.pick(SPACED).to_string(),
            2 => format!("$v"),
            3 => format!("${{{v}}}", v = r.pick(VARS)),
            4 => format!("${{v:-{}}}", r.pick(WORDS)),
            _ => format!("${{#v}}"),
        }
    };

    match r.n(56) {
        0 => format!("echo {}", atom(r)),
        1 => format!("echo {w} {w}"),
        2 => format!("printf '%s\\n' {w}"),
        3 => format!("printf '%d\\n' {n}"),
        4 => format!("printf '%s-%s\\n' {w} {w}"),
        5 => format!("v={w}; echo \"$v\""),
        6 => format!("v={w}; echo \"${{v:-def}}\""),
        7 => format!("v={w}; echo \"${{#v}}\""),
        8 => format!("v=abcabc; echo \"${{v/a/X}}\""),
        9 => format!("v=abcdef; echo \"${{v:1:3}}\""),
        10 => format!("v={w}; echo \"${{v%?}}\""),
        11 => format!("v={w}; echo \"${{v#?}}\""),
        12 => format!("echo $(({} {} {}))", n, r.pick(&["+", "-", "*"]), n),
        13 => format!("i={n}; ((i++)); echo $i"),
        14 => format!("i={n}; ((i+={})); echo $i", r.n(5)),
        15 => format!("x=$(echo {}); echo \"$x\"", atom(r)),
        16 => format!("echo \"$(printf '%s\\n' {w})\""),
        17 => format!("if [ {n} -lt {} ]; then echo lt; else echo ge; fi", n + 1),
        18 => format!("if [ \"{w}\" = \"{w}\" ]; then echo eq; fi"),
        19 => format!("if [[ {w} == {w} ]]; then echo eq; fi"),
        20 => format!("if [[ -f {f} ]]; then echo file; fi"),
        21 => format!("if [[ {n} -gt 1 && -f {f} ]]; then echo both; fi"),
        22 => format!("for i in 1 2 3; do echo $i; done"),
        23 => format!("for i in 1 2 3; do if [ $i -eq 2 ]; then break; fi; echo $i; done"),
        24 => format!("for i in 1 2 3; do if [ $i -eq 2 ]; then continue; fi; echo $i; done"),
        25 => format!("i=0; while [ $i -lt 3 ]; do echo $i; i=$((i+1)); done"),
        26 => format!("case {w} in a*) echo A;; *) echo other;; esac"),
        27 => format!("a=(x y z); echo \"${{a[1]}}\"; echo \"${{#a[@]}}\""),
        28 => format!("a=(x y z); for e in \"${{a[@]}}\"; do echo \"<$e>\"; done"),
        29 => format!("set -- a b c; echo $#; echo $1; echo $*"),
        30 => format!("set -- a b c; shift; echo $1"),
        31 => format!("f(){{ echo \"in $1\"; }}; f hi"),
        32 => format!("f(){{ return {n}; }}; f; echo $?"),
        33 => format!("unset v; f(){{ local v=1; echo $v; }}; f; echo \"[$v]\""),
        34 => format!("cat {f}"),
        35 => format!("head -n {n} {f}"),
        36 => format!("tail -n {n} {f}"),
        37 => format!("wc -l {f}"),
        38 => format!("sort {f}"),
        39 => format!("sort -n {f}"),
        40 => format!("uniq {f}"),
        41 => format!("grep {w} {f} || echo nomatch"),
        42 => format!("cut -d, -f1 {f}"),
        43 => format!("tr a-z A-Z < {f}"),
        44 => format!("sed 's/a/X/g' {f}"),
        45 => format!("awk '{{print $1}}' {f}"),
        46 => format!("awk 'BEGIN{{print {n}}}'"),
        47 => format!("seq 1 {n}"),
        48 => format!("expr {n} + {n}"),
        49 => format!("basename /x/y/z.txt"),
        50 => format!("dirname /x/y/z.txt"),
        51 => format!("cat {f} | head -n 1"),
        52 => format!("cat {f} | grep {w} | wc -l"),
        53 => format!("cat {f} > o.txt; cat o.txt"),
        54 => format!("base64 <<< \"{w}\""),
        55 => format!("printf '%s\\n' $'a\\tb' | cat"),
        _ => format!("echo {w}"),
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn fastshell_matches_bash_on_generated_programs() {
    let bin = common::oracle_bash();
    if !common::runnable(&bin) {
        eprintln!("bash not found; skipping fuzz");
        return;
    }
    let major = common::bash_major(&bin);
    eprintln!("fuzz oracle: {bin} (bash major={major})");
    let seeds: u64 = std::env::var("FB_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let (allow_always, allow_bash3) = common::load_allowlist();
    let (sdk, sh_dir) = setup();
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for seed in 0..seeds {
        let mut r = Rng::new(seed.wrapping_mul(0x1000_0001).wrapping_add(12345));
        let cmd = gen_program(&mut r);
        if allow_always.iter().any(|a| cmd.contains(a.as_str()))
            || (major < 4 && allow_bash3.iter().any(|a| cmd.contains(a.as_str())))
        {
            continue;
        }
        checked += 1;
        // fastshell is one long-lived shell (state persists — correct shell
        // behaviour), while the oracle is a fresh `bash -c` per program. Reset
        // the generator's free variables on both sides so comparisons are fair.
        let _ = sdk.execute("cd /");
        let f = sdk.execute(&format!("{RESET}{cmd}"));
        let (b_raw, b_code) = common::run_oracle(&bin, &sh_dir, &format!("{RESET}{cmd}"));
        let b_out = common::norm(&b_raw);
        let f_out = common::norm(&f.stdout);
        if f_out != b_out || f.exit_code != b_code {
            failures.push(format!(
                "seed={seed}\n  cmd: {cmd}\n  fs : exit={} out={f_out:?}\n  bash: exit={b_code} out={b_out:?}",
                f.exit_code
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "fastshell != bash for {} of {} generated program(s):\n\n{}",
        failures.len(),
        checked,
        failures.join("\n\n")
    );
}
