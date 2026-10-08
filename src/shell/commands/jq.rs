// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use serde_json::Value;

const JQ_HELP_TEXT: &str = "\
jq: command-line JSON processor
Usage: jq [OPTIONS] 'filter' [FILE...]
Options:
  -r, --raw-output     Output raw strings (not JSON)
  -c, --compact-output Output compact (default here)
  -s, --slurp          Read entire input into an array
  -n, --null-input     Use 'null' as input
  -R, --raw-input      Read input as raw strings (line per value)
  -e, --exit-status    Exit 1 if result is null or false
      --arg N V        Set variable $N to JSON string V
      --argjson N V    Set variable $N to JSON value V
  -h, --help           Show this help message
";

impl Shell {
    pub fn cmd_jq(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(JQ_HELP_TEXT.to_string());
        }
        let mut raw_output = false;
        let mut slurp = false;
        let mut null_input = false;
        let mut raw_input = false;
        let mut exit_status = false;
        let mut vars: Vec<(String, String)> = Vec::new();
        let mut positional: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            match a {
                "-r" | "--raw-output" => raw_output = true,
                "-c" | "--compact-output" => {}
                "-s" | "--slurp" => slurp = true,
                "-n" | "--null-input" => null_input = true,
                "-R" | "--raw-input" => raw_input = true,
                "-e" | "--exit-status" => exit_status = true,
                "--arg" | "--argjson" => {
                    if i + 2 < args.len() {
                        let name = args[i + 1].to_string();
                        let val = args[i + 2].to_string();
                        let v = if a == "--arg" {
                            serde_json::to_string(&val).unwrap_or_default()
                        } else {
                            val
                        };
                        vars.push((name, v));
                        i += 2;
                    }
                }
                arg if arg.starts_with("--arg=") => {
                    if let Some((n, v)) = arg[6..].split_once('=') {
                        vars.push((
                            n.to_string(),
                            serde_json::to_string(&v.to_string()).unwrap_or_default(),
                        ));
                    }
                }
                arg if !arg.starts_with('-') => positional.push(arg.to_string()),
                _ => crate::warn!("jq: warning: unsupported option '{}'", a),
            }
            i += 1;
        }

        if positional.is_empty() {
            return CommandOutput::error("jq: missing filter\n".to_string(), 1);
        }
        let mut filter = positional.remove(0);
        // Substitute --arg/--argjson variables ($name → value).
        for (name, val) in &vars {
            filter = filter.replace(&format!("${}", name), val);
        }

        let input = if null_input {
            "null".to_string()
        } else if !positional.is_empty() {
            let mut content = String::new();
            for file in &positional {
                match self.read_text_lossy(file) {
                    Ok(c) => content.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("jq: {}: {}\n", file, e), 1),
                }
            }
            content
        } else {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("jq: missing input\n".to_string(), 1),
            }
        };

        // Build the input value(s) to run the filter over.
        let mut input_values: Vec<Value> = Vec::new();
        if raw_input {
            for line in input.lines() {
                input_values.push(Value::String(line.to_string()));
            }
        } else if slurp {
            let v = parse_slurp(&input);
            input_values.push(v);
        } else {
            match serde_json::from_str::<Value>(input.trim()) {
                Ok(v) => input_values.push(v),
                Err(e) => return CommandOutput::error(format!("jq: parse error: {}\n", e), 1),
            }
        }

        let mut output = String::new();
        let mut last_truthy = true;
        for value in input_values {
            let results = apply_jq_filter(&value, &filter);
            if results.is_empty() {
                last_truthy = false;
            }
            for result in results {
                match result {
                    JqResult::Value(v) => {
                        if v.is_null() || v == Value::Bool(false) {
                            last_truthy = false;
                        }
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
        }

        let code = if exit_status && !last_truthy { 1 } else { 0 };
        CommandOutput {
            stdout: output,
            stderr: String::new(),
            exit_code: code,
        }
    }
}

fn parse_slurp(input: &str) -> Value {
    // Whole input as one value.
    if let Ok(v) = serde_json::from_str::<Value>(input.trim()) {
        if v.is_array() {
            return v;
        }
        return Value::Array(vec![v]);
    }
    // Newline-delimited JSON stream.
    let mut arr = Vec::new();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            arr.push(v);
        }
    }
    Value::Array(arr)
}

enum JqResult {
    Value(Value),
    Error(String),
}

// ── variable / function scopes (thread-local; jq eval is single-threaded) ──
thread_local! {
    static JQ_SCOPES: std::cell::RefCell<Vec<std::collections::HashMap<String, Value>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static JQ_DEFS: std::cell::RefCell<Vec<std::collections::HashMap<String, (Vec<String>, String)>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

struct JqScopeGuard;
impl Drop for JqScopeGuard {
    fn drop(&mut self) {
        JQ_SCOPES.with(|s| {
            s.borrow_mut().pop();
        });
    }
}

fn jq_push_scope(entries: Vec<(String, Value)>) -> JqScopeGuard {
    JQ_SCOPES.with(|s| s.borrow_mut().push(entries.into_iter().collect()));
    JqScopeGuard
}

fn jq_var_get(name: &str) -> Option<Value> {
    JQ_SCOPES.with(|s| {
        for sc in s.borrow().iter().rev() {
            if let Some(v) = sc.get(name) {
                return Some(v.clone());
            }
        }
        None
    })
}

struct JqDefGuard;
impl Drop for JqDefGuard {
    fn drop(&mut self) {
        JQ_DEFS.with(|s| {
            s.borrow_mut().pop();
        });
    }
}

fn jq_push_defs(name: &str, params: Vec<String>, body: String) -> JqDefGuard {
    JQ_DEFS.with(|s| {
        let mut stack = s.borrow_mut();
        if stack.is_empty() {
            stack.push(std::collections::HashMap::new());
        }
        stack
            .last_mut()
            .unwrap()
            .insert(name.to_string(), (params, body));
    });
    JqDefGuard
}

fn jq_def_get(name: &str) -> Option<(Vec<String>, String)> {
    JQ_DEFS.with(|s| {
        for sc in s.borrow().iter().rev() {
            if let Some(v) = sc.get(name) {
                return Some(v.clone());
            }
        }
        None
    })
}

fn jq_env_object() -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in std::env::vars() {
        m.insert(k, Value::String(v));
    }
    Value::Object(m)
}

/// `EXPR as $name` (the left side of an `as`-binding pipe).
fn jq_as_binding(s: &str) -> Option<(&str, String)> {
    let pos = find_top_level(s, " as $")?;
    let name = &s[pos + 5..];
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some((s[..pos].trim(), name.to_string()))
}

/// `reduce EXPR as $x (INIT; UPDATE)`.
fn jq_reduce(value: &Value, rest: &str) -> Option<Vec<JqResult>> {
    let as_pos = find_top_level(rest, " as $")?;
    let src = rest[..as_pos].trim();
    let after = rest[as_pos + 5..].trim();
    let name_end = after
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(after.len());
    let name = &after[..name_end];
    if name.is_empty() {
        return None;
    }
    let paren = after[name_end..].trim();
    let inner = paren.strip_prefix('(')?.strip_suffix(')')?;
    let parts = split_top_level(inner, ';')?;
    if parts.len() < 2 {
        return None;
    }
    let mut acc = match apply_jq_filter(value, parts[0].trim()).into_iter().next() {
        Some(JqResult::Value(v)) => v,
        _ => Value::Null,
    };
    for r in apply_jq_filter(value, src) {
        if let JqResult::Value(x) = r {
            let _g = jq_push_scope(vec![(name.to_string(), x)]);
            if let Some(JqResult::Value(v)) =
                apply_jq_filter(&acc, parts[1].trim()).into_iter().next()
            {
                acc = v;
            }
        }
    }
    Some(vec![JqResult::Value(acc)])
}

