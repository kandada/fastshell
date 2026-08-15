// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Dedicated end-to-end tests for the built-in `sqlite3` command — covering
//! the real-world usage patterns an AI agent relies on (data types, joins,
//! aggregates, transactions, CSV export, sandbox isolation, pipelines).

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_sqlite3_int_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().into(),
        python_enabled: false,
        ..Default::default()
    })
    .unwrap();
    sdk
}

#[test]
fn data_types_roundtrip() {
    let sdk = setup();
    sdk.execute("sqlite3 t.db 'CREATE TABLE t (id INTEGER, r REAL, s TEXT, b BLOB, n NULL)'");
    sdk.execute("sqlite3 t.db \"INSERT INTO t VALUES (1, 3.5, 'hi', x'0102ff', NULL)\"");

    let r = sdk.execute("sqlite3 t.db 'SELECT * FROM t'");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    let line = r.stdout.trim().to_string();
    // columns joined by `|`: 1 | 3.5 | hi | x'0102ff' | (empty for NULL)
    assert!(line.contains("1"), "stdout={line}");
    assert!(line.contains("3.5"), "stdout={line}");
    assert!(line.contains("hi"), "stdout={line}");
    assert!(line.contains("x'0102ff'"), "blob must be hex: {line}");
    assert!(line.ends_with('|'), "NULL must render as empty: {line}");
}

#[test]
fn null_is_distinct_from_empty_string() {
    let sdk = setup();
    sdk.execute("sqlite3 t.db 'CREATE TABLE t (id, v)'");
    sdk.execute("sqlite3 t.db \"INSERT INTO t VALUES (1, NULL), (2, '')\"");

    // NULL and '' both render empty in the default pipe format, so use
    // IS NULL / = '' to tell them apart semantically.
    let r = sdk.execute("sqlite3 t.db \"SELECT id FROM t WHERE v IS NULL\"");
    assert_eq!(r.stdout.trim(), "1", "NULL row: {}", r.stdout);
    let r = sdk.execute("sqlite3 t.db \"SELECT id FROM t WHERE v = ''\"");
    assert_eq!(r.stdout.trim(), "2", "empty-string row: {}", r.stdout);
}

#[test]
fn joins_and_aggregates() {
    let sdk = setup();
    sdk.execute("sqlite3 shop.db 'CREATE TABLE customers (id, name)'");
    sdk.execute("sqlite3 shop.db 'CREATE TABLE orders (id, customer_id, amount)'");
    sdk.execute("sqlite3 shop.db \"INSERT INTO customers VALUES (1, 'Alice'), (2, 'Bob')\"");
    sdk.execute(
        "sqlite3 shop.db \"INSERT INTO orders VALUES (1, 1, 10), (2, 1, 20), (3, 2, 15)\"",
    );

    let r = sdk.execute(
        "sqlite3 shop.db 'SELECT c.name, COUNT(o.id), SUM(o.amount) FROM customers c LEFT JOIN orders o ON o.customer_id = c.id GROUP BY c.name ORDER BY c.name'",
    );
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("Alice|2|30"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("Bob|1|15"), "stdout={}", r.stdout);
}

#[test]
fn transaction_commit_persists() {
    let sdk = setup();
    sdk.execute("sqlite3 t.db 'CREATE TABLE t (x)'");
    let r = sdk.execute("sqlite3 t.db 'BEGIN; INSERT INTO t(x) VALUES (42); COMMIT'");
    // No SELECT → the DDL/DML path produces no stdout, but must not error.
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);

    let r = sdk.execute("sqlite3 t.db 'SELECT x FROM t'");
    assert_eq!(r.stdout.trim(), "42", "committed data must persist: {}", r.stdout);
}

#[test]
fn transaction_rollback_discards() {
    let sdk = setup();
    sdk.execute("sqlite3 t.db 'CREATE TABLE t (x)'");
    let r = sdk.execute("sqlite3 t.db 'BEGIN; INSERT INTO t(x) VALUES (99); ROLLBACK'");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);

    let r = sdk.execute("sqlite3 t.db 'SELECT COUNT(*) FROM t'");
    assert_eq!(r.stdout.trim(), "0", "rolled-back data must be gone: {}", r.stdout);
}

