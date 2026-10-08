// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 差分测试：同一批 shell 语义命令在 **fastshell** 与 **真实 bash** 上分别执行，
//! 比较 stdout 与退出码。手写用例难以覆盖的兼容性 bug（glob/引号/管道/循环/
//! 变量/条件/命令替换/花括号）能被一次性批量暴露。
//!
//! 只比较 **stdout + exit code**（stderr 文案各实现不同，不比较）。无 `bash`
//! 时自动跳过。

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn bash_available() -> bool {
    common::runnable(&common::oracle_bash())
}

fn norm(s: &str) -> String {
    common::norm(s)
}

/// 在 fastshell 沙箱与一个 bash 工作目录里创建相同 fixture。
fn setup() -> (Fastshell, std::path::PathBuf) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let fs_dir = std::env::temp_dir().join(format!("fs_diff_fs_{}_{}", std::process::id(), n));
    let sh_dir = std::env::temp_dir().join(format!("fs_diff_sh_{}_{}", std::process::id(), n));
    for d in [&fs_dir, &sh_dir] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).unwrap();
    }
    // 相同 fixture
    for (name, body) in [
        ("a.txt", "alpha\n"),
        ("b.txt", "beta\n"),
        ("c.log", "gamma\n"),
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
        command_timeout_ms: 30_000,
        ..Default::default()
    })
    .unwrap();
    sdk.write_file("a.txt", "alpha\n").unwrap();
    sdk.write_file("b.txt", "beta\n").unwrap();
    sdk.write_file("c.log", "gamma\n").unwrap();
    sdk.execute("mkdir -p sub");
    sdk.write_file("sub/d.txt", "delta\n").unwrap();

    (sdk, sh_dir)
}

fn bash_run(dir: &std::path::Path, cmd: &str) -> (String, i32) {
    common::run_oracle(&common::oracle_bash(), dir, cmd)
}

