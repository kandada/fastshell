// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Dedicated tests for the "long tail + backbone" hardening push:
//! extra commands, shell builtins, arrays, variable attributes, job control.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn sdk() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_longtail_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
        ..Default::default()
    })
    .unwrap();
    s
}

fn run(s: &Fastshell, cmd: &str) -> (String, String, i32) {
    let r = s.execute(cmd);
    (r.stdout, r.stderr, r.exit_code)
}

fn out(s: &Fastshell, cmd: &str) -> String {
    let (o, e, rc) = run(s, cmd);
    assert_eq!(rc, 0, "cmd={cmd:?} err={e}");
    o
}

// ── new commands ───────────────────────────────────────────────────────────

#[test]
fn hash_and_encoding_extras() {
    let s = sdk();
    assert_eq!(
        out(&s, "printf abc | sha224sum")
            .split_whitespace()
            .next()
            .unwrap(),
        "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7"
    );
    assert_eq!(
        out(&s, "printf abc | sha384sum").split_whitespace().next().unwrap(),
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
    );
    assert_eq!(out(&s, "printf hello | base32"), "NBSWY3DP\n");
    assert_eq!(out(&s, "printf 'NBUQ====' | base32 -d"), "hi");
    assert_eq!(
        out(&s, "printf 'hello world' | base32 | base32 -d"),
        "hello world"
    );
}

#[test]
fn misc_new_commands() {
    let s = sdk();
    assert!(!out(&s, "arch").trim().is_empty());
    assert_eq!(out(&s, "factor 12"), "12: 2 2 3\n");
    assert_eq!(out(&s, "factor 100"), "100: 2 2 5 5\n");
    assert_eq!(out(&s, "factor 97"), "97: 97\n");
    assert_eq!(out(&s, "numfmt 1000"), "1.0K\n");
    assert_eq!(out(&s, "numfmt 1500000"), "1.5M\n");
    assert_eq!(out(&s, "numfmt --to=iec 2048"), "2.0Ki\n");
    assert_eq!(
        out(&s, "printf 'l1\\nl2\\n' | pr -t -n"),
        "    1\tl1\n    2\tl2\n"
    );
}

// ── shell builtins ─────────────────────────────────────────────────────────

#[test]
fn dir_stack_and_umask_and_ulimit() {
    let s = sdk();
    out(&s, "mkdir -p a b");
    assert_eq!(out(&s, "cad=/a; umask"), "0022\n");
    assert_eq!(out(&s, "umask 077; umask"), "0077\n");
    assert_eq!(out(&s, "ulimit -n"), "1024\n");
    assert!(out(&s, "ulimit -a").contains("open files"));
    assert_eq!(out(&s, "cd /a; pushd /b >/dev/null; pwd"), "/b\n");
    assert_eq!(out(&s, "dirs"), "/b /a\n");
    assert_eq!(out(&s, "popd >/dev/null; pwd"), "/a\n");
}

#[test]
fn builtin_hash_and_shopt() {
    let s = sdk();
    assert_eq!(out(&s, "builtin echo hi"), "hi\n");
    assert_eq!(out(&s, "hash -r; echo rc=$?"), "rc=0\n");
    // nullglob
    assert_eq!(
        out(&s, "shopt -s nullglob; printf '[%s]\\n' *.nomatch"),
        "[]\n"
    );
    assert_eq!(out(&s, "shopt -q nullglob; echo $?"), "0\n");
    assert_eq!(
        out(&s, "shopt -u nullglob; printf '[%s]\\n' *.nomatch"),
        "[*.nomatch]\n"
    );
    // set -f (noglob)
    assert_eq!(
        out(&s, "set -f; printf '[%s]\\n' *.nomatch"),
        "[*.nomatch]\n"
    );
    // glob matches when enabled (clear noglob first)
    out(&s, "set +f; mkdir -p d && touch d/a.txt d/b.txt");
    assert_eq!(
        out(&s, "shopt -s nullglob; echo d/*.txt"),
        "d/a.txt d/b.txt\n"
    );
}