#[test]
fn recursive_cte_and_aggregates() {
    let sdk = setup();
    let r = sdk.execute(
        "sqlite3 t.db 'WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt WHERE x < 1000) SELECT COUNT(*), MAX(x), SUM(x) FROM cnt'",
    );
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "1000|1000|500500", "stdout={}", r.stdout);
}

#[test]
fn invalid_sql_reports_error() {
    let sdk = setup();
    let r = sdk.execute("sqlite3 t.db 'SELEC * FROM nope'");
    assert_ne!(r.exit_code, 0, "syntax error must fail");
    assert!(r.stderr.contains("sqlite3"), "stderr={}", r.stderr);
}

#[test]
fn dot_schema_shows_typed_create() {
    let sdk = setup();
    sdk.execute("sqlite3 t.db 'CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)'");

    let r = sdk.execute("sqlite3 t.db .schema");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    let up = r.stdout.to_uppercase();
    assert!(up.contains("CREATE TABLE"), "stdout={}", r.stdout);
    assert!(up.contains("INTEGER"), "stdout={}", r.stdout);
    assert!(up.contains("NOT NULL"), "stdout={}", r.stdout);
}

#[test]
fn csv_header_export_workflow() {
    let sdk = setup();
    sdk.execute("sqlite3 t.db 'CREATE TABLE people (name, age)'");
    sdk.execute("sqlite3 t.db \"INSERT INTO people VALUES ('Alice', 30), ('Bob', 25)\"");

    let r = sdk.execute("sqlite3 -csv -header t.db 'SELECT * FROM people ORDER BY name'");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    let lines: Vec<&str> = r.stdout.trim().lines().collect();
    assert_eq!(lines.len(), 3, "header + 2 rows: {}", r.stdout);
    assert_eq!(lines[0], "name,age", "header row: {}", r.stdout);
    assert_eq!(lines[1], "Alice,30", "row: {}", r.stdout);
    assert_eq!(lines[2], "Bob,25", "row: {}", r.stdout);
}

#[test]
fn sandbox_path_escape_blocked() {
    let sdk = setup();
    let r = sdk.execute("sqlite3 ../../escape.db 'CREATE TABLE t (x)'");
    assert_ne!(r.exit_code, 0, "path escape must be rejected");
    assert!(
        r.stderr.contains("sqlite3"),
        "should surface a sqlite3 error: {}",
        r.stderr
    );
}

#[test]
fn data_persists_across_calls() {
    let sdk = setup();
    // Each `execute` opens a fresh connection — the DB file on disk is the
    // source of truth, so writes must survive across calls.
    sdk.execute("sqlite3 t.db 'CREATE TABLE kv (k, v)'");
    sdk.execute("sqlite3 t.db \"INSERT INTO kv VALUES ('theme', 'dark')\"");
    let r = sdk.execute("sqlite3 t.db \"SELECT v FROM kv WHERE k = 'theme'\"");
    assert_eq!(r.stdout.trim(), "dark", "persistence across calls: {}", r.stdout);
}

#[test]
fn pipeline_with_grep() {
    let sdk = setup();
    sdk.execute("sqlite3 t.db 'CREATE TABLE log (msg)'");
    sdk.execute("sqlite3 t.db \"INSERT INTO log VALUES ('info: ok'), ('error: boom'), ('info: done')\"");

    let r = sdk.execute("sqlite3 t.db \"SELECT msg FROM log WHERE msg LIKE 'error:%'\" | grep error");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("boom"), "stdout={}", r.stdout);
}

#[test]
fn upsert_and_case() {
    let sdk = setup();
    sdk.execute("sqlite3 t.db 'CREATE TABLE c (id INTEGER PRIMARY KEY, n)'");
    sdk.execute("sqlite3 t.db \"INSERT INTO c VALUES (1, 'a')\"");
    sdk.execute("sqlite3 t.db \"INSERT INTO c VALUES (1, 'b') ON CONFLICT(id) DO UPDATE SET n = excluded.n\"");
    let r = sdk.execute("sqlite3 t.db 'SELECT n FROM c WHERE id = 1'");
    assert_eq!(r.stdout.trim(), "b", "upsert must replace: {}", r.stdout);
}
