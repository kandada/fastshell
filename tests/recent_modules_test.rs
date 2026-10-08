// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Targeted tests for the modules added/adjusted in the recent robustness
//! passes: the jq mini-engine (built-ins + string interpolation), the
//! `PRODUCER | while read ...` idiom, `date -r`, and `type`/`help`/`which`.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_recent_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
        ..Default::default()
    })
    .unwrap();
    sdk
}

// ── jq mini-engine: built-ins ────────────────────────────────────────────

#[test]
fn jq_type_of_each_json_kind() {
    let sdk = setup();
    for (json, want) in [
        (r#"{"a":1}"#, "object"),
        ("[1,2,3]", "array"),
        (r#""hi""#, "string"),
        ("42", "number"),
        ("true", "boolean"),
        ("null", "null"),
    ] {
        sdk.write_file("v.json", json).unwrap();
        let r = sdk.execute("jq -r 'type' v.json");
        assert_eq!(r.exit_code, 0, "json={json} stderr={}", r.stderr);
        assert_eq!(r.stdout.trim(), want, "json={json} stdout={}", r.stdout);
    }
}

#[test]
fn jq_keys_on_object_and_array() {
    let sdk = setup();
    sdk.write_file("o.json", r#"{"b":1,"a":2}"#).unwrap();
    let r = sdk.execute("jq -c 'keys' o.json");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    // keys are emitted (order not asserted beyond containing both).
    assert!(
        r.stdout.contains("\"a\"") && r.stdout.contains("\"b\""),
        "stdout={}",
        r.stdout
    );

    sdk.write_file("a.json", "[10,20,30]").unwrap();
    let r = sdk.execute("jq -c 'keys' a.json");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "[0,1,2]", "stdout={}", r.stdout);
}

#[test]
fn jq_length_of_array_object_and_string() {
    let sdk = setup();
    sdk.write_file("d.json", r#"{"a":[1,2,3,4],"s":"hello"}"#)
        .unwrap();
    assert_eq!(sdk.execute("jq '.a | length' d.json").stdout.trim(), "4");
    assert_eq!(sdk.execute("jq '.s | length' d.json").stdout.trim(), "5");
    assert_eq!(sdk.execute("jq 'length' d.json").stdout.trim(), "2");
}

#[test]
fn jq_pipe_stage_function_after_path() {
    let sdk = setup();
    sdk.write_file("d.json", r#"{"a":[1,2],"b":"x"}"#).unwrap();
    let r = sdk.execute("jq -r '.a | type' d.json");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "array", "stdout={}", r.stdout);
    let r = sdk.execute("jq -r '.b | type' d.json");
    assert_eq!(r.stdout.trim(), "string", "stdout={}", r.stdout);
}

// ── jq mini-engine: string interpolation ─────────────────────────────────

#[test]
fn jq_interpolation_basic_and_null() {
    let sdk = setup();
    sdk.write_file("d.json", r#"{"t":"hi","n":null}"#).unwrap();
    let r = sdk.execute(r#"jq -r '"\(.t)-end"' d.json"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "hi-end", "stdout={}", r.stdout);
    // null interpolates to empty string.
    let r = sdk.execute(r#"jq -r '"x\(.n)y"' d.json"#);
    assert_eq!(r.stdout.trim(), "xy", "stdout={}", r.stdout);
}

#[test]
fn jq_interpolation_escapes_and_literals() {
    let sdk = setup();
    sdk.write_file("d.json", r#"{"t":"v"}"#).unwrap();
    // \n / \t are real escapes inside a jq string literal.
    let r = sdk.execute(r#"jq -r '"a\nb"' d.json"#);
    assert_eq!(r.stdout, "a\nb\n", "stdout={:?}", r.stdout);
    // A plain string literal with no interpolation is returned verbatim.
    let r = sdk.execute(r#"jq -r '"plain"' d.json"#);
    assert_eq!(r.stdout.trim(), "plain", "stdout={}", r.stdout);
}

#[test]
fn jq_interpolation_unterminated_is_error() {
    let sdk = setup();
    sdk.write_file("d.json", r#"{"t":"v"}"#).unwrap();
    let r = sdk.execute(r#"jq '"\(.t"' d.json"#);
    assert_ne!(r.exit_code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
}

// ── `PRODUCER | while read` ──────────────────────────────────────────────

#[test]
fn while_read_multiple_vars() {
    let sdk = setup();
    sdk.write_file("t.txt", "a b c\nx y z\n").unwrap();
    let r = sdk.execute("cat t.txt | while read k v rest; do echo \"[$k|$v|$rest]\"; done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("[a|b|c]"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("[x|y|z]"), "stdout={}", r.stdout);
}

#[test]
fn while_read_last_var_gets_rest() {
    let sdk = setup();
    sdk.write_file("t.txt", "first second third fourth\n")
        .unwrap();
    let r = sdk.execute("cat t.txt | while read k rest; do echo \"[$k|$rest]\"; done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("[first|second third fourth]"),
        "stdout={}",
        r.stdout
    );
}

#[test]
fn while_read_empty_producer_no_iterations() {
    let sdk = setup();
    let r = sdk.execute("true | while read x; do echo \"never $x\"; done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.is_empty(), "stdout={}", r.stdout);
}

#[test]
fn while_read_producer_error_surfaces() {
    let sdk = setup();
    let r = sdk.execute("cat definitely_missing.txt | while read x; do echo \"$x\"; done");
    assert_ne!(r.exit_code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
}

// ── date -r / -d @epoch ──────────────────────────────────────────────────

#[test]
fn date_reference_epoch_forms() {
    let sdk = setup();
    assert_eq!(
        sdk.execute("date -r 0 -u +%Y-%m-%d").stdout.trim(),
        "1970-01-01"
    );
    assert_eq!(
        sdk.execute("date -d @1000000000 -u +%Y-%m-%d")
            .stdout
            .trim(),
        "2001-09-09"
    );
    // Negative epoch clamps to the Unix epoch rather than underflowing.
    assert_eq!(sdk.execute("date -r -5 -u +%Y").stdout.trim(), "1970");
}

#[test]
fn date_invalid_input_errors() {
    let sdk = setup();
    let r = sdk.execute("date -d @notanumber");
    assert_ne!(r.exit_code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    let r = sdk.execute("date -d 'definitely not a date'");
    assert_ne!(r.exit_code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
}

// ── type / help / which ──────────────────────────────────────────────────

#[test]
fn type_reports_alias_and_function() {
    let sdk = setup();
    sdk.execute("alias hi='echo hello'");
    let r = sdk.execute("type hi");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("aliased"), "stdout={}", r.stdout);

    sdk.execute("greet() { echo hi; }");
    let r = sdk.execute("type greet");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("function"), "stdout={}", r.stdout);
}

#[test]
fn type_missing_operand_errors() {
    let sdk = setup();
    let r = sdk.execute("type");
    assert_ne!(r.exit_code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
}

#[test]
fn which_and_command_v() {
    let sdk = setup();
    let r = sdk.execute("which ls");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("ls"), "stdout={}", r.stdout);
    let r = sdk.execute("which definitely_not_a_command_xyz");
    assert_ne!(r.exit_code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    let r = sdk.execute("command -v ls");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
}

#[test]
fn help_for_specific_command() {
    let sdk = setup();
    let r = sdk.execute("help grep");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(!r.stdout.is_empty(), "help grep produced no output");
}

// ── empty quoted arguments survive tokenization ──────────────────────────

#[test]
fn empty_quoted_argument_is_preserved() {
    let sdk = setup();
    // `printf ''` used to fail with "missing format" because the empty quoted
    // argument was dropped from argv.
    let r = sdk.execute("printf ''");
    assert_eq!(r.exit_code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.is_empty(), "stdout={}", r.stdout);

    // An empty argument in the middle of a list keeps its slot.
    let r = sdk.execute("printf '%s-%s-%s' x '' y");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout, "x--y", "stdout={:?}", r.stdout);
}

#[test]
fn empty_double_quoted_argument_preserved_in_echo() {
    let sdk = setup();
    let r = sdk.execute("echo \"a\" \"\" \"b\"");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    // Two spaces: the empty middle argument is a real argument.
    assert_eq!(r.stdout, "a  b\n", "stdout={:?}", r.stdout);
}

#[test]
fn empty_quoted_argument_with_redirect() {
    let sdk = setup();
    // The empty argument must not be mistaken for the redirect operand.
    let r = sdk.execute("printf '[%s]' '' > out.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(sdk.read_file("out.txt").unwrap(), "[]");
}
