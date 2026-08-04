// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use serde_json::Value;

const JQ_HELP_TEXT: &str = "\
jq: command-line JSON processor
Usage: jq [OPTIONS] 'filter' [FILE]
Options:
  -r, --raw-output  Output raw strings (not JSON)
  -h, --help        Show this help message
";

impl Shell {
    pub fn cmd_jq(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(JQ_HELP_TEXT.to_string());
        }
        let mut raw_output = false;
        let mut filter = String::new();

        for arg in args {
            match *arg {
                "-r" | "--raw-output" => raw_output = true,
                arg if !arg.starts_with('-') && filter.is_empty() => filter = arg.to_string(),
                _ => eprintln!("jq: warning: unsupported option '{}'", arg),
            }
        }

        if filter.is_empty() {
            return CommandOutput::error("jq: missing filter\n".to_string(), 1);
        }

        let input = match stdin {
            Some(s) => s.to_string(),
            None => return CommandOutput::error("jq: missing input\n".to_string(), 1),
        };

        let value: Value = match serde_json::from_str(&input) {
            Ok(v) => v,
            Err(e) => return CommandOutput::error(format!("jq: parse error: {}\n", e), 1),
        };

        let results = apply_jq_filter(&value, &filter);

        let mut output = String::new();
        for result in results {
            match result {
                JqResult::Value(v) => {
                    if raw_output {
                        match v {
                            Value::String(s) => output.push_str(&s),
                            Value::Number(n) => output.push_str(&n.to_string()),
                            Value::Bool(b) => output.push_str(&format!("{}", b)),
                            Value::Null => {}
                            _ => {
                                let s = serde_json::to_string(&v).unwrap_or_default();
                                output.push_str(&s);
                            }
                        }
                    } else {
                        let s = serde_json::to_string(&v).unwrap_or_default();
                        output.push_str(&s);
                    }
                    output.push('\n');
                }
                JqResult::Error(e) => {
                    return CommandOutput::error(format!("jq: {}\n", e), 1);
                }
            }
        }

        CommandOutput::success(output)
    }
}

enum JqResult {
    Value(Value),
    Error(String),
}

fn apply_jq_filter(value: &Value, filter: &str) -> Vec<JqResult> {
    let filter = filter.trim();
    if filter == "." {
        return vec![JqResult::Value(value.clone())];
    }

    if let Some(rest) = filter.strip_prefix('.') {
        if rest.contains('|') {
            let parts: Vec<&str> = rest.split('|').map(|s| s.trim()).collect();
            let mut current = vec![JqResult::Value(value.clone())];
            for part in parts {
                let mut next = Vec::new();
                for result in current {
                    match result {
                        JqResult::Value(v) => {
                            let sub = apply_jq_filter(&v, &format!(".{}", part));
                            next.extend(sub);
                        }
                        e => next.push(e),
                    }
                }
                current = next;
            }
            return current;
        }

        if rest.contains('.') {
            let mut parts: Vec<&str> = rest.split('.').collect();
            let first = parts.remove(0);
            let first_results = apply_jq_filter(value, &format!(".{}", first));
            let mut all_results = Vec::new();
            for result in first_results {
                match result {
                    JqResult::Value(v) => {
                        let remaining = parts.join(".");
                        if remaining.is_empty() {
                            all_results.push(JqResult::Value(v));
                        } else {
                            all_results.extend(apply_jq_filter(&v, &format!(".{}", remaining)));
                        }
                    }
                    e => all_results.push(e),
                }
            }
            return all_results;
        }

        let (key, array_op) = parse_jq_key(rest);
        match value {
            Value::Object(obj) => {
                if let Some(v) = obj.get(&key) {
                    return apply_jq_array_op(v, &array_op);
                }
                return vec![];
            }
            _ => return vec![],
        }
    }

    if filter == "[]" {
        return apply_jq_array_op(value, "[]");
    }

    if filter.starts_with("[") && filter.ends_with("]") {
        let index_str = &filter[1..filter.len() - 1];
        return apply_jq_array_op(value, index_str);
    }

    if filter.starts_with('{') && filter.ends_with('}') {
        if let Value::Object(_) = value {
            return vec![JqResult::Value(value.clone())];
        }
        return vec![];
    }

    return vec![JqResult::Error(format!("unsupported filter: {}", filter))];
}

fn parse_jq_key(rest: &str) -> (String, String) {
    let mut key = String::new();
    let mut chars = rest.chars().peekable();

    while let Some(ch) = chars.peek() {
        if *ch == '[' || *ch == '.' {
            break;
        }
        key.push(chars.next().unwrap());
    }

    let remaining: String = chars.collect();
    (key, remaining)
}

fn apply_jq_array_op(value: &Value, op: &str) -> Vec<JqResult> {
    if op.is_empty() {
        return vec![JqResult::Value(value.clone())];
    }

    if op == "[]" {
        match value {
            Value::Array(arr) => {
                return arr.iter().map(|v| JqResult::Value(v.clone())).collect();
            }
            _ => return vec![],
        }
    }

    if op.starts_with('[') && op.ends_with(']') {
        let index_str = &op[1..op.len() - 1];
        match value {
            Value::Array(arr) => {
                if let Ok(idx) = index_str.parse::<usize>() {
                    return arr
                        .get(idx)
                        .map(|v| vec![JqResult::Value(v.clone())])
                        .unwrap_or_default();
                }
            }
            _ => {}
        }
    }

    return vec![JqResult::Value(value.clone())];
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;

    fn mk_shell() -> Shell {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("fastshell_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_jq_help() {
        let mut shell = mk_shell();
        let out = shell.execute("jq", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_jq_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("jq", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