/// `foreach EXPR as $x (INIT; UPDATE[; EXTRACT])`.
fn jq_foreach(value: &Value, rest: &str) -> Option<Vec<JqResult>> {
    let as_pos = find_top_level(rest, " as $")?;
    let src = rest[..as_pos].trim();
    let after = rest[as_pos + 5..].trim();
    let name_end = after
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(after.len());
    let name = &after[..name_end];
    if name.is_empty() {
        return None;
    }
    let paren = after[name_end..].trim();
    let inner = paren.strip_prefix('(')?.strip_suffix(')')?;
    let parts = split_top_level(inner, ';')?;
    if parts.len() < 2 {
        return None;
    }
    let update = parts[1].trim();
    let extract = parts.get(2).map(|p| p.trim());
    let mut acc = match apply_jq_filter(value, parts[0].trim()).into_iter().next() {
        Some(JqResult::Value(v)) => v,
        _ => Value::Null,
    };
    let mut out = Vec::new();
    for r in apply_jq_filter(value, src) {
        if let JqResult::Value(x) = r {
            let _g = jq_push_scope(vec![(name.to_string(), x)]);
            if let Some(JqResult::Value(v)) = apply_jq_filter(&acc, update).into_iter().next() {
                acc = v;
            }
            out.extend(apply_jq_filter(&acc, extract.unwrap_or(".")));
        }
    }
    Some(out)
}

/// `def name(params): body; rest`.
fn jq_def(value: &Value, rest: &str) -> Option<Vec<JqResult>> {
    let name_end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let name = rest[..name_end].to_string();
    if name.is_empty() {
        return None;
    }
    let mut tail = rest[name_end..].trim_start();
    let mut params: Vec<String> = Vec::new();
    if let Some(p) = tail.strip_prefix('(') {
        let close = p.find(')')?;
        params = p[..close]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        tail = p[close + 1..].trim_start();
    }
    let body_rest = tail.strip_prefix(':')?.trim();
    let semi = find_top_level(body_rest, ";")?;
    let body = body_rest[..semi].trim().to_string();
    let cont = body_rest[semi + 1..].trim();
    let _g = jq_push_defs(&name, params, body);
    Some(apply_jq_filter(value, cont))
}

/// Replace whole-word (outside strings) occurrences of `word` with `repl`.
fn jq_replace_word(s: &str, word: &str, repl: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let w: Vec<char> = word.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            in_str = !in_str;
            out.push(c);
            i += 1;
            continue;
        }
        if in_str {
            out.push(c);
            i += 1;
            continue;
        }
        let before_ok = i == 0
            || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '$');
        if before_ok
            && !w.is_empty()
            && i + w.len() <= chars.len()
            && chars[i..i + w.len()] == w[..]
        {
            let after = i + w.len();
            let after_ok =
                after >= chars.len() || !(chars[after].is_alphanumeric() || chars[after] == '_');
            if after_ok {
                out.push_str(repl);
                i = after;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Call a user-defined function (`name` or `name(a; b)`). Parameters are
/// substituted into the body (jq call-by-name, approximated by macro expansion).
fn jq_call_def(value: &Value, filter: &str) -> Option<Vec<JqResult>> {
    let (name, args) = match filter.find('(') {
        Some(p) if filter.ends_with(')') => (&filter[..p], Some(&filter[p + 1..filter.len() - 1])),
        _ => (filter, None),
    };
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let (params, body) = jq_def_get(name)?;
    let mut body = body;
    if let Some(argstr) = args {
        let argparts: Vec<&str> = split_top_level(argstr, ';').unwrap_or_else(|| vec![argstr]);
        for (i, p) in params.iter().enumerate() {
            let a = argparts.get(i).map(|s| s.trim()).unwrap_or("null");
            body = jq_replace_word(&body, p, &format!("({a})"));
        }
    }
    Some(apply_jq_filter(value, &body))
}

// ── extended-builtin helpers ──
fn jq_key_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        _ => a.to_string().cmp(&b.to_string()),
    }
}

fn jq_key_of(v: &Value, arg: &str) -> Value {
    apply_jq_filter(v, arg)
        .into_iter()
        .find_map(|r| match r {
            JqResult::Value(x) => Some(x),
            _ => None,
        })
        .unwrap_or(Value::Null)
}

fn jq_sort_by(value: &Value, arg: &str) -> Vec<JqResult> {
    let Value::Array(a) = value else {
        return vec![];
    };
    let mut items: Vec<(Value, Value)> = a.iter().map(|v| (jq_key_of(v, arg), v.clone())).collect();
    items.sort_by(|x, y| jq_key_cmp(&x.0, &y.0));
    vec![JqResult::Value(Value::Array(
        items.into_iter().map(|(_, v)| v).collect(),
    ))]
}

fn jq_group_by(value: &Value, arg: &str) -> Vec<JqResult> {
    let Value::Array(a) = value else {
        return vec![];
    };
    let mut items: Vec<(Value, Value)> = a.iter().map(|v| (jq_key_of(v, arg), v.clone())).collect();
    items.sort_by(|x, y| jq_key_cmp(&x.0, &y.0));
    let mut groups: Vec<Value> = Vec::new();
    let mut cur: Vec<Value> = Vec::new();
    let mut cur_key: Option<Value> = None;
    for (k, v) in items {
        if let Some(ck) = &cur_key {
            if jq_key_cmp(ck, &k) != std::cmp::Ordering::Equal {
                groups.push(Value::Array(std::mem::take(&mut cur)));
            }
        }
        cur_key = Some(k);
        cur.push(v);
    }
    if !cur.is_empty() {
        groups.push(Value::Array(cur));
    }
    vec![JqResult::Value(Value::Array(groups))]
}

fn jq_unique_by(value: &Value, arg: &str) -> Vec<JqResult> {
    let Value::Array(a) = value else {
        return vec![];
    };
    let mut items: Vec<(Value, Value)> = a.iter().map(|v| (jq_key_of(v, arg), v.clone())).collect();
    items.sort_by(|x, y| jq_key_cmp(&x.0, &y.0));
    let mut out: Vec<Value> = Vec::new();
    let mut last: Option<Value> = None;
    for (k, v) in items {
        if last
            .as_ref()
            .map(|l| jq_key_cmp(l, &k))
            .is_none_or(|o| o != std::cmp::Ordering::Equal)
        {
            last = Some(k);
            out.push(v);
        }
    }
    vec![JqResult::Value(Value::Array(out))]
}

fn jq_minmax_by(value: &Value, arg: &str, min: bool) -> Vec<JqResult> {
    let Value::Array(a) = value else {
        return vec![];
    };
    let mut best: Option<(Value, Value)> = None;
    for v in a {
        let k = jq_key_of(v, arg);
        let take = match &best {
            None => true,
            Some((bk, _)) => {
                let o = jq_key_cmp(&k, bk);
                if min {
                    o == std::cmp::Ordering::Less
                } else {
                    o == std::cmp::Ordering::Greater
                }
            }
        };
        if take {
            best = Some((k, v.clone()));
        }
    }
    best.map(|(_, v)| JqResult::Value(v)).into_iter().collect()
}