#[test]
fn readonly_and_integer_attributes() {
    let s = sdk();
    assert_eq!(out(&s, "readonly R=1; echo $R"), "1\n");
    let (o, e, rc) = run(&s, "R=2");
    assert_ne!(rc, 0);
    assert!(e.contains("readonly"), "stderr={e:?}");
    assert_eq!(out(&s, "echo $R"), "1\n");
    assert_eq!(out(&s, "declare -i n; n=3+4; echo $n"), "7\n");
    assert_eq!(out(&s, "declare -p n"), "declare -i n=\"7\"\n");
}

#[test]
fn trap_exit_runs() {
    let s = sdk();
    assert_eq!(out(&s, "trap 'echo BYE' EXIT; echo hi"), "hi\nBYE\n");
}

// ── arrays & jobs ──────────────────────────────────────────────────────────

#[test]
fn array_operations() {
    let s = sdk();
    assert_eq!(out(&s, "a=(x y z); echo ${a[@]:1}"), "y z\n");
    assert_eq!(out(&s, "a=(x y z); echo ${a[@]:1:1}"), "y\n");
    assert_eq!(out(&s, "a=(x y z); echo ${!a[@]}"), "0 1 2\n");
    assert_eq!(
        out(&s, "arr=(a b); arr+=(c d); echo ${arr[@]}"),
        "a b c d\n"
    );
    assert_eq!(
        out(&s, "arr=(a b c); unset 'arr[1]'; echo [${arr[@]}]"),
        "[a c]\n"
    );
    assert_eq!(
        out(&s, "read -a r <<< 'p q s'; echo ${#r[@]} ${r[1]}"),
        "3 q\n"
    );
    out(&s, "mkdir -p /dd && printf 'a\\nb\\n' > /dd/f.txt");
    assert_eq!(
        out(
            &s,
            "mapfile -t lines < /dd/f.txt; echo n=${#lines[@]} first=${lines[0]}"
        ),
        "n=2 first=a\n"
    );
    assert_eq!(
        out(&s, "declare -A m; m[foo]=1; m[bar]=2; echo ${!m[@]}"),
        "bar foo\n"
    );
}

#[test]
fn background_jobs() {
    let s = sdk();
    let o = out(&s, "sleep 0.01 & echo pid=$!");
    assert!(o.starts_with("pid="), "output={o:?}");
    assert!(
        !o.trim().ends_with("pid="),
        "pid should be non-empty: {o:?}"
    );
    assert!(out(&s, "true & wait; echo done").contains("done"));
    assert!(out(&s, "jobs").contains("Done"));
}

// ── deep glob options ──────────────────────────────────────────────────────

#[test]
fn glob_deep_options() {
    let s = sdk();
    out(&s, "mkdir -p /g && cd /g && touch a.txt b.log AB.TXT .hidden c1 c2 && mkdir -p sub && touch sub/x.txt");
    assert_eq!(out(&s, "cd /g && echo *.txt"), "a.txt\n");
    assert_eq!(
        out(&s, "cd /g && shopt -s nocaseglob; echo *.txt"),
        "AB.TXT a.txt\n"
    );
    assert_eq!(
        out(
            &s,
            "cd /g && shopt -u nocaseglob; shopt -s dotglob; echo .hidden"
        ),
        ".hidden\n"
    );
    assert_eq!(
        out(
            &s,
            "cd /g && shopt -u dotglob; shopt -s extglob; echo @(a.txt|b.log)"
        ),
        "a.txt b.log\n"
    );
    assert_eq!(out(&s, "cd /g && echo ?(a).txt"), "a.txt\n");
    assert_eq!(out(&s, "cd /g && echo +(c[12])"), "c1 c2\n");
    assert_eq!(
        out(&s, "cd /g && echo !(a.txt)"),
        "AB.TXT b.log c1 c2 sub\n"
    );
    assert_eq!(
        out(&s, "cd /g && shopt -s globstar; echo **/*.txt"),
        "a.txt sub/x.txt\n"
    );
    let (o, e, rc) = run(&s, "cd /g && shopt -s failglob; echo *.nomatch");
    assert_eq!(o, "");
    assert_ne!(rc, 0);
    assert!(e.contains("no match"), "stderr={e:?}");
}

