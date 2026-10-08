// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Broad Python execution compatibility/stability coverage: language features,
//! stdlib, sandbox confinement, argv/stdin, exit codes, and the py2 dialect.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup_dir() -> (Fastshell, std::path::PathBuf) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_pycompat_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 0,
        ..Default::default()
    })
    .unwrap();
    (sdk, dir)
}

fn setup() -> Fastshell {
    setup_dir().0
}

fn py(s: &Fastshell, code: &str) -> String {
    let r = s.execute_python(code);
    assert!(
        r.is_success(),
        "python failed ({}) code={code}\nstderr={}",
        r.exit_code,
        r.stderr
    );
    r.stdout
}

#[test]
fn py_language_core() {
    let s = setup();
    let out = py(
        &s,
        r#"
xs = [x*x for x in range(5) if x % 2 == 0]
d = {k: v for k, v in zip('ab', [1, 2])}
a, *rest = [1, 2, 3, 4]
print(xs, d, a, rest)
def f(a, b=2, *args, **kw): return (a, b, args, sorted(kw.items()))
print(f(1, 3, 4, z=5))
sq = lambda n: n*n
print([sq(i) for i in range(3)])
"#,
    );
    assert!(out.contains("[0, 4, 16]"), "{out}");
    assert!(out.contains("{'a': 1, 'b': 2}"), "{out}");
    assert!(out.contains("1 [2, 3, 4]"), "{out}");
    assert!(out.contains("[0, 1, 4]"), "{out}");
}

#[test]
fn py_fstrings_and_unicode() {
    let s = setup();
    let out = py(
        &s,
        r#"
name = "世界"
print(f"hi {name} {2+3} {'x'*3}")
print("café".upper(), "汉字".encode('utf-8').hex())
print("{:>5}|{:.2f}".format(42, 3.14159))
"#,
    );
    assert!(out.contains("hi 世界 5 xxx"), "{out}");
    assert!(out.contains("CAFÉ"), "{out}");
    assert!(out.contains("   42|3.14"), "{out}");
}

#[test]
fn py_generators_decorators_classes() {
    let s = setup();
    let out = py(
        &s,
        r#"
def gen(n):
    for i in range(n):
        yield i
print(list(gen(3)))
def deco(fn):
    def w(*a, **k):
        return fn(*a, **k) * 2
    return w
@deco
def add(a, b): return a + b
print(add(2, 3))
class Base:
    def hi(self): return "base"
class Child(Base):
    def hi(self): return super().hi() + "+child"
print(Child().hi())
from dataclasses import dataclass
@dataclass
class P:
    x: int
    y: int
print(P(1, 2))
"#,
    );
    assert!(out.contains("[0, 1, 2]"), "{out}");
    assert!(out.contains("10"), "{out}");
    assert!(out.contains("base+child"), "{out}");
    assert!(out.contains("P(x=1, y=2)"), "{out}");
}

#[test]
fn py_exceptions_context_managers() {
    let s = setup();
    let out = py(
        &s,
        r#"
try:
    raise ValueError("boom")
except ValueError as e:
    print("caught", e)
finally:
    print("finally")
import contextlib
@contextlib.contextmanager
def ctx():
    print("enter"); yield 7; print("exit")
with ctx() as v:
    print("inside", v)
class MyErr(Exception): pass
try:
    raise MyErr("custom")
except MyErr:
    print("custom ok")
"#,
    );
    assert!(out.contains("caught boom"), "{out}");
    assert!(out.contains("finally"), "{out}");
    assert!(out.contains("inside 7"), "{out}");
    assert!(out.contains("custom ok"), "{out}");
}