fn jq_with_entries(value: &Value, arg: &str) -> Vec<JqResult> {
    let entries = match value {
        Value::Object(o) => Value::Array(
            o.iter()
                .map(|(k, v)| serde_json::json!({"key": k, "value": v}))
                .collect(),
        ),
        Value::Array(a) => Value::Array(
            a.iter()
                .enumerate()
                .map(|(i, v)| serde_json::json!({"key": i, "value": v}))
                .collect(),
        ),
        _ => return vec![],
    };
    let mapped: Vec<Value> = match &entries {
        Value::Array(a) => a
            .iter()
            .flat_map(|e| {
                apply_jq_filter(e, arg).into_iter().filter_map(|r| match r {
                    JqResult::Value(v) => Some(v),
                    _ => None,
                })
            })
            .collect(),
        _ => return vec![],
    };
    let mut out = serde_json::Map::new();
    for e in mapped {
        let k = e
            .get("key")
            .or_else(|| e.get("k"))
            .map(|k| jq_str(k).unwrap_or_default());
        let v = e.get("value").or_else(|| e.get("v")).cloned();
        if let (Some(k), Some(v)) = (k, v) {
            out.insert(k, v);
        }
    }
    vec![JqResult::Value(Value::Object(out))]
}

fn jq_anyall(value: &Value, arg: &str, is_any: bool) -> Vec<JqResult> {
    let parts = split_top_level(arg, ';').unwrap_or_else(|| vec![arg]);
    let (gen, cond) = if parts.len() >= 2 {
        (parts[0].trim(), parts[1].trim())
    } else {
        // `any(cond)` / `all(cond)` iterate the input array.
        (".[]", parts[0].trim())
    };
    let mut r = !is_any;
    for it in apply_jq_filter(value, gen) {
        if let JqResult::Value(v) = it {
            let t = apply_jq_filter(&v, cond)
                .iter()
                .any(|x| matches!(x, JqResult::Value(v) if jq_truthy(v)));
            if is_any && t {
                r = true;
                break;
            }
            if !is_any && !t {
                r = false;
                break;
            }
        }
    }
    vec![JqResult::Value(Value::Bool(r))]
}

