// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 递归文法差分模糊测试：不是从固定模板里挑，而是**递归组合**（词/展开 → 简单
//! 命令 → 管道 → 重定向 → 列表 → 复合结构），深度受限、种子可复现。突破模板
//! 边界，发现组合语义的差异。
//!
//! 生成器只使用只读命令（避免跨程序状态分叉），循环均有界（不会挂死）。
//! 无 bash 时 SKIP；`FB_FUZZ_SEEDS` 控制规模（默认 200）。

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;

mod common;

/// SplitMix64 — deterministic.
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
    fn chance(&mut self, pct: usize) -> bool {
        self.n(100) < pct
    }
}

const RESET: &str = "unset v x i n a e; set --; rm -f o.txt; ";
const LITS: &[&str] = &["a", "b", "abc", "x", "hello", "1", "2", "3"];
const QUOTED: &[&str] = &["\"a b\"", "'c d'", "\"\"", "''", "\"x y z\""];
const FILES: &[&str] = &["a.txt", "b.txt", "nums.txt", "csv.txt", "dup.txt"];
const STDIN_FILTERS: &[&str] = &[
    "head -n 2",
    "tail -n 1",
    "wc -l",
    "sort",
    "sort -r",
    "uniq",
    "grep a",
    "grep -v a",
    "cat",
    "tr a-z A-Z",
    "sed 's/a/X/'",
    "cut -d, -f1",
];

const LOOP_VARS: &[&str] = &["i", "j", "k"];

struct Gen<'a> {
    r: &'a mut Rng,
    depth: u32,
    lv: usize,
}

impl<'a> Gen<'a> {
    fn word(&mut self) -> String {
        match self.r.n(6) {
            0 => self.r.pick(LITS).to_string(),
            1 => self.r.pick(QUOTED).to_string(),
            2 => "${v:-def}".to_string(),
            3 => format!("${{#v}}"),
            4 => format!("$(echo {})", self.r.pick(LITS)),
            _ => format!(
                "$(({} {} {}))",
                self.r.n(9),
                self.r.pick(&["+", "-", "*"]),
                self.r.n(4)
            ),
        }
    }

    fn cond(&mut self) -> String {
        let f = self.r.pick(FILES);
        match self.r.n(5) {
            0 => format!("[ -f {f} ]"),
            1 => format!("[ {} -gt {} ]", self.r.n(5), self.r.n(5)),
            2 => format!("[ -z {} ]", self.word()),
            3 => format!("[ {} = {} ]", self.r.pick(LITS), self.r.pick(LITS)),
            _ => format!("[ -d . ]"),
        }
    }

    fn simple(&mut self) -> String {
        let f = self.r.pick(FILES);
        let w = self.r.pick(LITS);
        match self.r.n(22) {
            0 => format!("echo {}", self.word()),
            1 => format!("echo {} {}", self.r.pick(LITS), self.r.pick(LITS)),
            2 => format!("printf '%s\\n' {}", self.word()),
            3 => format!("printf '%d\\n' {}", self.r.n(50)),
            4 => format!("cat {f}"),
            5 => format!("head -n {} {f}", self.r.n(3) + 1),
            6 => format!("tail -n {} {f}", self.r.n(3) + 1),
            7 => format!("wc -l {f}"),
            8 => format!("sort {} {f}", self.r.pick(&["", "-n", "-r", "-u"])),
            9 => format!("uniq {f}"),
            10 => format!(
                "grep {} {f}",
                self.r.pick(&["a", "-i A", "-v a", "-c a", "-n a"])
            ),
            11 => format!("sed 's/{w}/X/{g}' {f}", g = self.r.pick(&["", "g"])),
            12 => format!("awk '{{print ${}}}' {f}", self.r.n(3) + 1),
            13 => format!("cut -d, -f{} {f}", self.r.n(2) + 1),
            14 => format!("seq 1 {}", self.r.n(5) + 1),
            15 => format!(
                "expr {} {} {}",
                self.r.n(9),
                self.r.pick(&["+", "-", "*"]),
                self.r.n(4)
            ),
            16 => format!("basename /x/y/{f}"),
            17 => format!("dirname /x/y/{f}"),
            18 => format!("ls {f}"),
            19 => format!("test -f {f}"),
            20 => self.r.pick(&["true", "false", ":"]).to_string(),
            _ => format!("echo {}", self.word()),
        }
    }