/// 应一致的 shell 语义用例（输出确定、且两实现都应支持）。
const CASES: &[&str] = &[
    "echo hello",
    "echo a b c",
    "echo \"a  b\"",
    "echo 'a  b'",
    "echo \"nested 'single'\"",
    "echo \"a\\\"b\"",
    "echo \\*",
    "echo \"a\" \"b\"",
    "echo *.txt",
    "for f in *.txt; do echo $f; done",
    "for f in a.txt b.txt; do echo \"$f\"; done",
    "for i in 1 2 3; do echo $i; done",
    "x=5; echo $((x+1))",
    "echo $((2*3+1))",
    "echo $((10 % 3))",
    "v=\"a b\"; echo \"$v\"",
    "echo $(echo hi)",
    "echo \"$(echo nested $(echo deep))\"",
    "echo one; echo two",
    "echo {a,b}c",
    "printf '%s\\n' a b c",
    "printf '%d\\n' 42",
    "if [ 1 -eq 1 ]; then echo yes; else echo no; fi",
    "if [ 1 -eq 2 ]; then echo yes; else echo no; fi",
    "case abc in a*) echo match;; *) echo no;; esac",
    "case xyz in a*) echo match;; *) echo other;; esac",
    "true && echo ok",
    "false || echo fallback",
    "echo a > out.txt; cat out.txt",
    "echo l1 > f2; echo l2 >> f2; cat f2",
    "echo skip > /dev/null; echo done",
    "cd sub && echo $PWD > /dev/null; echo moved",
    "for f in sub/*.txt; do echo $f; done",
    // `'\''` idiom + command substitution in a for-list + pipelines after it.
    r#"echo 'a'\''b'"#,
    r#"for x in $(echo a b c); do echo "<$x>"; done"#,
    r#"v=$(echo hi); echo "$v""#,
    r#"echo $(printf 'x\ny\n' | head -n 1)"#,
    r#"printf "x'\ny\n" | grep -E 'x'\''|y' | head -n 1"#,
    // ── 精细语料：引号 / 转义 / 变量 / 算术 / 条件 / case / 循环 / 管道 / 重定向 / glob / sed ──
    r#"echo "a'b""#,
    r#"echo 'a"b'"#,
    r#"echo a\ b"#,
    r#"printf '%s\n' "a b""#,
    r#"x=1; echo "${x}2""#,
    r#"echo "\$x""#,
    r#"echo 'it'\''s'"#,
    r#"a=1; b=2; echo $((a+b))"#,
    r#"echo $((7/2))"#,
    r#"echo $((7%2))"#,
    r#"echo $((10 - 3 * 2))"#,
    r#"echo $(( (1+2)*3 ))"#,
    r#"x=$(echo hi); echo $x"#,
    r#"if [ -z "" ]; then echo empty; fi"#,
    r#"if [ -n "x" ]; then echo nonempty; fi"#,
    r#"if [ "a" = "a" ]; then echo eq; fi"#,
    r#"if [ 2 -gt 1 ]; then echo gt; fi"#,
    r#"if [ -f a.txt ]; then echo file; fi"#,
    r#"if [ -d sub ]; then echo dir; fi"#,
    r#"if false; then echo no; else echo yes; fi"#,
    r#"if true; then echo a; elif true; then echo b; else echo c; fi"#,
    r#"case abc in a*) echo A;; *) echo other;; esac"#,
    r#"case 5 in [0-9]) echo digit;; *) echo no;; esac"#,
    r#"i=0; while [ $i -lt 3 ]; do i=$((i+1)); echo $i; done"#,
    r#"printf 'b\na\nc\n' | sort"#,
    r#"printf 'a\na\nb\n' | uniq"#,
    r#"printf 'a\nb\nc\n' | head -n 2"#,
    r#"printf 'a\nb\nc\n' | tail -n 2"#,
    r#"echo 'a b c' | cut -d' ' -f2"#,
    r#"printf 'x\ny\n' | grep y"#,
    r#"echo hi > o.txt; cat o.txt"#,
    r#"echo a > o2; echo b >> o2; cat o2"#,
    r#"echo x 2>/dev/null; echo y"#,
    r#"cat < a.txt"#,
    r#"echo {1..3}"#,
    r#"echo {a,b}{c,d}"#,
    r#"echo abc | sed 's/b/X/'"#,
    r#"printf 'a\nb\n' | grep -v a"#,
    // ── 更难：参数展开 / [[ ]] / printf 格式 / sed 反向引用 / 负数 head·tail / 多级管道 ──
    r#"echo $(echo $(echo x))"#,
    r#"echo "$(echo 'a b')""#,
    r#"x="a b"; echo $x"#,
    r#"printf '%s\n' a "b c""#,
    r#"n=5; echo $((n*2))"#,
    r#"false || echo ok"#,
    r#"true && echo ok"#,
    r#"false; echo $?"#,
    r#"true; echo $?"#,
    r#"echo a; false; echo b"#,
    r#"printf '%d-%s\n' 5 hi"#,
    r#"printf '%05d\n' 42"#,
    r#"printf '%-5s|\n' ab"#,
    r#"echo abc | sed -E 's/(a)(b)/\2\1/'"#,
    r#"printf 'l1\nl2\nl3\n' | sed -n '2p'"#,
    r#"printf 'a\nb\nb\n' | grep -c b"#,
    r#"echo abcd | grep -o -E 'b|d'"#,
    r#"echo abcdef | cut -c1-3"#,
    r#"printf '3\n1\n2\n' | sort -n"#,
    r#"printf 'a\nb\n' | sort -r"#,
    r#"printf 'l1\nl2\nl3\n' | tail -n +2"#,
    r#"echo abc | tr a-z A-Z"#,
    r#"echo abc | rev"#,
    r#"printf 'a b c\n' | tr ' ' '\n' | sort | head -n 1"#,
    r#"if [ 3 -eq 3 ]; then echo eq; fi"#,
    r#"if [ ! -f nope.xyz ]; then echo absent; fi"#,
    r#"if [ -e a.txt ] && [ -d sub ]; then echo both; fi"#,
    r#"case b in a|b) echo ab;; esac"#,
    r#"echo sub/*"#,
    r#"echo */"#,
    r#"export V=1; echo $V"#,
    r#"x=abcdef; echo ${x#abc}"#,
    r#"x=abcdef; echo ${x%def}"#,
    r#"echo "$?" "#,
    // ── 更多命令：tr / cut / awk / sort -k / uniq -d / xargs / tee / seq / basename / dirname / tac / paste / base64 / sed 进阶 / find ──
    r#"echo abc | tr a-c x-z"#,
    r#"echo 'a  b' | tr -s ' '"#,
    r#"echo abc | tr -d b"#,
    r#"printf 'a:b:c\n' | cut -d: -f2"#,
    r#"echo abcdef | cut -c2-4"#,
    r#"echo 'a b c' | awk '{print $2}'"#,
    r#"printf 'a,b\n' | awk -F, '{print $2}'"#,
    r#"echo hi | awk '{print length($0)}'"#,
    r#"printf 'b 2\na 1\n' | sort -k2"#,
    r#"printf 'b\na\nb\n' | sort -u"#,
    r#"printf 'a\na\nb\n' | uniq -d"#,
    r#"printf 'a\nb\n' | xargs echo"#,
    r#"echo hi | tee t2.txt; cat t2.txt"#,
    r#"seq 1 3"#,
    r#"seq 3"#,
    r#"basename /a/b/c.txt"#,
    r#"basename /a/b/c.txt .txt"#,
    r#"dirname /a/b/c.txt"#,
    r#"printf 'a\nb\n' > p1; printf '1\n2\n' > p2; paste p1 p2"#,
    r#"printf hi | base64"#,
    r#"echo aaa | sed 's/a/b/g'"#,
    r#"printf 'l1\nl2\nl3\n' | sed -n '1,2p'"#,
    r#"printf 'a\nb\nc\n' | sed '2d'"#,
    r#"find . -name 'a.txt'"#,
    r#"expr 2 \* 3"#,
    r#"echo abc | rev | rev"#,
    // ── 进阶：xargs -I / sort -t -k / find -exec / paste -d / comm / column -t ──
    r#"printf 'a\nb\n' | xargs -I{} echo "[{}]""#,
    r#"printf 'b:2\na:1\n' | sort -t: -k2"#,
    r#"printf 'b:2\na:1\n' | sort -t: -k1"#,
    r#"find . -name 'a.txt' -exec cat {} \;"#,
    r#"printf 'a\nb\n' > p1; printf '1\n2\n' > p2; paste -d, p1 p2"#,
    r#"printf 'a\nb\n' > c1; printf 'b\nc\n' > c2; comm c1 c2"#,
    r#"printf 'a b\nc d\n' | column -t"#,
    r#"printf 'a\nb\n' > c1; printf 'b\nc\n' > c2; comm -12 c1 c2"#,
    r#"printf 'a\nb\n' > c1; printf 'b\nc\n' > c2; comm -3 c1 c2"#,
    // ── column 多列对齐 / xargs -I 空行·引号 / sort -t 分隔 ──
    r#"printf 'a bb\nccc d\n' | column -t"#,
    r#"printf 'name age\nbob 30\nalice 25\n' | column -t"#,
    r#"printf 'a\n\nb\n' | xargs -I{} echo "[{}]""#,
    r#"printf 'a b\n' | xargs -I{} echo "[{}]""#,
    r#"printf 'x y\nz\n' | xargs -I{} echo "[{}]""#,
    r#"printf 'b,2\na,1\n' | sort -t, -k2"#,
    r#"printf 'b,2\na,1\nc,1\n' | sort -t, -k2 -k1"#,
    // ── column 多列/变宽 / xargs -0 / xargs 引号分组 ──
    r#"printf 'a b c\nddd ee f\n' | column -t"#,
    r#"printf '1 22 333\n4444 5 6\n' | column -t"#,
    r#"printf 'a bb\nccc d\n' | column -t -s ' '"#,
    r#"printf 'a\0b\0' | xargs -0 echo"#,
    r#"printf 'a b\0c\0' | xargs -0 -n1 echo"#,
    r#"printf "'a b'\nc\n" | xargs -n1 echo"#,
    r#"printf "'x y' z\n" | xargs -n1 echo"#,
    // column -o / -s 组合
    r#"printf 'a,bb\nccc,d\n' | column -s, -t"#,
    // xargs -L（每 N 行一条命令）
    r#"printf 'a b\nc\nd\n' | xargs -L1 echo"#,
    r#"printf 'a b\nc\nd\n' | xargs -L2 echo"#,
    // ── B1/B2 大批量：printf 格式 / expr / 归档 / 文件操作 / 校验和 / 文本工具 ──
    r#"printf '%s %s\n' a b"#,
    r#"printf '%-5s|%5s|\n' a b"#,
    r#"printf '%03d\n' 7"#,
    r#"printf '%x\n' 255"#,
    r#"printf '%o\n' 8"#,
    r#"printf '%c\n' abc"#,
    r#"printf '%.2f\n' 3.14159"#,
    r#"printf '%s\n' a b c"#,
    r#"printf 'x\ty\n'"#,
    r#"echo -n hi; echo"#,
    r#"echo -e 'a\tb'"#,
    r#"expr 5 + 3"#,
    r#"expr 10 / 3"#,
    r#"expr 10 % 3"#,
    r#"expr 2 \* 3 + 1"#,
    r#"printf 'abcdef' > t; truncate -s 3 t; cat t"#,
    r#"ln -s target link; readlink link"#,
    r#"printf 'x\n' > f1; cp f1 f2; cat f2"#,
    r#"printf 'x\n' > f3; mv f3 f4; cat f4"#,
    r#"rm -f nope; echo done"#,
    r#"printf 'hello\n' > gf; gzip gf; gunzip gf.gz; cat gf"#,
    r#"printf 'x' > tf; tar cf tf.tar tf; tar tf tf.tar"#,
    r#"printf 'a' > c1; printf 'a' > c2; cmp c1 c2; echo $?"#,
    r#"printf 'abcdef\n' | fold -w 3"#,
    r#"printf 'a\tb\n' | expand"#,
    r#"printf 'abc\n' | md5sum"#,
    r#"printf 'abc' | sha256sum"#,
    r#"x=5; y=$((x*2)); echo $y"#,
    r#"x=; echo "${x:-def}""#,
    r#"x=abc; echo ${x%bc}"#,
    r#"x=abc; echo ${x#a}"#,
    r#"x=a.b.c; echo ${x%.*}"#,
    r#"f() { echo "fn:$1"; }; f hi"#,
    r#"for i in $(seq 1 3); do echo $i; done"#,
    r#"case x in x) echo one;; *) echo other;; esac"#,
    r#"printf 'a\nb\n' | while read l; do echo "-$l"; done"#,
    r#"{ echo a; echo b; }"#,
    r#"(echo sub)"#,
    r#"true && echo y || echo n"#,
    r#"false && echo y || echo n"#,
    r#"echo $((1<2))"#,
    r#"echo $((2>=3))"#,
    r#"echo "a
b""#,
    r#"printf 'line1\nline2\n' | grep -n line"#,
    r#"printf 'a\nb\nc\n' | grep -v b"#,
    r#"printf 'aa\nab\n' | grep '^a'"#,
    r#"printf 'x\ny\n' | grep -c ''"#,
    r#"printf 'ab\ncd\n' | grep -o '.'"#,
    r#"echo 'Hello World' | grep -o 'World'"#,
    r#"printf 'a1\nb2\n' | sed 's/[0-9]/#/g'"#,
    r#"printf 'a\nb\nc\n' | sed -n '2,3p'"#,
    r#"echo 'a b c' | awk '{print $NF}'"#,
    r#"echo 'a b c' | awk '{print NF}'"#,
    r#"printf 'x:1\ny:2\n' | awk -F: '{print $1}'"#,
    r#"printf '1\n2\n3\n' | awk '{s+=$1} END{print s}'"#,
    r#"echo 'a,b,c' | cut -d, -f1,3"#,
    r#"echo 'abcdef' | cut -c2,4"#,
    r#"printf 'c\na\nb\n' | sort"#,
    r#"printf '10\n2\n1\n' | sort -n"#,
    r#"printf 'B\na\nC\n' | sort -f"#,
    r#"printf 'a\na\nb\n' | uniq"#,
    r#"printf 'a\na\nb\n' | uniq -u"#,
    r#"printf 'ABC\n' | tr 'A-Z' 'a-z'"#,
    r#"printf 'a\n\nb\n' | grep -v '^$'"#,
    r#"printf 'x' | base64"#,
    r#"printf 'aGk=' | base64 -d"#,
    r#"printf 'a\nb\n' | head -n 1"#,
    r#"printf 'a\nb\n' | tail -n 1"#,
    r#"printf 'abcdef' | head -c 3"#,
    r#"printf 'abcdef' | tail -c 3"#,
    r#"printf 'a\nb\n' | nl"#,
    r#"printf 'hello' | rev"#,
    // ── `:` no-op builtin / `find -prune` 表达式 ──
    r#": ; echo colon-ok"#,
    r#": > colon.txt; cat colon.txt; echo after"#,
    r#": && echo and-ok"#,
    r#"find . -path ./sub -prune -o -name '*.txt' -print | sort"#,
    r#"find . -name '*.log' -prune -o -type f -print | sort"#,
    // ── 参数展开 / 位置参数 / 数组 / [[ ]] / (( )) / $'...' ──
    r#"v=abc; echo "${v:-def}"; echo "${#v}"; echo "${v%c}"; echo "${v#a}""#,
    r#"v=abcabc; echo "${v/a/X}"; echo "${v//a/X}""#,
    r#"v=abcdef; echo "${v:2:3}""#,
    r#"set -- a b c; echo "$#"; echo "$1"; echo "$*""#,
    r#"set -- a b c; for x in "$@"; do echo "<$x>"; done"#,
    r#"set -- a b c; shift; echo "$1"; echo "$#""#,
    r#"a=(x y z); echo "${a[1]}"; echo "${#a[@]}"; echo "${a[@]}""#,
    r#"a=(x y z); for e in "${a[@]}"; do echo "<$e>"; done"#,
    r#"i=5; ((i++)); echo $i"#,
    r#"i=5; ((i+=3)); echo $i"#,
    r#"if [[ -f a.txt ]]; then echo y; fi"#,
    r#"if [[ abc =~ ^a.c$ ]]; then echo m; fi"#,
    r#"if [[ "a b" == *" "* ]]; then echo sp; fi"#,
    r#"if [[ 3 -gt 2 && -f a.txt ]]; then echo both; fi"#,
    r#"if [[ ! -f nope ]]; then echo nofile; fi"#,
    r#"for i in 1 2 3; do if [ $i -eq 2 ]; then break; fi; echo $i; done"#,
    r#"for i in 1 2 3; do if [ $i -eq 2 ]; then continue; fi; echo $i; done"#,
    r#"f(){ return 3; }; f; echo $?"#,
    r#"(exit 3); echo $?"#,
    r#"unset v; f(){ local v=1; echo $v; }; f; echo "[$v]""#,
    r#"base64 <<< "hi""#,
    r#"expr abc : 'a.c'"#,
    r#"printf '%d\n' 0x1f"#,
    r#"ls nope 2>/dev/null; echo X"#,
    r#"echo E >&2; echo X"#,
    // 需要 bash>=4 的现代特性（oracle 已是 bash 5.2）
    r#"v=abc; echo "${v^^}""#,
    r#"v=ABC; echo "${v,,}""#,
    r#"v=abcdef; echo "${v: -2}""#,
    // ── 拼接赋值 / += 追加 / 命令替换拼接 ──
    r#"s=x; s=${s}$(printf y); echo $s"#,
    r#"s=x; s=${s}y; echo $s"#,
    r#"s=x; s="$s$(printf y)"; echo $s"#,
    r#"s=$(printf a); s=${s}$(printf b); echo [$s]"#,
    r#"s=x; s+=$(printf y); echo $s"#,
    r#"s=x; s+=y; echo $s"#,
    r#"s=; s=${s}a; s=${s}b; echo $s"#,
    r#"a=1; b=2; s=$a$b; echo $s"#,
    r#"s=pre; s=${s}$(echo mid)post; echo $s"#,
    r#"s=$(printf a)$(printf b); echo $s"#,
    r#"s=x; s=${s}${s}; echo $s"#,
    r#"s=z; s=$s-$s; echo [$s]"#,
    r#"s=x; s=${s} y; echo $s"#,
];