fn jq_index_of(value: &Value, needle: &Value, from_end: bool) -> Value {
    match (value, needle) {
        (Value::String(s), Value::String(n)) => {
            let chars: Vec<char> = s.chars().collect();
            let nc: Vec<char> = n.chars().collect();
            let mut found: Option<usize> = None;
            if nc.len() <= chars.len() {
                for i in 0..=(chars.len() - nc.len()) {
                    if chars[i..i + nc.len()] == nc[..] {
                        if from_end {
                            found = Some(i);
                        } else {
                            return serde_json::json!(i);
                        }
                    }
                }
            }
            found.map(|i| serde_json::json!(i)).unwrap_or(Value::Null)
        }
        (Value::Array(a), _) => {
            let idxs: Vec<usize> = a
                .iter()
                .enumerate()
                .filter(|(_, v)| *v == needle)
                .map(|(i, _)| i)
                .collect();
            let pick = if from_end { idxs.last() } else { idxs.first() };
            pick.map(|i| serde_json::json!(i)).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

fn jq_indices(value: &Value, needle: &Value) -> Value {
    match (value, needle) {
        (Value::String(s), Value::String(n)) => {
            let chars: Vec<char> = s.chars().collect();
            let nc: Vec<char> = n.chars().collect();
            let mut out = Vec::new();
            if !nc.is_empty() && nc.len() <= chars.len() {
                for i in 0..=(chars.len() - nc.len()) {
                    if chars[i..i + nc.len()] == nc[..] {
                        out.push(serde_json::json!(i));
                    }
                }
            }
            Value::Array(out)
        }
        (Value::Array(a), _) => Value::Array(
            a.iter()
                .enumerate()
                .filter(|(_, v)| *v == needle)
                .map(|(i, _)| serde_json::json!(i))
                .collect(),
        ),
        _ => Value::Array(vec![]),
    }
}

fn jq_range(arg: &str) -> Vec<JqResult> {
    let parts = split_top_level(arg, ';').unwrap_or_else(|| vec![arg]);
    let num = |s: &str| jq_operand(&Value::Null, s).as_f64().unwrap_or(0.0);
    let (start, end, step) = match parts.len() {
        1 => (0.0, num(parts[0].trim()), 1.0),
        2 => (num(parts[0].trim()), num(parts[1].trim()), 1.0),
        _ => (
            num(parts[0].trim()),
            num(parts[1].trim()),
            num(parts[2].trim()),
        ),
    };
    let mut out = Vec::new();
    if step == 0.0 {
        return out;
    }
    let mut x = start;
    let mut guard = 0;
    while (step > 0.0 && x < end) || (step < 0.0 && x > end) {
        out.push(JqResult::Value(jq_number(x)));
        x += step;
        guard += 1;
        if guard > 1_000_000 {
            break;
        }
    }
    out
}

fn apply_jq_filter(value: &Value, filter: &str) -> Vec<JqResult> {
    let filter = filter.trim();
    if filter == "." {
        return vec![JqResult::Value(value.clone())];
    }

    // Assignment / update operators: `PATH = V`, `PATH |= F`, `PATH += V`, …
    if let Some(out) = jq_try_assignment(value, filter) {
        return out;
    }
    // `del(PATH, PATH, …)`
    if let Some(args) = filter
        .strip_prefix("del(")
        .and_then(|r| r.strip_suffix(')'))
    {
        let mut out = value.clone();
        for p in split_top_level_commas(args) {
            if let Some(segs) = parse_jq_path(p.trim()) {
                jq_del_path(&mut out, &segs);
            }
        }
        return vec![JqResult::Value(out)];
    }

    // Outer parentheses: `( EXPR )` is just `EXPR`.
    if let Some(inner) = jq_outer_paren(filter) {
        return apply_jq_filter(value, inner.trim());
    }

    // Literals: `null`, `true`, `false`, numbers.
    match filter {
        "null" => return vec![JqResult::Value(Value::Null)],
        "true" => return vec![JqResult::Value(Value::Bool(true))],
        "false" => return vec![JqResult::Value(Value::Bool(false))],
        _ => {}
    }
    if let Ok(n) = filter.parse::<i64>() {
        return vec![JqResult::Value(serde_json::json!(n))];
    }
    if let Ok(f) = filter.parse::<f64>() {
        if f.is_finite() {
            return vec![JqResult::Value(serde_json::json!(f))];
        }
    }

    // `$name` / `$ENV` variable references (with optional `.path`/`[i]` suffix).
    if let Some(rest) = filter.strip_prefix('$') {
        let name_end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        let suffix = rest[name_end..].trim();
        // Only a bare `$x` or a path suffix (`$x.a`, `$x[0]`) is handled here;
        // otherwise fall through so operators (`$x + 1`) can consume it.
        if suffix.is_empty() || suffix.starts_with('.') || suffix.starts_with('[') {
            let base = if name == "ENV" {
                Some(jq_env_object())
            } else if name.is_empty() {
                None
            } else {
                jq_var_get(name)
            };
            if let Some(v) = base {
                if suffix.is_empty() {
                    return vec![JqResult::Value(v)];
                }
                return apply_jq_filter(&v, &format!(".{}", suffix.trim_start_matches('.')));
            }
            return vec![JqResult::Error(format!("$ {name} is not defined"))];
        }
    }

    // `def name(params): body; rest`
    if let Some(rest) = filter.strip_prefix("def ") {
        if let Some(out) = jq_def(value, rest) {
            return out;
        }
    }
    // `reduce EXPR as $x (INIT; UPDATE)`
    if let Some(rest) = filter.strip_prefix("reduce ") {
        if let Some(out) = jq_reduce(value, rest) {
            return out;
        }
    }
    // `foreach EXPR as $x (INIT; UPDATE[; EXTRACT])`
    if let Some(rest) = filter.strip_prefix("foreach ") {
        if let Some(out) = jq_foreach(value, rest) {
            return out;
        }
    }
    // `try EXPR catch HANDLER`
    if let Some(rest) = filter.strip_prefix("try ") {
        let (body, handler) = match find_top_level(rest, " catch ") {
            Some(p) => (rest[..p].trim(), Some(rest[p + 7..].trim())),
            None => (rest.trim(), None),
        };
        let mut out = Vec::new();
        for r in apply_jq_filter(value, body) {
            match r {
                JqResult::Value(v) => out.push(JqResult::Value(v)),
                JqResult::Error(e) => {
                    if let Some(h) = handler {
                        out.extend(apply_jq_filter(&Value::String(e), h));
                    }
                }
            }
        }
        return out;
    }
    // user-defined function call
    if let Some(out) = jq_call_def(value, filter) {
        return out;
    }

    // `if COND then A else B end` (keyword — before pipe/comma).
    if let Some(rest) = filter.strip_prefix("if ") {
        if let Some(out) = jq_if(value, rest) {
            return out;
        }
    }

    // Top-level pipe `a | b` (lowest precedence). Also handles `a as $x | b`.
    if let Some(pos) = find_top_level(filter, "|") {
        let l = filter[..pos].trim();
        let r = filter[pos + 1..].trim();
        if let Some((e, name)) = jq_as_binding(l) {
            let mut out = Vec::new();
            for res in apply_jq_filter(value, e) {
                match res {
                    JqResult::Value(v) => {
                        let _g = jq_push_scope(vec![(name.clone(), v)]);
                        out.extend(apply_jq_filter(value, r));
                    }
                    err => out.push(err),
                }
            }
            return out;
        }
        let mut out = Vec::new();
        for res in apply_jq_filter(value, l) {
            match res {
                JqResult::Value(v) => out.extend(apply_jq_filter(&v, r)),
                e => out.push(e),
            }
        }
        return out;
    }

    // `a, b` — comma produces multiple outputs.
    if let Some(parts) = split_top_level(filter, ',') {
        if parts.len() > 1 {
            let mut out = Vec::new();
            for p in parts {
                out.extend(apply_jq_filter(value, p.trim()));
            }
            return out;
        }
    }

    // `a // b` — alternative (must precede binary `/`).
    if let Some(pos) = find_top_level(filter, "//") {
        let l = filter[..pos].trim();
        let r = filter[pos + 2..].trim();
        let lr = apply_jq_filter(value, l);
        let ok = lr
            .iter()
            .any(|x| matches!(x, JqResult::Value(v) if !v.is_null() && *v != Value::Bool(false)));
        if ok {
            return lr;
        }
        return apply_jq_filter(value, r);
    }

    // `expr?` — suppress errors.
    if let Some(inner) = filter.strip_suffix('?') {
        return apply_jq_filter(value, inner.trim())
            .into_iter()
            .filter(|r| matches!(r, JqResult::Value(_)))
            .collect();
    }

    // Binary operators: arithmetic (`+ - * / %`), comparison (`== != < <= > >=`)
    // and string concatenation via `+`.
    if let Some((pos, op)) = find_jq_binop(filter) {
        let lhs_s = filter[..pos].trim();
        let rhs_s = filter[pos + op.len()..].trim();
        if !lhs_s.is_empty() && !rhs_s.is_empty() {
            let lhs = jq_operand(value, lhs_s);
            let rhs = jq_operand(value, rhs_s);
            return vec![jq_binop(lhs, op, rhs)];
        }
    }

    // String literal with interpolation: `"text \(expr) more"`.
    if filter.len() >= 2 && filter.starts_with('"') && filter.ends_with('"') {
        return match jq_interpolate(value, &filter[1..filter.len() - 1]) {
            Ok(s) => vec![JqResult::Value(Value::String(s))],
            Err(e) => vec![JqResult::Error(e)],
        };
    }

    // Built-ins (with/without args) + `@format` strings.
    if let Some(out) = jq_builtin(value, filter) {
        return out;
    }

    // `<builtin>[…]` — apply the builtin, then index the resulting array, e.g.
    // `keys[0]`, `keys[-1]`, `keys[]`. Restricted to the simple builtins (not
    // paths) so the existing path / `.[…]` handling is untouched.
    if filter.ends_with(']') {
        if let Some(open) = filter.rfind('[') {
            let base = filter[..open].trim();
            if matches!(base, "keys" | "length" | "type") {
                let idx = filter[open + 1..filter.len() - 1].trim();
                let mut out = Vec::new();
                for r in apply_jq_filter(value, base) {
                    match r {
                        JqResult::Value(Value::Array(a)) => {
                            if idx.is_empty() {
                                out.extend(a.into_iter().map(JqResult::Value));
                            } else if let Ok(n) = idx.parse::<isize>() {
                                let len = a.len() as isize;
                                let k = if n < 0 { len + n } else { n };
                                if k >= 0 && k < len {
                                    out.push(JqResult::Value(a[k as usize].clone()));
                                }
                            } else {
                                return vec![JqResult::Error(format!(
                                    "unsupported index: {}",
                                    &filter[open..]
                                ))];
                            }
                        }
                        other => out.push(other),
                    }
                }
                return out;
            }
        }
    }

    // Built-in filters: length / keys / type.
    match filter {
        "length" => {
            let n = match value {
                Value::Array(a) => a.len(),
                Value::Object(o) => o.len(),
                Value::String(s) => s.chars().count(),
                Value::Null => 0,
                _ => return vec![],
            };
            return vec![JqResult::Value(serde_json::json!(n))];
        }
        "keys" => {
            let keys: Vec<Value> = match value {
                Value::Object(o) => o.keys().map(|k| Value::String(k.clone())).collect(),
                Value::Array(a) => (0..a.len()).map(|i| serde_json::json!(i)).collect(),
                _ => return vec![],
            };
            return vec![JqResult::Value(Value::Array(keys))];
        }
        "type" => {
            let t = match value {
                Value::Null => "null",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            return vec![JqResult::Value(Value::String(t.to_string()))];
        }
        _ => {}
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
                            // A pipe stage is either a path (`.a.b`), a built-in
                            // function (`length`), or a string literal.
                            let sub = if looks_like_jq_function(part)
                                || (part.starts_with('"') && part.ends_with('"'))
                            {
                                apply_jq_filter(&v, part)
                            } else {
                                apply_jq_filter(&v, &format!(".{}", part))
                            };
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
            Value::Array(_) if key.is_empty() => {
                return apply_jq_array_op(value, &array_op);
            }
            _ => return vec![],
        }
    }

    if filter == "[]" {
        return apply_jq_array_op(value, "[]");
    }

    if filter.starts_with('[') && filter.ends_with(']') && filter.len() >= 2 {
        let inner = &filter[1..filter.len() - 1];
        let mut out = Vec::new();
        for part in split_top_level(inner, ',').unwrap_or_else(|| vec![inner]) {
            if part.trim().is_empty() {
                continue;
            }
            for r in apply_jq_filter(value, part.trim()) {
                if let JqResult::Value(v) = r {
                    out.push(v);
                }
            }
        }
        return vec![JqResult::Value(Value::Array(out))];
    }

    if filter.starts_with('{') && filter.ends_with('}') && filter.len() >= 2 {
        let inner = &filter[1..filter.len() - 1];
        let mut obj = serde_json::Map::new();
        for part in split_top_level(inner, ',').unwrap_or_else(|| vec![inner]) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(colon) = find_top_level(part, ":") {
                let k = part[..colon].trim().trim_matches('"').to_string();
                let vs = part[colon + 1..].trim();
                if let Some(JqResult::Value(v)) = apply_jq_filter(value, vs).into_iter().next() {
                    obj.insert(k, v);
                }
            } else {
                let k = part.trim_matches('"').to_string();
                if let Some(JqResult::Value(v)) =
                    apply_jq_filter(value, &format!(".{k}")).into_iter().next()
                {
                    obj.insert(k, v);
                }
            }
        }
        return vec![JqResult::Value(Value::Object(obj))];
    }

    return vec![JqResult::Error(format!("unsupported filter: {}", filter))];
}

/// Built-in jq filters supported by the minimal engine.
fn is_jq_function(name: &str) -> bool {
    matches!(name, "length" | "keys" | "type")
}

/// Heuristic: is a pipe stage a function/builtin (vs a path `.foo`)?
fn looks_like_jq_function(part: &str) -> bool {
    let p = part.trim();
    if is_jq_function(p) || p.contains('(') || p.starts_with('@') || p.starts_with("if ") {
        return true;
    }
    matches!(
        p,
        "empty"
            | "first"
            | "last"
            | "reverse"
            | "sort"
            | "unique"
            | "values"
            | "to_entries"
            | "from_entries"
            | "add"
            | "flatten"
            | "min"
            | "max"
            | "not"
            | "tonumber"
            | "tostring"
            | "ascii_downcase"
            | "ascii_upcase"
            | "floor"
            | "ceil"
            | "round"
            | "abs"
            | "any"
            | "all"
    )
}

/// Evaluate a jq string body: expand `\(expr)` interpolations and the usual
/// backslash escapes.
fn jq_interpolate(value: &Value, s: &str) -> std::result::Result<String, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                match chars[i + 1] {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    '/' => out.push('/'),
                    '(' => {
                        // \(expr) — find the matching ')'.
                        let mut depth = 1usize;
                        let mut j = i + 2;
                        while j < chars.len() {
                            match chars[j] {
                                '(' => depth += 1,
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                        if j >= chars.len() {
                            return Err("unterminated \\( in string".to_string());
                        }
                        let expr: String = chars[i + 2..j].iter().collect();
                        let mut parts: Vec<String> = Vec::new();
                        for r in apply_jq_filter(value, expr.trim()) {
                            match r {
                                JqResult::Value(Value::String(s)) => parts.push(s),
                                JqResult::Value(Value::Null) => parts.push(String::new()),
                                JqResult::Value(v) => parts.push(v.to_string()),
                                JqResult::Error(e) => return Err(e),
                            }
                        }
                        out.push_str(&parts.join(" "));
                        i = j + 1;
                        continue;
                    }
                    c => {
                        out.push('\\');
                        out.push(c);
                    }
                }
                i += 2;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
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
        let dir = std::env::temp_dir().join(format!(
            "fastshell_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
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

/// Finds a top-level binary operator in a jq filter (outside quotes/brackets).
fn find_jq_binop(s: &str) -> Option<(usize, &'static str)> {
    let b = s.as_bytes();
    let mut d = 0i32;
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            in_str = !in_str;
            i += 1;
            continue;
        }
        if in_str {
            i += if c == b'\\' { 2 } else { 1 };
            continue;
        }
        match c {
            b'(' | b'[' | b'{' => d += 1,
            b')' | b']' | b'}' => d -= 1,
            _ => {}
        }
        if d == 0 {
            let rest = &s[i..];
            for op in ["==", "!=", "<=", ">="] {
                if rest.starts_with(op) {
                    return Some((i, op));
                }
            }
            match c {
                b'+' => return Some((i, "+")),
                b'-' => return Some((i, "-")),
                b'*' => return Some((i, "*")),
                b'%' => return Some((i, "%")),
                b'<' => return Some((i, "<")),
                b'>' => return Some((i, ">")),
                b'/' if b.get(i + 1) != Some(&b'/') => return Some((i, "/")),
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Evaluates a jq operand: a number/string literal, or a filter against `value`.
fn jq_operand(value: &Value, expr: &str) -> Value {
    let e = expr.trim();
    if let Ok(n) = e.parse::<f64>() {
        return jq_number(n);
    }
    if e.len() >= 2 && e.starts_with('"') && e.ends_with('"') {
        return Value::String(e[1..e.len() - 1].to_string());
    }
    match apply_jq_filter(value, e).into_iter().next() {
        Some(JqResult::Value(v)) => v,
        _ => Value::Null,
    }
}

fn jq_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.is_finite() && n.abs() < 9.0e15 {
        serde_json::json!(n as i64)
    } else {
        serde_json::json!(n)
    }
}

fn jq_binop(a: Value, op: &str, b: Value) -> JqResult {
    use serde_json::Value::*;
    let num = |v: &Value| -> Option<f64> {
        match v {
            Number(n) => n.as_f64(),
            _ => None,
        }
    };
    let arith = |x: f64, y: f64| -> JqResult {
        let r = match op {
            "+" => x + y,
            "-" => x - y,
            "*" => x * y,
            "/" => {
                if y == 0.0 {
                    return JqResult::Error("division by zero".into());
                }
                x / y
            }
            "%" => {
                if y == 0.0 {
                    return JqResult::Error("division by zero".into());
                }
                x % y
            }
            _ => x,
        };
        JqResult::Value(jq_number(r))
    };
    match op {
        "+" => {
            if let (String(x), String(y)) = (&a, &b) {
                JqResult::Value(String(format!("{x}{y}")))
            } else {
                match (num(&a), num(&b)) {
                    (Some(x), Some(y)) => arith(x, y),
                    _ => JqResult::Error("+ expects two numbers or two strings".into()),
                }
            }
        }
        "-" | "*" | "/" | "%" => match (num(&a), num(&b)) {
            (Some(x), Some(y)) => arith(x, y),
            _ => JqResult::Error(format!("{op} expects two numbers")),
        },
        "==" => JqResult::Value(Bool(a == b)),
        "!=" => JqResult::Value(Bool(a != b)),
        "<" | "<=" | ">" | ">=" => {
            let ord = if let (Some(x), Some(y)) = (num(&a), num(&b)) {
                x.partial_cmp(&y)
            } else if let (String(x), String(y)) = (&a, &b) {
                Some(x.cmp(y))
            } else {
                None
            };
            match ord {
                Some(o) => {
                    use std::cmp::Ordering::*;
                    let r = match op {
                        "<" => o == Less,
                        "<=" => o != Greater,
                        ">" => o == Greater,
                        _ => o != Less,
                    };
                    JqResult::Value(Bool(r))
                }
                None => JqResult::Error(format!("{op}: incompatible types")),
            }
        }
        _ => JqResult::Error(format!("unsupported operator: {op}")),
    }
}

// ── extended jq built-ins ────────────────────────────────────────────

/// Finds a top-level occurrence of `op` (outside quotes / brackets / parens).
/// If `s` is entirely wrapped in one balanced `( ... )`, return the inner text.
fn jq_outer_paren(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if b.first() != Some(&b'(') {
        return None;
    }
    let mut d = 0i32;
    let mut in_str = false;
    for (i, &c) in b.iter().enumerate() {
        if c == b'"' {
            in_str = !in_str;
            continue;
        }
        if in_str {
            continue;
        }
        match c {
            b'(' => d += 1,
            b')' => {
                d -= 1;
                if d == 0 {
                    return if i == b.len() - 1 {
                        Some(&s[1..i])
                    } else {
                        None
                    };
                }
            }
            _ => {}
        }
    }
    None
}

fn find_top_level(s: &str, op: &str) -> Option<usize> {
    let b = s.as_bytes();
    let ob = op.as_bytes();
    let mut d = 0i32;
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            in_str = !in_str;
            i += 1;
            continue;
        }
        if in_str {
            i += if c == b'\\' { 2 } else { 1 };
            continue;
        }
        match c {
            b'(' | b'[' | b'{' => d += 1,
            b')' | b']' | b'}' => d -= 1,
            _ => {}
        }
        if d == 0 && i + ob.len() <= b.len() && &b[i..i + ob.len()] == ob {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Splits on a top-level separator, returning None when there is no separator.
fn split_top_level(s: &str, sep: char) -> Option<Vec<&str>> {
    let sb = [sep as u8];
    let mut parts = Vec::new();
    let mut start = 0;
    let mut rest = s;
    while let Some(pos) = find_top_level(rest, std::str::from_utf8(&sb).ok()?) {
        parts.push(&s[start..start + pos]);
        start += pos + 1;
        rest = &s[start..];
    }
    if parts.is_empty() {
        None
    } else {
        parts.push(&s[start..]);
        Some(parts)
    }
}

fn jq_arg(filter: &str, name: &str) -> Option<String> {
    let r = filter.strip_prefix(name)?.trim_start();
    let r = r.strip_prefix('(')?.trim_end().strip_suffix(')')?;
    Some(r.trim().to_string())
}

fn jq_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => Some(String::new()),
        _ => None,
    }
}

fn jq_truthy(v: &Value) -> bool {
    !v.is_null() && *v != Value::Bool(false)
}

/// `if COND then A else B end` (nested `if` handled recursively).
fn jq_if(value: &Value, rest: &str) -> Option<Vec<JqResult>> {
    let then_pos = find_top_level(rest, " then ")?;
    let cond = rest[..then_pos].trim();
    let after = rest[then_pos + 6..].trim();
    let c = apply_jq_filter(value, cond);
    let taken = c
        .iter()
        .any(|r| matches!(r, JqResult::Value(v) if jq_truthy(v)));
    // Earliest top-level ` elif ` / ` else ` bounds the taken branch.
    let stop = [" elif ", " else "]
        .iter()
        .filter_map(|k| find_top_level(after, k))
        .min();
    if taken {
        let body = match stop {
            Some(p) => &after[..p],
            None => after.trim_end().strip_suffix("end").unwrap_or(after),
        };
        return Some(apply_jq_filter(value, body.trim()));
    }
    if let Some(p) = find_top_level(after, " elif ") {
        return jq_if(value, after[p + 6..].trim());
    }
    if let Some(p) = find_top_level(after, " else ") {
        let mut e = &after[p + 6..];
        if let Some(stripped) = e.trim_end().strip_suffix("end") {
            e = stripped;
        }
        return Some(apply_jq_filter(value, e.trim()));
    }
    Some(vec![])
}

/// Handles jq built-ins. Returns None when `filter` is not one of them.
fn jq_builtin(value: &Value, filter: &str) -> Option<Vec<JqResult>> {
    use serde_json::json;
    let one = |v: Value| Some(vec![JqResult::Value(v)]);
    // `@format`
    if let Some(fmt) = filter.strip_prefix('@') {
        return Some(match fmt {
            "json" => vec![JqResult::Value(Value::String(value.to_string()))],
            "text" => vec![JqResult::Value(Value::String(
                jq_str(value).unwrap_or_else(|| value.to_string()),
            ))],
            "base64" => {
                use base64::Engine;
                let s = jq_str(value).unwrap_or_else(|| value.to_string());
                vec![JqResult::Value(Value::String(
                    base64::engine::general_purpose::STANDARD.encode(s),
                ))]
            }
            "csv" | "tsv" => {
                let sep = if fmt == "csv" { "," } else { "\t" };
                if let Value::Array(a) = value {
                    let row: Vec<String> = a
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => format!("\"{}\"", s.replace('"', "\"\"")),
                            other => other.to_string(),
                        })
                        .collect();
                    vec![JqResult::Value(Value::String(row.join(sep)))]
                } else {
                    vec![]
                }
            }
            "uri" => {
                let s = jq_str(value).unwrap_or_default();
                let mut o = String::new();
                for b in s.bytes() {
                    if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                        o.push(b as char);
                    } else {
                        o.push_str(&format!("%{:02X}", b));
                    }
                }
                vec![JqResult::Value(Value::String(o))]
            }
            "sh" => {
                let s = jq_str(value).unwrap_or_else(|| value.to_string());
                vec![JqResult::Value(Value::String(format!(
                    "'{}'",
                    s.replace('\'', "'\\''")
                )))]
            }
            "html" => {
                let s = jq_str(value).unwrap_or_else(|| value.to_string());
                let esc = s
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&#39;");
                vec![JqResult::Value(Value::String(esc))]
            }
            _ => vec![JqResult::Error(format!("unsupported @format: @{fmt}"))],
        });
    }
    match filter {
        "empty" => return Some(vec![]),
        "first" => {
            return Some(match value {
                Value::Array(a) => a
                    .first()
                    .cloned()
                    .map(JqResult::Value)
                    .into_iter()
                    .collect(),
                _ => vec![],
            })
        }
        "last" => {
            return Some(match value {
                Value::Array(a) => a.last().cloned().map(JqResult::Value).into_iter().collect(),
                _ => vec![],
            })
        }
        "reverse" => {
            return Some(match value {
                Value::Array(a) => {
                    let mut v = a.clone();
                    v.reverse();
                    vec![JqResult::Value(Value::Array(v))]
                }
                Value::String(s) => {
                    vec![JqResult::Value(Value::String(s.chars().rev().collect()))]
                }
                _ => vec![],
            })
        }
        "sort" => {
            return Some(match value {
                Value::Array(a) => {
                    let mut v = a.clone();
                    v.sort_by_key(|x| x.to_string());
                    vec![JqResult::Value(Value::Array(v))]
                }
                _ => vec![],
            })
        }
        "unique" => {
            return Some(match value {
                Value::Array(a) => {
                    let mut seen = Vec::new();
                    for v in a {
                        if !seen.contains(v) {
                            seen.push(v.clone());
                        }
                    }
                    vec![JqResult::Value(Value::Array(seen))]
                }
                _ => vec![],
            })
        }
        "values" => {
            return Some(match value {
                Value::Object(o) => {
                    vec![JqResult::Value(Value::Array(o.values().cloned().collect()))]
                }
                _ => vec![],
            })
        }
        "to_entries" => {
            return Some(match value {
                Value::Object(o) => vec![JqResult::Value(Value::Array(
                    o.iter()
                        .map(|(k, v)| json!({"key": k, "value": v}))
                        .collect(),
                ))],
                Value::Array(a) => vec![JqResult::Value(Value::Array(
                    a.iter()
                        .enumerate()
                        .map(|(i, v)| json!({"key": i, "value": v}))
                        .collect(),
                ))],
                _ => vec![],
            })
        }
        "from_entries" => {
            return Some(match value {
                Value::Array(a) => {
                    let mut o = serde_json::Map::new();
                    for e in a {
                        let k = e
                            .get("key")
                            .or_else(|| e.get("k"))
                            .map(|k| jq_str(k).unwrap_or_default());
                        let v = e.get("value").or_else(|| e.get("v")).cloned();
                        if let (Some(k), Some(v)) = (k, v) {
                            o.insert(k, v);
                        }
                    }
                    vec![JqResult::Value(Value::Object(o))]
                }
                _ => vec![],
            })
        }
        "add" => {
            return Some(match value {
                Value::Array(a) => {
                    let mut acc = Value::Null;
                    for v in a {
                        acc = match (acc, v) {
                            (Value::Null, x) => x.clone(),
                            (Value::Number(x), Value::Number(y)) => {
                                let s = x.as_f64().unwrap_or(0.0) + y.as_f64().unwrap_or(0.0);
                                jq_number(s)
                            }
                            (Value::String(x), Value::String(y)) => Value::String(x + y),
                            (Value::Array(mut x), Value::Array(y)) => {
                                x.extend(y.clone());
                                Value::Array(x)
                            }
                            (Value::Object(mut x), Value::Object(y)) => {
                                for (k, v) in y {
                                    x.insert(k.clone(), v.clone());
                                }
                                Value::Object(x)
                            }
                            (a, _) => a,
                        };
                    }
                    vec![JqResult::Value(acc)]
                }
                _ => vec![],
            })
        }
        "flatten" => {
            fn fl(v: &Value, out: &mut Vec<Value>) {
                match v {
                    Value::Array(a) => a.iter().for_each(|x| fl(x, out)),
                    x => out.push(x.clone()),
                }
            }
            return Some(match value {
                Value::Array(a) => {
                    let mut out = Vec::new();
                    a.iter().for_each(|x| fl(x, &mut out));
                    vec![JqResult::Value(Value::Array(out))]
                }
                _ => vec![],
            });
        }
        "min" | "max" => {
            return Some(match value {
                Value::Array(a) => {
                    let pick = a
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| (f, v.clone())))
                        .reduce(|x, y| {
                            if (filter == "min" && y.0 < x.0) || (filter == "max" && y.0 > x.0) {
                                y
                            } else {
                                x
                            }
                        })
                        .map(|(_, v)| v);
                    pick.map(JqResult::Value).into_iter().collect()
                }
                _ => vec![],
            });
        }
        "not" => return one(Value::Bool(!jq_truthy(value))),
        "tonumber" => {
            return Some(vec![match value {
                Value::String(s) => s
                    .trim()
                    .parse::<f64>()
                    .map(jq_number)
                    .map(JqResult::Value)
                    .unwrap_or_else(|_| JqResult::Error(format!("cannot parse {s:?} as number"))),
                Value::Number(_) => JqResult::Value(value.clone()),
                _ => JqResult::Error("tonumber: not a string/number".into()),
            }])
        }
        "tostring" => {
            return one(Value::String(
                jq_str(value).unwrap_or_else(|| value.to_string()),
            ))
        }
        "ascii_downcase" => {
            return one(Value::String(
                jq_str(value).unwrap_or_default().to_ascii_lowercase(),
            ))
        }
        "ascii_upcase" => {
            return one(Value::String(
                jq_str(value).unwrap_or_default().to_ascii_uppercase(),
            ))
        }
        "floor" => return one(jq_number(value.as_f64().unwrap_or(0.0).floor())),
        "ceil" => return one(jq_number(value.as_f64().unwrap_or(0.0).ceil())),
        "round" => return one(jq_number(value.as_f64().unwrap_or(0.0).round())),
        "abs" => return one(jq_number(value.as_f64().unwrap_or(0.0).abs())),
        _ => {}
    }
    // Function-with-argument forms.
    if let Some(arg) = jq_arg(filter, "map") {
        if let Value::Array(a) = value {
            let mut out = Vec::new();
            for v in a {
                out.extend(apply_jq_filter(v, &arg));
            }
            return one(Value::Array(
                out.into_iter()
                    .filter_map(|r| match r {
                        JqResult::Value(v) => Some(v),
                        _ => None,
                    })
                    .collect(),
            ));
        }
        return Some(vec![]);
    }
    if let Some(arg) = jq_arg(filter, "select") {
        let r = apply_jq_filter(value, &arg);
        let keep = r
            .iter()
            .any(|x| matches!(x, JqResult::Value(v) if jq_truthy(v)));
        return if keep {
            one(value.clone())
        } else {
            Some(vec![])
        };
    }
    if let Some(arg) = jq_arg(filter, "has") {
        let key = jq_operand(value, &arg);
        let has = match (value, &key) {
            (Value::Object(o), Value::String(k)) => o.contains_key(k),
            (Value::Array(a), Value::Number(n)) => {
                n.as_u64().map(|i| (i as usize) < a.len()).unwrap_or(false)
            }
            _ => false,
        };
        return one(Value::Bool(has));
    }
    if let Some(arg) = jq_arg(filter, "join") {
        let sep = jq_operand(value, &arg);
        let sep = jq_str(&sep).unwrap_or_default();
        return Some(match value {
            Value::Array(a) => one(Value::String(
                a.iter()
                    .map(|v| jq_str(v).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(&sep),
            ))
            .unwrap(),
            _ => vec![],
        });
    }
    if let Some(arg) = jq_arg(filter, "split") {
        let sep = jq_operand(value, &arg);
        let sep = jq_str(&sep).unwrap_or_default();
        return one(Value::Array(
            jq_str(value)
                .unwrap_or_default()
                .split(&sep)
                .map(|s| Value::String(s.to_string()))
                .collect(),
        ));
    }
    for (name, mode) in [("contains", 0), ("startswith", 1), ("endswith", 2)] {
        if let Some(arg) = jq_arg(filter, name) {
            let a = jq_operand(value, &arg);
            let r = match (value, &a) {
                (Value::String(s), Value::String(sub)) => match mode {
                    1 => s.starts_with(sub),
                    2 => s.ends_with(sub),
                    _ => s.contains(sub),
                },
                (Value::Array(arr), _) => match &a {
                    Value::Array(sub) => sub.iter().all(|x| arr.contains(x)),
                    x => arr.contains(x),
                },
                _ => false,
            };
            return one(Value::Bool(r));
        }
    }
    if filter == "any" || filter == "all" {
        if let Value::Array(a) = value {
            let r = if filter == "any" {
                a.iter().any(jq_truthy)
            } else {
                a.iter().all(jq_truthy)
            };
            return one(Value::Bool(r));
        }
    }
    if filter == "env" {
        return one(jq_env_object());
    }
    if filter == "explode" {
        return one(Value::Array(
            jq_str(value)
                .unwrap_or_default()
                .chars()
                .map(|c| json!(c as u32))
                .collect(),
        ));
    }
    if filter == "implode" {
        let s: String = value
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_u64())
                    .filter_map(|u| char::from_u32(u as u32))
                    .collect()
            })
            .unwrap_or_default();
        return one(Value::String(s));
    }
    if filter == "ascii" {
        return one(json!(jq_str(value)
            .unwrap_or_default()
            .chars()
            .next()
            .map(|c| c as u32)
            .unwrap_or(0)));
    }
    if let Some(arg) = jq_arg(filter, "sort_by") {
        return Some(jq_sort_by(value, &arg));
    }
    if let Some(arg) = jq_arg(filter, "group_by") {
        return Some(jq_group_by(value, &arg));
    }
    if let Some(arg) = jq_arg(filter, "unique_by") {
        return Some(jq_unique_by(value, &arg));
    }
    if let Some(arg) = jq_arg(filter, "min_by") {
        return Some(jq_minmax_by(value, &arg, true));
    }
    if let Some(arg) = jq_arg(filter, "max_by") {
        return Some(jq_minmax_by(value, &arg, false));
    }
    if let Some(arg) = jq_arg(filter, "with_entries") {
        return Some(jq_with_entries(value, &arg));
    }
    if let Some(arg) = jq_arg(filter, "any") {
        return Some(jq_anyall(value, &arg, true));
    }
    if let Some(arg) = jq_arg(filter, "all") {
        return Some(jq_anyall(value, &arg, false));
    }
    if let Some(arg) = jq_arg(filter, "ltrimstr") {
        let p = jq_str(&jq_operand(value, &arg)).unwrap_or_default();
        return one(Value::String(
            jq_str(value)
                .unwrap_or_default()
                .trim_start_matches(&p)
                .to_string(),
        ));
    }
    if let Some(arg) = jq_arg(filter, "rtrimstr") {
        let p = jq_str(&jq_operand(value, &arg)).unwrap_or_default();
        return one(Value::String(
            jq_str(value)
                .unwrap_or_default()
                .trim_end_matches(&p)
                .to_string(),
        ));
    }
    if let Some(arg) = jq_arg(filter, "index") {
        let needle = jq_operand(value, &arg);
        return one(jq_index_of(value, &needle, false));
    }
    if let Some(arg) = jq_arg(filter, "rindex") {
        let needle = jq_operand(value, &arg);
        return one(jq_index_of(value, &needle, true));
    }
    if let Some(arg) = jq_arg(filter, "indices") {
        let needle = jq_operand(value, &arg);
        return one(jq_indices(value, &needle));
    }
    if let Some(arg) = jq_arg(filter, "range") {
        return Some(jq_range(&arg));
    }
    None
}