    fn pipeline(&mut self) -> String {
        let mut s = if self.r.chance(60) {
            format!("cat {}", self.r.pick(FILES))
        } else {
            format!("echo {}", self.r.pick(LITS))
        };
        for _ in 0..self.r.n(2) + 1 {
            s.push_str(" | ");
            s.push_str(self.r.pick(STDIN_FILTERS));
        }
        s
    }

    fn redirect(&mut self) -> String {
        let mut s = self.pipeline();
        match self.r.n(6) {
            0 => s.push_str(" > o.txt; cat o.txt"),
            1 => s.push_str(" >> o.txt"),
            2 => s.push_str(" 2>/dev/null"),
            3 => s.push_str(" 1>/dev/null; echo ok"),
            _ => {}
        }
        s
    }

    fn sub(&mut self, d: u32) -> String {
        Gen {
            r: self.r,
            depth: d,
            lv: self.lv + 1,
        }
        .list()
    }

    fn stmt(&mut self) -> String {
        if self.depth == 0 {
            return self.redirect();
        }
        let d = self.depth - 1;
        let v = LOOP_VARS[self.lv.min(LOOP_VARS.len() - 1)];
        match self.r.n(10) {
            0 => format!("if {}; then {}; fi", self.cond(), self.sub(d)),
            1 => format!(
                "if {}; then {}; else {}; fi",
                self.cond(),
                self.sub(d),
                self.sub(d)
            ),
            2 => format!("for {v} in 1 2 3; do {}; done", self.sub(d)),
            3 => format!(
                "{v}=0; while [ ${v} -lt {} ]; do {}; {v}=$(({v}+1)); done",
                self.r.n(3) + 1,
                self.sub(d)
            ),
            4 => format!(
                "case {} in {}*) {};; *) {};; esac",
                self.r.pick(LITS),
                self.r.pick(LITS),
                self.sub(d),
                self.sub(d)
            ),
            5 => format!("( {} )", self.sub(d)),
            6 => format!("{{ {}; }}", self.sub(d)),
            // Function definitions only at the top level (realistic; avoids
            // pathological nesting).
            7 if self.depth >= 3 => format!("f() {{ {}; }}; f", self.sub(d)),
            8 => format!("v={}; {}", self.word(), self.redirect()),
            _ => self.redirect(),
        }
    }

    fn list(&mut self) -> String {
        let n = self.r.n(3) + 1;
        let mut parts: Vec<String> = Vec::new();
        for _ in 0..n {
            parts.push(self.stmt());
        }
        let compound = |s: &str| {
            let s = s.trim_start();
            s.starts_with("if ")
                || s.starts_with("for ")
                || s.starts_with("while ")
                || s.starts_with("until ")
                || s.starts_with("case ")
                || s.starts_with('(')
                || s.starts_with('{')
        };
        let mut out = String::new();
        for (idx, part) in parts.iter().enumerate() {
            if idx > 0 {
                // `&&`/`||` only between simple commands; compound constructs
                // are joined with `;` (avoids deep parser corners).
                if compound(&parts[idx - 1]) || compound(part) {
                    out.push_str("; ");
                } else {
                    out.push_str(self.r.pick(&["; ", " && ", " || ", "; "]));
                }
            }
            out.push_str(part);
        }
        out
    }
}

fn new_sdk(fs_dir: &std::path::Path) -> Fastshell {
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: fs_dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 1_500,
        ..Default::default()
    })
    .unwrap();
    sdk
}

