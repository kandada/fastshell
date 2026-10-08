// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Dedicated (heavy) coverage for the extended `jq` / `awk` engines.
//!
//! These are intentionally `#[ignore]`d so the default `cargo test` stays fast.
//! Run them via `bash scripts/full-test.sh` or:
//!   cargo test --test jq_awk_full -- --ignored --nocapture

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_jqawk_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 2000,
        ..Default::default()
    })
    .unwrap();
    s
}

/// Run `jq -n '<filter>'` and return the raw stdout.
fn jq_raw(s: &Fastshell, filter: &str) -> String {
    let r = s.execute(&format!("jq -n '{filter}'"));
    assert_eq!(r.exit_code, 0, "jq {filter:?} failed: {}", r.stderr);
    r.stdout
}

/// Run `jq -n '<filter>'` and parse the (single-line) JSON result.
fn jq(s: &Fastshell, filter: &str) -> Value {
    let out = jq_raw(s, filter);
    let line = out.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("jq {filter:?} -> {line:?}: {e}"))
}

fn check(s: &Fastshell, filter: &str, expected: Value) {
    let got = jq(s, filter);
    assert_eq!(got, expected, "jq {filter:?}");
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_map_select() {
    let s = setup();
    check(&s, "[1,2,3] | map(.+1)", serde_json::json!([2, 3, 4]));
    check(
        &s,
        "[1,2,3,4] | map(select(. > 2))",
        serde_json::json!([3, 4]),
    );
    check(&s, "[1,2,3] | map(. * 2) | add", serde_json::json!(12));
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_objects() {
    let s = setup();
    check(&s, "{a:1,b:2} | has(\"a\")", Value::Bool(true));
    check(&s, "{a:1,b:2} | has(\"z\")", Value::Bool(false));
    check(
        &s,
        "[{key:\"a\",value:1},{key:\"b\",value:2}] | from_entries",
        serde_json::json!({"a": 1, "b": 2}),
    );
    check(&s, "{a:{b:1}} | .a.b", serde_json::json!(1));
    let entries = jq(&s, "{a:1,b:2} | to_entries");
    assert!(entries.is_array());
    assert_eq!(entries.as_array().unwrap().len(), 2);
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_arrays() {
    let s = setup();
    check(&s, "[1,2,3] | add", serde_json::json!(6));
    check(&s, "[3,1,2] | sort", serde_json::json!([1, 2, 3]));
    check(&s, "[1,1,2,3,3] | unique", serde_json::json!([1, 2, 3]));
    check(&s, "[1,2,3] | reverse", serde_json::json!([3, 2, 1]));
    check(&s, "[1,2,3] | first", serde_json::json!(1));
    check(&s, "[1,2,3] | last", serde_json::json!(3));
    check(&s, "[5,2,9,1] | min", serde_json::json!(1));
    check(&s, "[5,2,9,1] | max", serde_json::json!(9));
    check(&s, "[1,[2,[3]]] | flatten", serde_json::json!([1, 2, 3]));
    check(&s, "[1,2] | contains([2])", Value::Bool(true));
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_strings() {
    let s = setup();
    check(
        &s,
        "\"a,b,c\" | split(\",\")",
        serde_json::json!(["a", "b", "c"]),
    );
    check(
        &s,
        "[\"a\",\"b\",\"c\"] | join(\",\")",
        serde_json::json!("a,b,c"),
    );
    check(
        &s,
        "\"a,b\" | split(\",\") | join(\"-\")",
        serde_json::json!("a-b"),
    );
    check(&s, "\"abcdef\" | startswith(\"abc\")", Value::Bool(true));
    check(&s, "\"abcdef\" | endswith(\"def\")", Value::Bool(true));
    check(&s, "\"HELLO\" | ascii_downcase", serde_json::json!("hello"));
    check(&s, "\"hi\" | @base64", serde_json::json!("aGk="));
    check(&s, "\"42\" | tonumber", serde_json::json!(42));
    check(&s, "5 | tostring", serde_json::json!("5"));
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_control_flow() {
    let s = setup();
    check(&s, "null // 5", serde_json::json!(5));
    check(&s, "3 // 5", serde_json::json!(3));
    check(
        &s,
        "if 1 > 0 then \"yes\" else \"no\" end",
        serde_json::json!("yes"),
    );
    check(
        &s,
        "if 1 < 0 then \"yes\" else \"no\" end",
        serde_json::json!("no"),
    );
    // comma produces multiple outputs
    assert_eq!(jq_raw(&s, "1,2,3"), "1\n2\n3\n");
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_values() {
    let s = setup();
    let mut v = jq(&s, "{a:1,b:2} | values");
    if let Some(a) = v.as_array_mut() {
        a.sort_by_key(|x| x.as_i64().unwrap_or(0));
    }
    assert_eq!(v, serde_json::json!([1, 2]));
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn awk_essentials() {
    let s = setup();
    s.write_file("d.txt", "a 1\nb 2\nc 3\n").unwrap();
    let r = s.execute("awk '{print $1}' d.txt");
    assert_eq!(r.stdout, "a\nb\nc\n");
    let r = s.execute("awk '{s+=$2} END{print s}' d.txt");
    assert_eq!(r.stdout.trim(), "6");
    let r = s.execute("awk 'BEGIN{for(i=1;i<=3;i++)printf \"%d \", i}'");
    assert_eq!(r.stdout.trim(), "1 2 3");
    let r = s.execute("awk -F, '{print $2}' <<< 'x,y,z'");
    assert_eq!(r.stdout.trim(), "y");
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_variables_as_reduce_foreach() {
    let s = setup();
    assert_eq!(jq_raw(&s, "[1,2,3] | .[] as $x | $x + 1"), "2\n3\n4\n");
    check(
        &s,
        "[1,2,3] | reduce .[] as $x (0; . + $x)",
        serde_json::json!(6),
    );
    assert_eq!(
        jq_raw(&s, "[1,2,3] | foreach .[] as $x (0; . + $x)"),
        "1\n3\n6\n"
    );
    check(&s, "{a:{b:2}} as $x | $x.a.b", serde_json::json!(2));
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_try_def_elif() {
    let s = setup();
    check(&s, "try (1/0) catch \"err\"", serde_json::json!("err"));
    check(&s, "def inc: . + 1; 5 | inc", serde_json::json!(6));
    check(&s, "def add(x): . + x; 3 | add(4)", serde_json::json!(7));
    check(
        &s,
        "if 1 > 2 then \"a\" elif 2 > 1 then \"b\" else \"c\" end",
        serde_json::json!("b"),
    );
}

#[test]
#[ignore = "heavy: run explicitly via scripts/full-test.sh"]
fn jq_extended_builtins() {
    let s = setup();
    check(
        &s,
        "[{n:2},{n:1},{n:2}] | sort_by(.n)",
        serde_json::json!([{"n":1},{"n":2},{"n":2}]),
    );
    check(
        &s,
        "[{n:2},{n:1},{n:2}] | unique_by(.n)",
        serde_json::json!([{"n":1},{"n":2}]),
    );
    check(&s, "[{n:2},{n:1}] | min_by(.n)", serde_json::json!({"n":1}));
    check(&s, "[{n:2},{n:1}] | max_by(.n)", serde_json::json!({"n":2}));
    check(&s, "[1,2,3] | any(. > 2)", serde_json::json!(true));
    check(&s, "[1,2,3] | all(. > 0)", serde_json::json!(true));
    check(
        &s,
        "{a:1,b:2} | with_entries(select(.value > 1))",
        serde_json::json!({"b":2}),
    );
    check(&s, "\"abc\" | explode", serde_json::json!([97, 98, 99]));
    check(&s, "[97,98,99] | implode", serde_json::json!("abc"));
    check(&s, "\"abc\" | ascii", serde_json::json!(97));
    check(
        &s,
        "\"foobar\" | ltrimstr(\"foo\")",
        serde_json::json!("bar"),
    );
    check(
        &s,
        "\"foobar\" | rtrimstr(\"bar\")",
        serde_json::json!("foo"),
    );
    check(&s, "[1,2,3,2] | indices(2)", serde_json::json!([1, 3]));
    check(&s, "\"abcabc\" | index(\"bc\")", serde_json::json!(1));
    assert_eq!(jq_raw(&s, "range(3)"), "0\n1\n2\n");
    assert_eq!(jq_raw(&s, "range(1;4)"), "1\n2\n3\n");
    check(&s, "\"a b\" | @sh", serde_json::json!("'a b'"));
    check(&s, "\"<x>\" | @html", serde_json::json!("&lt;x&gt;"));
    check(&s, "$ENV | type", serde_json::json!("object"));
}