// ── jq assignment / path helpers ─────────────────────────────────────────

#[derive(Clone, Debug)]
enum JqPathSeg {
    Key(String),
    Index(usize),
}

/// Find a top-level assignment operator (`=`, `|=`, `+=`, `-=`, `*=`, `/=`),
/// skipping `==`/`!=`/`<=`/`>=` and anything inside quotes / parens / brackets.
fn find_top_level_assign(filter: &str) -> Option<(&str, &str, &str)> {
    let b = filter.as_bytes();
    let mut depth = 0i32;
    let mut dq = false;
    let mut sq = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' if !sq => dq = !dq,
            b'\'' if !dq => sq = !sq,
            b'(' | b'[' if !dq && !sq => depth += 1,
            b')' | b']' if !dq && !sq => depth -= 1,
            b'=' if !dq && !sq && depth == 0 => {
                let prev = if i > 0 { b[i - 1] } else { 0 };
                let next = if i + 1 < b.len() { b[i + 1] } else { 0 };
                // comparisons
                if next == b'=' || prev == b'=' || prev == b'!' || prev == b'<' || prev == b'>' {
                    i += 1;
                    continue;
                }
                let (path, op) = match prev {
                    b'|' => (&filter[..i - 1], "|="),
                    b'+' => (&filter[..i - 1], "+="),
                    b'-' => (&filter[..i - 1], "-="),
                    b'*' => (&filter[..i - 1], "*="),
                    b'/' => (&filter[..i - 1], "/="),
                    _ => (&filter[..i], "="),
                };
                let rhs = &filter[i + 1..];
                if path.trim().is_empty() {
                    return None;
                }
                return Some((path.trim(), op, rhs.trim()));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Parse `.a.b[0]["k"]` into path segments. Returns `None` for non-path filters.
fn parse_jq_path(p: &str) -> Option<Vec<JqPathSeg>> {
    let p = p.trim();
    if p.is_empty() || p == "." {
        return Some(Vec::new());
    }
    let bytes = p.as_bytes();
    let mut segs = Vec::new();
    let mut i = 0;
    if bytes[0] == b'.' {
        i = 1;
    }
    while i < bytes.len() {
        match bytes[i] {
            b'.' => {
                i += 1;
            }
            b'[' => {
                let close = p[i..].find(']')? + i;
                let inner = p[i + 1..close]
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'');
                if let Ok(n) = inner.parse::<usize>() {
                    segs.push(JqPathSeg::Index(n));
                } else {
                    segs.push(JqPathSeg::Key(inner.to_string()));
                }
                i = close + 1;
            }
            _ => {
                let start = i;
                while i < bytes.len() && bytes[i] != b'.' && bytes[i] != b'[' {
                    i += 1;
                }
                let key = p[start..i].trim();
                if key.is_empty() {
                    return None;
                }
                segs.push(JqPathSeg::Key(key.to_string()));
            }
        }
    }
    Some(segs)
}

fn jq_get_path(value: &Value, segs: &[JqPathSeg]) -> Value {
    let mut cur = value;
    for seg in segs {
        match seg {
            JqPathSeg::Key(k) => match cur.get(k) {
                Some(v) => cur = v,
                None => return Value::Null,
            },
            JqPathSeg::Index(n) => match cur.get(n) {
                Some(v) => cur = v,
                None => return Value::Null,
            },
        }
    }
    cur.clone()
}

fn jq_set_path(value: &mut Value, segs: &[JqPathSeg], new: Value) -> bool {
    if segs.is_empty() {
        *value = new;
        return true;
    }
    let (first, rest) = segs.split_first().unwrap();
    match first {
        JqPathSeg::Key(k) => {
            if !value.is_object() {
                *value = Value::Object(serde_json::Map::new());
            }
            let obj = value.as_object_mut().unwrap();
            if rest.is_empty() {
                obj.insert(k.clone(), new);
                true
            } else {
                let entry = obj.entry(k.clone()).or_insert(Value::Null);
                jq_set_path(entry, rest, new)
            }
        }
        JqPathSeg::Index(n) => {
            if !value.is_array() {
                *value = Value::Array(Vec::new());
            }
            let arr = value.as_array_mut().unwrap();
            while arr.len() <= *n {
                arr.push(Value::Null);
            }
            if rest.is_empty() {
                arr[*n] = new;
                true
            } else {
                jq_set_path(&mut arr[*n], rest, new)
            }
        }
    }
}

fn jq_del_path(value: &mut Value, segs: &[JqPathSeg]) {
    if segs.is_empty() {
        return;
    }
    let (first, rest) = segs.split_first().unwrap();
    if rest.is_empty() {
        match first {
            JqPathSeg::Key(k) => {
                if let Some(o) = value.as_object_mut() {
                    o.remove(k);
                }
            }
            JqPathSeg::Index(n) => {
                if let Some(a) = value.as_array_mut() {
                    if *n < a.len() {
                        a.remove(*n);
                    }
                }
            }
        }
        return;
    }
    match first {
        JqPathSeg::Key(k) => {
            if let Some(v) = value.get_mut(k) {
                jq_del_path(v, rest);
            }
        }
        JqPathSeg::Index(n) => {
            if let Some(v) = value.get_mut(n) {
                jq_del_path(v, rest);
            }
        }
    }
}

fn jq_try_assignment(value: &Value, filter: &str) -> Option<Vec<JqResult>> {
    let (path_s, op, rhs) = find_top_level_assign(filter)?;
    let segs = parse_jq_path(path_s)?;
    let cur = jq_get_path(value, &segs);
    let newv = match op {
        "=" => match apply_jq_filter(value, rhs).into_iter().next() {
            Some(JqResult::Value(v)) => v,
            _ => return Some(vec![JqResult::Error("assignment RHS error".into())]),
        },
        "|=" => match apply_jq_filter(&cur, rhs).into_iter().next() {
            Some(JqResult::Value(v)) => v,
            _ => return Some(vec![JqResult::Error("update RHS error".into())]),
        },
        "+=" | "-=" | "*=" | "/=" => {
            let o = &op[..1];
            let rv = match apply_jq_filter(value, rhs).into_iter().next() {
                Some(JqResult::Value(v)) => v,
                _ => return Some(vec![JqResult::Error("assignment RHS error".into())]),
            };
            match jq_binop(cur.clone(), o, rv) {
                JqResult::Value(v) => v,
                JqResult::Error(e) => return Some(vec![JqResult::Error(e)]),
                other => return Some(vec![other]),
            }
        }
        _ => return None,
    };
    let mut out = value.clone();
    if jq_set_path(&mut out, &segs, newv) {
        Some(vec![JqResult::Value(out)])
    } else {
        Some(vec![JqResult::Error(format!("cannot set path: {path_s}"))])
    }
}

/// Split on top-level commas (respecting quotes / parens / brackets).
fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut dq = false;
    let mut sq = false;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '"' if !sq => {
                dq = !dq;
                cur.push(c);
            }
            '\'' if !dq => {
                sq = !sq;
                cur.push(c);
            }
            '(' | '[' if !dq && !sq => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' if !dq && !sq => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 && !dq && !sq => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}