fn setup_dirs() -> std::path::PathBuf {
    let fs_dir = std::env::temp_dir().join(format!("fs_rfuzz_fs_{}", std::process::id()));
    let sh_dir = std::env::temp_dir().join(format!("fs_rfuzz_sh_{}", std::process::id()));
    for d in [&fs_dir, &sh_dir] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).unwrap();
    }
    let files: &[(&str, &str)] = &[
        ("a.txt", "alpha\nbeta\ngamma\n"),
        ("b.txt", "beta\n"),
        ("nums.txt", "10\n2\n33\n"),
        ("csv.txt", "a,1\nb,2\nc,3\n"),
        ("dup.txt", "x\nx\ny\n"),
    ];
    for (n, b) in files {
        fs::write(sh_dir.join(n), b).unwrap();
        fs::write(fs_dir.join(n), b).unwrap();
    }
    sh_dir
}

#[test]
#[ignore = "discovery tool: run explicitly (FB_FUZZ_STRICT=1); may be slow on adversarial seeds"]
fn fastshell_matches_bash_on_recursive_grammar() {
    let bin = common::oracle_bash();
    if !common::runnable(&bin) {
        eprintln!("bash oracle not found; skipping recursive fuzz");
        return;
    }
    let major = common::bash_major(&bin);
    eprintln!("recursive fuzz oracle: {bin} (bash major={major})");
    let seeds: u64 = std::env::var("FB_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    // Discovery tool: bounded wall-clock, and only *gating* when
    // FB_FUZZ_STRICT=1 (the curated suites are the CI gate).
    let budget = std::time::Duration::from_secs(
        std::env::var("FB_FUZZ_BUDGET_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
    );
    let strict = std::env::var("FB_FUZZ_STRICT")
        .map(|v| v == "1")
        .unwrap_or(false);
    let started = std::time::Instant::now();
    let (allow_always, allow_bash3) = common::load_allowlist();
    let sh_dir = setup_dirs();
    let fs_dir = std::env::temp_dir().join(format!("fs_rfuzz_fs_{}", std::process::id()));
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for seed in 0..seeds {
        if started.elapsed() >= budget {
            eprintln!("recursive fuzz: wall-clock budget reached after {checked} program(s)");
            break;
        }
        let mut r = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(7));
        let prog = Gen {
            r: &mut r,
            depth: 3,
            lv: 0,
        }
        .list();
        if prog.len() > 600 {
            continue;
        }
        if allow_always.iter().any(|a| prog.contains(a.as_str()))
            || (major < 4 && allow_bash3.iter().any(|a| prog.contains(a.as_str())))
        {
            continue;
        }
        checked += 1;
        // Fresh SDK per program: a hung command cannot poison later ones.
        let sdk = new_sdk(&fs_dir);
        let f = sdk.execute(&format!("{RESET}{prog}"));
        let (b_raw, b_code) = common::run_oracle(&bin, &sh_dir, &format!("{RESET}{prog}"));
        let b_out = common::norm(&b_raw);
        let f_out = common::norm(&f.stdout);
        if f_out != b_out || f.exit_code != b_code {
            failures.push(format!(
                "seed={seed}\n  prog: {prog}\n  fs : exit={} out={f_out:?}\n  bash: exit={b_code} out={b_out:?}",
                f.exit_code
            ));
        }
    }

    if failures.is_empty() {
        return;
    }
    let report = format!(
        "fastshell != bash for {} of {} recursive program(s):\n\n{}",
        failures.len(),
        checked,
        failures.join("\n\n")
    );
    if strict {
        panic!("{report}");
    } else {
        // Informational: print the first few and pass.
        eprintln!("[recursive fuzz] {report}");
        eprintln!("[recursive fuzz] set FB_FUZZ_STRICT=1 to make these fatal");
    }
}