#[test]
fn py_stdlib_json_re_datetime_collections() {
    let s = setup();
    let out = py(
        &s,
        r#"
import json, re, datetime, math, collections, itertools, functools, hashlib, base64
print(json.dumps({"a": [1, 2]}, sort_keys=True))
print(re.findall(r'\d+', 'a1b22c333'))
print(datetime.date(2024, 2, 29).isoformat())
print(round(math.pi, 3))
print(collections.Counter('aab').most_common(1))
print(list(itertools.islice(itertools.count(1), 3)))
print(functools.reduce(lambda a, b: a + b, [1, 2, 3]))
print(hashlib.md5(b'abc').hexdigest())
print(base64.b64encode(b'hi').decode())
"#,
    );
    assert!(out.contains(r#"{"a": [1, 2]}"#), "{out}");
    assert!(out.contains("['1', '22', '333']"), "{out}");
    assert!(out.contains("2024-02-29"), "{out}");
    assert!(out.contains("3.142"), "{out}");
    assert!(out.contains("6"), "{out}");
    assert!(out.contains("900150983cd24fb0d6963f7d28e17f72"), "{out}");
    assert!(out.contains("aGk="), "{out}");
}

#[test]
fn py_file_io_sandbox() {
    let s = setup();
    let out = py(
        &s,
        r#"
import os, json, glob
with open("data.txt", "w") as f:
    f.write("hello\nworld\n")
print(open("data.txt").read().splitlines())
os.makedirs("sub/dir", exist_ok=True)
open("sub/dir/x.txt", "w").write("x")
print(sorted(os.listdir("sub/dir")))
print(sorted(glob.glob("sub/**/*.txt", recursive=True)))
print(os.path.exists("data.txt"), os.path.getsize("data.txt"))
os.remove("data.txt")
print(os.path.exists("data.txt"))
"#,
    );
    assert!(out.contains("['hello', 'world']"), "{out}");
    assert!(out.contains("['x.txt']"), "{out}");
    assert!(out.contains("True 12"), "{out}");
    assert!(out.contains("False"), "{out}");
}

#[test]
fn py_sandbox_escape_builtins_open() {
    let (s, root) = setup_dir();
    // `../` must not escape the sandbox.
    py(&s, r#"open("../escape.txt", "w").write("leak")"#);
    assert!(
        !root.parent().unwrap().join("escape.txt").exists(),
        "builtins.open escaped the sandbox"
    );
}

#[test]
fn py_sandbox_pathlib_and_io() {
    let s = setup();
    // pathlib / io.open must also be confined.
    let out = py(
        &s,
        r#"
import pathlib, io
pathlib.Path("p.txt").write_text("ok")
print(pathlib.Path("p.txt").read_text())
with io.open("io.txt", "w") as f:
    f.write("io")
print(open("io.txt").read())
"#,
    );
    assert!(out.contains("ok"), "{out}");
    assert!(out.contains("io"), "{out}");
}

#[test]
fn py_argv_and_script() {
    let s = setup();
    s.write_file(
        "args.py",
        "import sys\nprint(sys.argv[1:], sys.argv[0].endswith('args.py'))\n",
    )
    .unwrap();
    let r = s.execute("python3 args.py a b c");
    assert!(r.is_success(), "stderr={}", r.stderr);
    assert!(r.stdout.contains("['a', 'b', 'c'] True"), "{}", r.stdout);
}

#[test]
fn py_stdin_pipe() {
    let s = setup();
    let r = s.execute(
        r#"echo hello | python3 -c "import sys; print(sys.stdin.read().strip().upper())""#,
    );
    assert!(r.is_success(), "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "HELLO", "stdout={}", r.stdout);
}

#[test]
fn py_exit_code_and_stderr() {
    let s = setup();
    let r = s.execute("python3 -c \"import sys; sys.stderr.write('e\\n'); sys.exit(3)\"");
    assert_eq!(r.exit_code, 3);
    assert!(r.stderr.contains('e'));
}

#[test]
fn py_module_invocation() {
    let s = setup();
    s.write_file("d.json", r#"{"b":2,"a":1}"#).unwrap();
    let r = s.execute("python3 -m json.tool d.json");
    assert!(r.is_success(), "stderr={}", r.stderr);
    assert!(r.stdout.contains("\"a\": 1"), "{}", r.stdout);
}

#[test]
fn py_subprocess_and_threading() {
    let s = setup();
    let out = py(
        &s,
        r#"
import subprocess, threading
r = subprocess.run(["echo", "sub"], capture_output=True, text=True)
print(r.stdout.strip())
box = []
t = threading.Thread(target=lambda: box.append("t"))
t.start(); t.join()
print(box)
"#,
    );
    assert!(out.contains("sub"), "{out}");
    assert!(out.contains("['t']"), "{out}");
}

#[test]
fn py2_dialect_shims() {
    let s = setup();
    // py2-only names should work under the py2 dialect.
    let out = py(
        &s,
        r#"
import sys
print(sys.version_info[0])
print(list(xrange(3)))
print(isinstance("x", basestring))
"#,
    );
    assert!(out.contains("3"), "{out}");
}

#[test]
fn py_sandbox_escape_pathlib_io_scandir() {
    let (s, root) = setup_dir();
    py(
        &s,
        r#"
import pathlib, io, os, shutil
pathlib.Path("../pleak.txt").write_text("x")
io.open("../ioleak.txt", "w").write("x")
try:
    list(os.scandir("../"))
except Exception:
    pass
"#,
    );
    let parent = root.parent().unwrap();
    assert!(
        !parent.join("pleak.txt").exists(),
        "pathlib escaped sandbox"
    );
    assert!(
        !parent.join("ioleak.txt").exists(),
        "io.open escaped sandbox"
    );
}

#[test]
fn py_dialect_module_aliases() {
    let s = setup();
    let out = py(
        &s,
        r#"
import urllib2, StringIO, ConfigParser, Queue, cPickle, httplib
from urllib2 import urlopen
from StringIO import StringIO
print(StringIO("hello").read())
print(hasattr(ConfigParser, "ConfigParser"), hasattr(Queue, "Queue"))
"#,
    );
    assert!(out.contains("hello"), "{out}");
    assert!(out.contains("True True"), "{out}");
}

#[test]
fn py3_unaffected_by_dialect() {
    let s = setup();
    let out = py(
        &s,
        r#"
print(list(range(3)))
xrange = "mine"
print(xrange)
import sys
print(sys.version_info[0] >= 3)
"#,
    );
    assert!(out.contains("[0, 1, 2]"), "{out}");
    assert!(out.contains("mine"), "{out}");
    assert!(out.contains("True"), "{out}");
}

#[test]
fn py_stdin_program_heredoc() {
    let s = setup();
    // `python3 - << EOF`: program from heredoc.
    let r = s.execute("python3 - << 'EOF'\nprint('ok', 1 + 1)\nEOF");
    assert!(r.is_success(), "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "ok 2", "stdout={}", r.stdout);
    // `python3 - args << EOF`: heredoc is the program, args go to sys.argv.
    let r = s.execute("python3 - hello << 'EOF'\nimport sys\nprint(sys.argv[1], 2 + 2)\nEOF");
    assert!(r.is_success(), "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "hello 4", "stdout={}", r.stdout);
}

#[test]
fn py_stdin_program_pipe() {
    let s = setup();
    let r = s.execute("echo 'print(6 * 7)' | python3 -");
    assert!(r.is_success(), "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "42", "stdout={}", r.stdout);
}

#[test]
fn py_c_alias_works() {
    let s = setup();
    let r = s.execute(r#"python -c "print('alias-ok')""#);
    assert!(r.is_success(), "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "alias-ok", "stdout={}", r.stdout);
}