// ── find -size / sed step addresses ────────────────────────────────────────

#[test]
fn find_size_and_sed_step() {
    let s = sdk();
    out(&s, "cd / && mkdir -p t && cd t && touch empty.txt && echo abc > big.txt && printf '1\\n2\\n3\\n4\\n5\\n6\\n' > n.txt");
    assert_eq!(
        out(&s, "cd /t && find . -type f -size +1c"),
        "./big.txt\n./n.txt\n"
    );
    assert_eq!(
        out(&s, "cd /t && find . -type f -size -1c"),
        "./empty.txt\n"
    );
    assert_eq!(out(&s, "cd /t && find . -type f -size 0c"), "./empty.txt\n");
    assert_eq!(out(&s, "cd /t && sed -n '1~2p' n.txt"), "1\n3\n5\n");
    assert_eq!(out(&s, "cd /t && sed -n '0~3p' n.txt"), "3\n6\n");
}

// ── jobs: kill %n / disown / fg / bg ───────────────────────────────────────

#[test]
fn jobs_kill_disown() {
    let s = sdk();
    out(&s, "sleep 0.01 & sleep 0.01 &");
    assert!(out(&s, "jobs").contains("[2] Done"), "jobs listing");
    assert_eq!(
        out(&s, "kill %1; jobs"),
        "[1] Killed sleep 0.01\n[2] Done sleep 0.01\n"
    );
    assert_eq!(out(&s, "disown; jobs"), "");
    let (_o, _e, rc) = run(&s, "fg");
    assert_ne!(rc, 0);
    let (_o, _e, rc) = run(&s, "bg");
    assert_ne!(rc, 0);
}

// ── pip show / uninstall ───────────────────────────────────────────────────

#[test]
fn pip_show_and_uninstall() {
    let s = sdk();
    out(&s, "mkdir -p site-packages/six-1.17.0.dist-info && printf 'Name: six\\nVersion: 1.17.0\\nSummary: compat\\n' > site-packages/six-1.17.0.dist-info/METADATA && echo x=1 > site-packages/six.py");
    let show = out(&s, "pip show six");
    assert!(
        show.contains("Name: six") && show.contains("Version: 1.17.0"),
        "{show:?}"
    );
    assert_eq!(out(&s, "pip list"), "six\n");
    assert_eq!(
        out(&s, "pip uninstall six"),
        "Successfully uninstalled six\n"
    );
    let (_o, e, rc) = run(&s, "pip show six");
    assert_ne!(rc, 0);
    assert!(e.contains("not found"), "{e:?}");
}

// ── awk arrays / bc bases / b2sum / zstd / sed -z ──────────────────────────

#[test]
fn awk_split_and_array_elements() {
    let s = sdk();
    assert_eq!(
        out(
            &s,
            "awk 'BEGIN{split(\"a b c\",arr,\" \"); print arr[1], arr[3]}'"
        ),
        "a c\n"
    );
    assert_eq!(
        out(&s, "awk 'BEGIN{n=split(\"x,y,z\",a,\",\"); print n, a[2]}'"),
        "3 y\n"
    );
    assert_eq!(
        out(
            &s,
            "printf 'one two\\nthree four\\n' | awk '{split($0,w); print w[1]\"-\"w[2]}'"
        ),
        "one-two\nthree-four\n"
    );
}

#[test]
fn bc_bases() {
    let s = sdk();
    assert_eq!(out(&s, "echo 'obase=16; 255' | bc"), "FF\n");
    assert_eq!(out(&s, "echo 'obase=2; 5' | bc"), "101\n");
    assert_eq!(out(&s, "echo 'ibase=2; 101' | bc"), "5\n");
    assert_eq!(out(&s, "echo 'scale=2; 10/3' | bc"), "3.33\n");
}

