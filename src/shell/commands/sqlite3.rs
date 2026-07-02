// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_sqlite3(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut db_path: Option<String> = None;
        let mut sql: Option<String> = None;
        let mut dot_command: Option<String> = None;
        let mut csv_mode = false;
        let mut header_mode = false;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-csv" => csv_mode = true,
                "-header" | "-headers" => header_mode = true,
                "-line" => {} // accepted, simple line mode
                arg if !arg.starts_with('-') && db_path.is_none() => {
                    db_path = Some(arg.to_string());
                }
                arg if !arg.starts_with('-') && sql.is_none() && !arg.starts_with('.') => {
                    sql = Some(arg.to_string());
                }
                arg if arg.starts_with('.') && dot_command.is_none() => {
                    dot_command = Some(arg.to_string());
                }
                _ => {}
            }
            i += 1;
        }

        let db_path = match db_path {
            Some(p) => p,
            None => {
                return CommandOutput::error(
                    "sqlite3: missing database path\nUsage: sqlite3 <database> [query]\n"
                        .to_string(),
                    1,
                )
            }
        };

        // Resolve VFS path
        let abs_path = match self.vfs.resolve(&db_path, &self.cwd) {
            Ok(p) => p,
            Err(e) => return CommandOutput::error(format!("sqlite3: {}\n", e), 1),
        };

        // Handle dot-commands
        if let Some(ref dc) = dot_command {
            return self.sqlite3_dot_command(&abs_path, dc);
        }

        // Handle direct SQL or stdin
        let sql_str = if let Some(ref s) = sql {
            s.clone()
        } else if let Some(stdin_data) = stdin {
            stdin_data.to_string()
        } else {
            // Try reading SQL from args if no stdin
            let remaining: Vec<&str> = args
                .iter()
                .skip_while(|a| !a.starts_with('-') || **a == db_path)
                .copied()
                .collect();
            if remaining.len() > 1 {
                remaining[1..].join(" ")
            } else {
                return CommandOutput::success(format!(
                    "SQLite version 3.45.3 (fastshell)\nEnter \".help\" for usage hints.\nConnected to {}\n",
                    db_path
                ));
            }
        };

        if sql_str.trim().is_empty() {
            return CommandOutput::success(String::new());
        }

        self.sqlite3_execute(&abs_path, &sql_str, csv_mode, header_mode)
    }

    fn sqlite3_dot_command(&self, db_path: &std::path::Path, cmd: &str) -> CommandOutput {
        let conn = match rusqlite::Connection::open(db_path) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("sqlite3: {}\n", e), 1),
        };

        match cmd {
            ".tables" | ".table" => {
                match conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name") {
                    Ok(mut stmt) => {
                        let mut output = String::new();
                        let rows = stmt.query_map([], |row| row.get::<_, String>(0));
                        match rows {
                            Ok(mapped) => {
                                for name in mapped.flatten() {
                                    output.push_str(&name);
                                    output.push(' ');
                                }
                                output.push('\n');
                                CommandOutput::success(output)
                            }
                            Err(e) => CommandOutput::error(format!("sqlite3: {}\n", e), 1),
                        }
                    }
                    Err(e) => CommandOutput::error(format!("sqlite3: {}\n", e), 1),
                }
            }
            ".schema" => {
                match conn.prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name") {
                    Ok(mut stmt) => {
                        let mut output = String::new();
                        let rows = stmt.query_map([], |row| row.get::<_, String>(0));
                        match rows {
                            Ok(mapped) => {
                                for sql in mapped.flatten() {
                                    output.push_str(&sql);
                                    output.push_str(";\n");
                                }
                                CommandOutput::success(output)
                            }
                            Err(e) => CommandOutput::error(format!("sqlite3: {}\n", e), 1),
                        }
                    }
                    Err(e) => CommandOutput::error(format!("sqlite3: {}\n", e), 1),
                }
            }
            ".help" | ".h" => {
                CommandOutput::success(
                    ".tables          List tables\n\
                     .schema           Show CREATE statements\n\
                     .help             Show this message\n\
                     .quit             Exit\n".to_string()
                )
            }
            ".quit" | ".q" | ".exit" => {
                CommandOutput::success(String::new())
            }
            ".dump" => {
                match conn.prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL UNION ALL SELECT 'INSERT INTO ' || name || ' VALUES(...);' FROM sqlite_master WHERE type='table'") {
                    Ok(_) => CommandOutput::error("sqlite3: .dump not fully implemented\n".to_string(), 1),
                    Err(e) => CommandOutput::error(format!("sqlite3: {}\n", e), 1),
                }
            }
            _ => {
                CommandOutput::error(format!("sqlite3: unknown command: {}\n", cmd), 1)
            }
        }
    }

    fn sqlite3_execute(
        &self,
        db_path: &std::path::Path,
        sql: &str,
        csv_mode: bool,
        header_mode: bool,
    ) -> CommandOutput {
        let conn = match rusqlite::Connection::open(db_path) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("sqlite3: {}\n", e), 1),
        };

        let mut output = String::new();
        let mut stderr = String::new();

        let statements: Vec<&str> = sql
            .split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        for stmt_str in &statements {
            let sql_upper = stmt_str.trim().to_uppercase();

            if sql_upper.starts_with("SELECT")
                || sql_upper.starts_with("PRAGMA")
                || sql_upper.starts_with("EXPLAIN")
            {
                match conn.prepare(stmt_str) {
                    Ok(mut stmt) => {
                        let col_count = stmt.column_count();
                        let col_names: Vec<String> = (0..col_count)
                            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
                            .collect();

                        if header_mode && !col_names.is_empty() {
                            if csv_mode {
                                output.push_str(&col_names.join(","));
                            } else {
                                output.push_str(&col_names.join("|"));
                            }
                            output.push('\n');
                        }

                        let rows = stmt.query_map([], |row| {
                            let mut vals = Vec::new();
                            for i in 0..col_count {
                                let val: String = match row.get_ref(i) {
                                    Ok(rusqlite::types::ValueRef::Null) => String::new(),
                                    Ok(rusqlite::types::ValueRef::Integer(n)) => n.to_string(),
                                    Ok(rusqlite::types::ValueRef::Real(f)) => f.to_string(),
                                    Ok(rusqlite::types::ValueRef::Text(t)) => {
                                        String::from_utf8_lossy(t).to_string()
                                    }
                                    Ok(rusqlite::types::ValueRef::Blob(b)) => {
                                        format!(
                                            "x'{}'",
                                            b.iter()
                                                .map(|byte| format!("{:02x}", byte))
                                                .collect::<String>()
                                        )
                                    }
                                    Err(_) => "ERROR".to_string(),
                                };
                                vals.push(val);
                            }
                            Ok(vals)
                        });

                        match rows {
                            Ok(mapped) => {
                                for row in mapped.flatten() {
                                    if csv_mode {
                                        output.push_str(&row.join(","));
                                    } else {
                                        output.push_str(&row.join("|"));
                                    }
                                    output.push('\n');
                                }
                            }
                            Err(e) => {
                                stderr.push_str(&format!("sqlite3: {}\n", e));
                            }
                        }
                    }
                    Err(e) => {
                        stderr.push_str(&format!("sqlite3: {}\n", e));
                    }
                }
            } else {
                match conn.execute_batch(stmt_str) {
                    Ok(_) => {}
                    Err(e) => {
                        stderr.push_str(&format!("sqlite3: {}\n", e));
                    }
                }
            }
        }

        if stderr.is_empty() {
            CommandOutput::success(output)
        } else {
            let had_output = !output.is_empty();
            CommandOutput {
                stdout: output,
                stderr,
                exit_code: if had_output { 0 } else { 1 },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup() -> Shell {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fs_sqlite3_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_sqlite3_create_and_select() {
        let shell = setup();
        shell.cmd_sqlite3(
            &[
                "test.db",
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
            ],
            None,
        );
        shell.cmd_sqlite3(&["test.db", "INSERT INTO users VALUES (1, 'Alice')"], None);
        shell.cmd_sqlite3(&["test.db", "INSERT INTO users VALUES (2, 'Bob')"], None);

        let out = shell.cmd_sqlite3(&["test.db", "SELECT * FROM users ORDER BY id"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Alice"));
        assert!(out.stdout.contains("Bob"));
        assert!(out.stdout.contains("1"));
        assert!(out.stdout.contains("2"));
    }

    #[test]
    fn test_sqlite3_dot_tables() {
        let shell = setup();
        shell.cmd_sqlite3(&["test.db", "CREATE TABLE users (id INTEGER)"], None);
        shell.cmd_sqlite3(&["test.db", "CREATE TABLE posts (id INTEGER)"], None);

        let out = shell.cmd_sqlite3(&["test.db", ".tables"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("users"));
        assert!(out.stdout.contains("posts"));
    }

    #[test]
    fn test_sqlite3_dot_schema() {
        let shell = setup();
        shell.cmd_sqlite3(&["test.db", "CREATE TABLE t (a TEXT, b INTEGER)"], None);

        let out = shell.cmd_sqlite3(&["test.db", ".schema"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.to_uppercase().contains("CREATE TABLE"));
    }

    #[test]
    fn test_sqlite3_dot_help() {
        let shell = setup();
        let out = shell.cmd_sqlite3(&["test.db", ".help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains(".tables"));
    }

    #[test]
    fn test_sqlite3_missing_db() {
        let shell = setup();
        let out = shell.cmd_sqlite3(&[], None);
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("missing database path"));
    }

    #[test]
    fn test_sqlite3_csv_mode() {
        let shell = setup();
        shell.cmd_sqlite3(&["test.db", "CREATE TABLE t (x, y)"], None);
        shell.cmd_sqlite3(&["test.db", "INSERT INTO t VALUES ('a', 'b')"], None);

        let out = shell.cmd_sqlite3(&["-csv", "test.db", "SELECT * FROM t"], None);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "a,b");
    }

    #[test]
    fn test_sqlite3_header_mode() {
        let shell = setup();
        shell.cmd_sqlite3(&["test.db", "CREATE TABLE t (col_a, col_b)"], None);
        shell.cmd_sqlite3(&["test.db", "INSERT INTO t VALUES ('x', 'y')"], None);

        let out = shell.cmd_sqlite3(&["-header", "test.db", "SELECT * FROM t"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("col_a"));
        assert!(out.stdout.contains("col_b"));
    }

    #[test]
    fn test_sqlite3_pipe_sql() {
        let shell = setup();
        // Use stdin to pass SQL
        let out = shell.cmd_sqlite3(
            &["test.db"],
            Some("CREATE TABLE x (n INTEGER); INSERT INTO x VALUES (42); SELECT * FROM x"),
        );
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("42"));
    }

    #[test]
    fn test_sqlite3_multiple_statements() {
        let shell = setup();
        let out = shell.cmd_sqlite3(&["test.db", "CREATE TABLE m (v TEXT); INSERT INTO m VALUES ('hello'); INSERT INTO m VALUES ('world'); SELECT * FROM m"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("hello"));
        assert!(out.stdout.contains("world"));
    }

    #[test]
    fn test_sqlite3_update_and_delete() {
        let shell = setup();
        shell.cmd_sqlite3(&["test.db", "CREATE TABLE d (id, val)"], None);
        shell.cmd_sqlite3(&["test.db", "INSERT INTO d VALUES (1, 'old')"], None);
        shell.cmd_sqlite3(&["test.db", "UPDATE d SET val='new' WHERE id=1"], None);

        let out = shell.cmd_sqlite3(&["test.db", "SELECT val FROM d WHERE id=1"], None);
        assert!(out.stdout.contains("new"));
        assert!(!out.stdout.contains("old"));

        shell.cmd_sqlite3(&["test.db", "DELETE FROM d WHERE id=1"], None);
        let out = shell.cmd_sqlite3(&["test.db", "SELECT count(*) FROM d"], None);
        assert!(out.stdout.contains("0"));
    }
}