#[test]
#[ignore = "heavy: run with --ignored (differential vs bash)"]
fn fastshell_matches_bash_on_shell_semantics() {
    if !bash_available() {
        eprintln!("bash not found; skipping diff test");
        return;
    }
    let (sdk, sh_dir) = setup();
    let (allow_always, allow_bash3) = common::load_allowlist();
    let major = common::bash_major(&common::oracle_bash());
    let mut failures = Vec::new();
    for cmd in CASES {
        // Central platform/format allowlist (see tests/known_divergences.txt).
        if allow_always.iter().any(|a| cmd.contains(a.as_str()))
            || (major < 4 && allow_bash3.iter().any(|a| cmd.contains(a.as_str())))
        {
            continue;
        }
        // fastshell persists cwd across execute() calls; bash here runs each
        // command in a fresh cwd. Reset to the sandbox root so a `cd` case
        // cannot leak into the next one.
        let _ = sdk.execute("cd /");
        let f = sdk.execute(cmd);
        let (b_out, b_code) = bash_run(&sh_dir, cmd);
        let (f_out, f_code) = (norm(&f.stdout), f.exit_code);
        let b_out = norm(&b_out);
        if f_out != b_out || f_code != b_code {
            failures.push(format!(
                "cmd: {cmd}\n  fastshell: exit={f_code} stdout={f_out:?}\n  bash:      exit={b_code} stdout={b_out:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "fastshell != bash for {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