#[test]
fn b2sum_and_zstd_and_sed_null() {
    let s = sdk();
    let o = out(&s, "printf abc | b2sum");
    assert!(
        o.starts_with("ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d1"),
        "b2sum={o:?}"
    );
    assert_eq!(
        out(&s, "echo 'hello zstd' | zstd -c | zstd -dc"),
        "hello zstd\n"
    );
    assert_eq!(
        out(&s, "echo 'roundtrip' | zstd -c | zstdcat"),
        "roundtrip\n"
    );
    // sed -z: NUL-separated records.
    assert_eq!(
        out(&s, "printf 'a\\0b\\0c' | sed -z 's/b/X/' | tr '\\0' '/'"),
        "a/X/c/"
    );
}

#[test]
fn cpio_newc_roundtrip() {
    let s = sdk();
    out(&s, "cd / && mkdir -p d && echo hi > d/a.txt");
    out(&s, "cd / && printf 'd\\nd/a.txt\\n' | cpio -o > out.cpio");
    assert_eq!(out(&s, "cd / && cpio -it < out.cpio"), "d\nd/a.txt\n");
    out(&s, "cd / && rm -rf d");
    out(&s, "cd / && cpio -id < out.cpio");
    assert_eq!(out(&s, "cd / && cat d/a.txt"), "hi\n");
}

#[test]
fn dialect_aliases_and_new_commands() {
    let s = sdk();
    out(&s, "mkdir -p /dd && echo hi > /dd/f.txt");
    assert_eq!(out(&s, "dir /dd"), "f.txt\n");
    assert!(out(&s, "ll /dd").contains("f.txt"), "ll output");
    assert_eq!(out(&s, "unlink /dd/f.txt; ls /dd"), "");
    out(&s, "echo x > /a.txt");
    assert_eq!(out(&s, "link /a.txt /b.txt; cat /b.txt"), "x\n");
    assert_eq!(out(&s, "FOO=bar; echo 'v=$FOO' | envsubst"), "v=bar\n");
    assert!(out(&s, "ver").contains("fastshell"));
    assert!(out(&s, "whereis ls").contains("ls"));
}

// ── session-1790432672 / -79433082 fixes ───────────────────────────────────

#[test]
fn tar_auto_detect_and_pipe_subshell() {
    let s = sdk();
    out(
        &s,
        "cd / && mkdir -p ar && echo x > ar/x.txt && echo y > ar/y.txt",
    );
    // compressed archive: tf/xf auto-detect (no -z), and -C on extract.
    out(&s, "cd / && tar czf t.tgz ar");
    assert_eq!(out(&s, "cd / && tar tf t.tgz"), "ar/x.txt\nar/y.txt\n");
    out(&s, "cd / && mkdir -p ex && tar xf t.tgz -C ex");
    assert_eq!(out(&s, "cd / && cat ex/ar/x.txt"), "x\n");
    out(&s, "cd / && tar cJf t.xz ar && tar tf t.xz");
    assert_eq!(out(&s, "cd / && tar tf t.xz"), "ar/x.txt\nar/y.txt\n");
    // PRODUCER | ( read ...; body )
    assert_eq!(out(&s, "echo hi | ( read a; echo \"a=[$a]\" )"), "a=[hi]\n");
    assert_eq!(
        out(&s, "printf 'x\\ny\\n' | ( read b; echo \"b=$b\" )"),
        "b=x\n"
    );
}

#[test]
fn random_and_unzip_dir() {
    let s = sdk();
    let v = out(&s, "echo $RANDOM");
    let n: i64 = v.trim().parse().expect("numeric $RANDOM");
    assert!((0..32768).contains(&n), "RANDOM={n}");
    out(&s, "cd / && echo hi > u.txt && zip -q u.zip u.txt");
    out(&s, "cd / && mkdir -p uz && unzip -o -q -d uz u.zip");
    assert_eq!(out(&s, "cd / && cat uz/u.txt"), "hi\n");
}

// ── batch A/B: find-perm, trap ERR, nameref, awk funcs/getline ─────────────

#[test]
fn find_perm_and_newermt() {
    let s = sdk();
    out(
        &s,
        "cd / && mkdir -p p && echo x > p/a.txt && chmod 644 p/a.txt",
    );
    assert_eq!(out(&s, "cd / && find p -perm 644"), "p/a.txt\n");
    assert!(out(&s, "cd / && find p -perm -600").contains("p/a.txt"));
    assert_eq!(
        out(
            &s,
            "cd / && find p -newermt 2000-01-01 | sort | tr '\\n' ' '"
        ),
        "p p/a.txt "
    );
}

#[test]
fn trap_err_and_nameref() {
    let s = sdk();
    assert_eq!(
        out(&s, "trap 'echo ERRD' ERR; false; echo done"),
        "ERRD\ndone\n"
    );
    assert_eq!(
        out(&s, "trap 'echo ERRD' ERR; false && echo no; echo done"),
        "done\n"
    );
    assert_eq!(
        out(
            &s,
            "target=hello; declare -n ref=target; echo $ref; ref=world; echo $target"
        ),
        "hello\nworld\n"
    );
}

#[test]
fn awk_user_functions_and_getline() {
    let s = sdk();
    assert_eq!(
        out(&s, "awk 'function f(x){return x*2} BEGIN{print f(3)}'"),
        "6\n"
    );
    assert_eq!(
        out(
            &s,
            "awk 'function add(a,b){return a+b} BEGIN{print add(2,3)}'"
        ),
        "5\n"
    );
    assert_eq!(
        out(
            &s,
            "awk 'function up(s){return toupper(s)} BEGIN{print up(\"hi\")}'"
        ),
        "HI\n"
    );
    assert_eq!(
        out(
            &s,
            "awk 'function fib(n){if(n<2)return n; return fib(n-1)+fib(n-2)} BEGIN{print fib(10)}'"
        ),
        "55\n"
    );
    out(&s, "cd / && printf 'l1\\nl2\\n' > g.txt");
    assert_eq!(
        out(
            &s,
            "cd / && awk 'BEGIN{while((getline line < \"g.txt\")>0) print \"got:\" line}'"
        ),
        "got:l1\ngot:l2\n"
    );
}

// ── sed full interpreter + awk main-input getline ──────────────────────────

#[test]
fn sed_full_interpreter() {
    let s = sdk();
    assert_eq!(out(&s, "printf 'a\\nb\\nc\\n' | sed 'n;d'"), "a\nc\n");
    assert_eq!(
        out(&s, "printf '1\\n2\\n3\\n4\\n' | sed 'N;s/\\n/-/'"),
        "1-2\n3-4\n"
    );
    assert_eq!(out(&s, "printf 'a\\nb\\nc\\n' | sed -n '2{p;q}'"), "b\n");
    assert_eq!(out(&s, "printf 'aaa\\n' | sed ':x; s/a/b/; tx'"), "bbb\n");
    assert_eq!(out(&s, "printf '1\\n2\\n' | sed 'N;P;D'"), "1\n2\n");
    assert_eq!(out(&s, "printf 'a\\nb\\n' | sed -n '1h;2{G;p}'"), "b\na\n");
    assert_eq!(
        out(&s, "printf '1\\n2\\n3\\n4\\n' | sed -n '/2/,/3/p'"),
        "2\n3\n"
    );
    assert_eq!(out(&s, "printf '1\\n2\\n3\\n4\\n' | sed '2,3d'"), "1\n4\n");
}

#[test]
fn awk_main_input_getline() {
    let s = sdk();
    assert_eq!(
        out(&s, "printf 'p\\nq\\n' | awk 'NR==1{while((getline v)>0) print \"v=\" v; print \"after:\" $0}'"),
        "v=q\nafter:p\n"
    );
}

// ── round: tar -C relative, /dev devices, numfmt stdin, help coverage ──────

#[test]
fn tar_dash_c_relative_dir_roundtrips() {
    let s = sdk();
    let o = out(
        &s,
        "mkdir -p /arc; echo A > /arc/f.txt; cd /; \
tar czf /t.tgz -C arc f.txt; mkdir -p /un2; tar xzf /t.tgz -C un2; cat /un2/f.txt",
    );
    assert_eq!(o, "A\n", "tar -C relative dir must round-trip");
}

#[test]
fn pseudo_devices_and_numfmt_stdin() {
    let s = sdk();
    assert_eq!(
        out(&s, "sha256sum /dev/null | cut -d' ' -f1"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n"
    );
    assert_eq!(out(&s, "cat /dev/null").len(), 0);
    assert_eq!(
        out(&s, "printf '1000\\n2000000\\n' | numfmt --to=si"),
        "1.0K\n2.0M\n"
    );
}

#[test]
fn newly_added_commands_are_discoverable() {
    let s = sdk();
    for c in [
        "mapfile",
        "readarray",
        "zstd",
        "b2sum",
        "cpio",
        "numfmt",
        "arch",
    ] {
        let o = out(&s, &format!("type {c}"));
        assert!(o.contains("built-in"), "type {c} => {o:?}");
    }
}

#[test]
fn unzip_dash_d_option_order() {
    let s = sdk();
    let o = out(
        &s,
        "mkdir -p /arc; echo Z > /arc/z.txt; cd /arc; zip -q /z.zip z.txt; cd /; \
mkdir -p /d1 /d2; unzip -o -d d1 /z.zip >/dev/null 2>&1; unzip -o /z.zip -d d2 >/dev/null 2>&1; \
echo \"$(ls d1)|$(ls d2)\"",
    );
    assert_eq!(
        o, "z.txt|z.txt\n",
        "unzip -d must work before and after the archive"
    );
}

#[test]
fn bounded_binary_sampling_commands() {
    let s = sdk();
    out(&s, "printf 'ABCDEFGHIJ' > /b.bin");
    let xxd = out(&s, "xxd -l 4 /b.bin");
    assert!(xxd.contains("4142 4344"), "xxd -l: {xxd:?}");
    assert_eq!(out(&s, "head -c 4 /b.bin"), "ABCD");
    let od = out(&s, "od -An -N 4 -tx1 /b.bin");
    assert!(od.contains("41 42 43 44"), "od -N: {od:?}");
    let x2 = out(&s, "xxd -s 4 -l 2 -p /b.bin");
    assert!(x2.contains("4546"), "xxd -s -p: {x2:?}");
}

#[test]
fn gnu_alignment_round() {
    let s = sdk();
    // sort -k inline modifiers (numeric on key)
    let o = out(&s, "printf 'b 2\\na 10\\nc 1\\n' | sort -k2n");
    assert_eq!(o, "c 1\nb 2\na 10\n", "sort -k2n");
    let o = out(&s, "printf 'b 2\\na 10\\nc 1\\n' | sort -k2,2n");
    assert_eq!(o, "c 1\nb 2\na 10\n", "sort -k2,2n");
    // readlink -f resolves a VFS symlink
    let o = out(&s, "echo t > /t.txt; ln -s /t.txt /ln; readlink -f /ln");
    assert_eq!(o, "/t.txt\n", "readlink -f");
    // realpath is VFS-relative (no host leak) and -m allows missing
    assert_eq!(out(&s, "echo x > /rp.txt; realpath /rp.txt"), "/rp.txt\n");
    assert_eq!(out(&s, "realpath -m /no/such/file"), "/no/such/file\n");
    // mktemp returns a VFS path
    assert!(out(&s, "mktemp -d").starts_with("/tmp/"));
    // rm -d removes an empty dir
    let o = out(&s, "mkdir /ed; rm -d /ed; ls /ed 2>&1; echo rc=$?");
    assert!(o.contains("rc=1"), "rm -d: {o:?}");
    // ls -S sorts largest first
    let o = out(
        &s,
        "mkdir /SD; : > /SD/a; head -c 100 /dev/zero > /SD/big; ls -1S /SD | head -1",
    );
    assert_eq!(o, "big\n", "ls -S");
    // sed -i.bak creates a backup
    let o = out(
        &s,
        "printf 'a\\n' > /sf.txt; sed -i.bak 's/a/b/' /sf.txt; cat /sf.txt; cat /sf.txt.bak",
    );
    assert_eq!(o, "b\na\n", "sed -i.bak");
    // tar -O extracts to stdout
    let o = out(
        &s,
        "mkdir -p /TO; echo hi > /TO/f; tar cf /to.tar -C /TO f; tar xf /to.tar -O f",
    );
    assert_eq!(o, "hi\n", "tar -O");
}

#[test]
fn awk_fs_ofs_ors_alignment() {
    let s = sdk();
    assert_eq!(out(&s, "echo 'a:b:c' | awk -F: '{print $2}'"), "b\n");
    assert_eq!(
        out(&s, "echo 'a:b:c' | awk 'BEGIN{FS=\":\"}{print $2}'"),
        "b\n"
    );
    assert_eq!(out(&s, "echo 'a1b2c' | awk -F'[0-9]' '{print $2}'"), "b\n");
    assert_eq!(
        out(
            &s,
            "printf 'a b c\\n' | awk 'BEGIN{OFS=\"-\"}{print $1,$2,$3}'"
        ),
        "a-b-c\n"
    );
    assert_eq!(
        out(&s, "printf 'a\\nb\\n' | awk 'BEGIN{ORS=\";\"}{print}'"),
        "a;b;"
    );
}

#[test]
fn awk_associative_split_match() {
    let s = sdk();
    // associative arrays + for-in (classic word count)
    assert_eq!(
        out(
            &s,
            "printf 'a\\nb\\na\\nc\\na\\n' | awk '{c[$1]++} END{for(k in c) print k, c[k]}' | sort"
        ),
        "a 3\nb 1\nc 1\n"
    );
    // string-valued assoc + numeric op
    assert_eq!(
        out(
            &s,
            "printf 'a 1\\nb 2\\n' | awk '{t[$1]=$2} END{print t[\"a\"]+t[\"b\"]}'"
        ),
        "3\n"
    );
    // split() returns count AND populates the array
    assert_eq!(
        out(
            &s,
            "echo 'a:b:c' | awk '{n=split($0,a,\":\"); print n, a[1], a[3]}'"
        ),
        "3 a c\n"
    );
    // match() returns 1-based position
    assert_eq!(
        out(&s, "echo abc123 | awk '{print match($0,/[0-9]+/)}'"),
        "4\n"
    );
    // numeric-keyed assoc
    assert_eq!(
        out(
            &s,
            "printf 'x\\ny\\n' | awk '{a[NR]=$0} END{print a[1], a[2]}'"
        ),
        "x y\n"
    );
}

#[test]
fn sed_read_file_command() {
    let s = sdk();
    let o = out(
        &s,
        "printf 'X\\nY\\n' > /rw; printf 'a\\nb\\nc\\n' | sed '1r /rw'",
    );
    assert_eq!(o, "a\nX\nY\nb\nc\n");
}

#[test]
fn pip_and_python_alias_routing() {
    let s = sdk();
    // pip sub-commands and aliases route to the native pip implementation.
    assert!(out(&s, "pip --help").contains("pip install"));
    assert!(out(&s, "pip3 --help").contains("pip install"));
    assert!(out(&s, "python3 -m pip --help").contains("pip install"));
    assert!(out(&s, "pip list").contains("no packages installed"));
}
