// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::collections::HashMap;

const AWK_HELP_TEXT: &str = "\
awk: pattern scanning and processing language
Usage: awk [OPTIONS] 'program' [FILE...]
Options:
  -F SEP      Set field separator character
  -v VAR=VAL  Assign value to variable
  -h, --help  Show this help message
";

impl Shell {
    pub fn cmd_awk(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(AWK_HELP_TEXT.to_string());
        }
        let mut program: Option<String> = None;
        let mut files = Vec::new();
        let mut field_sep = ' ';
        let mut fs_arg: Option<String> = None;
        let mut variables: HashMap<String, f64> = HashMap::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-F" => {
                    if i + 1 < args.len() {
                        field_sep = args[i + 1].chars().next().unwrap_or(' ');
                        fs_arg = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-v" => {
                    if i + 1 < args.len() {
                        let assign = args[i + 1];
                        if let Some(eq) = assign.find('=') {
                            let var = assign[..eq].trim().to_string();
                            let val: f64 = assign[eq + 1..].trim().parse().unwrap_or(0.0);
                            variables.insert(var, val);
                        }
                        i += 1;
                    }
                }
                arg if arg.starts_with("-v") && arg.len() > 2 => {
                    let assign = &arg[2..];
                    if let Some(eq) = assign.find('=') {
                        let var = assign[..eq].trim().to_string();
                        let val: f64 = assign[eq + 1..].trim().parse().unwrap_or(0.0);
                        variables.insert(var, val);
                    }
                }
                arg if arg.starts_with("-F") && arg.len() > 2 => {
                    field_sep = arg[2..].chars().next().unwrap_or(' ');
                    fs_arg = Some(arg[2..].to_string());
                }
                // `awk -f script.awk` — read the program from a file.
                "-f" => {
                    if i + 1 < args.len() {
                        let file = args[i + 1];
                        match self.vfs.read_to_string(file, &self.cwd) {
                            Ok(c) => program = Some(c),
                            Err(e) => {
                                return CommandOutput::error(format!("awk: {}: {}\n", file, e), 1)
                            }
                        }
                        i += 1;
                    }
                }
                arg if arg.starts_with("-f") && arg.len() > 2 => {
                    let file = &arg[2..];
                    match self.vfs.read_to_string(file, &self.cwd) {
                        Ok(c) => program = Some(c),
                        Err(e) => {
                            return CommandOutput::error(format!("awk: {}: {}\n", file, e), 1)
                        }
                    }
                }
                arg if !arg.starts_with('-') && program.is_none() => {
                    program = Some(arg.to_string());
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => crate::warn!("awk: warning: unsupported option '{}'", args[i]),
            }
            i += 1;
        }

        let prog = match program {
            Some(p) => p,
            None => return CommandOutput::error("awk: missing program\n".to_string(), 1),
        };
        // Extract user-defined functions (`function f(a,b){...}`) into a
        // thread-local registry; the program keeps only rules/BEGIN/END.
        let prog = awk_register_functions(&prog);
        // Preload files referenced by `getline ... < "file"` so the evaluator
        // (a free fn without VFS access) can read them.
        {
            let mut rest = prog.as_str();
            while let Some(gp) = rest.find("getline") {
                let after = &rest[gp + "getline".len()..];
                if let Some(lt) = after.find('<') {
                    let tail = after[lt + 1..].trim_start();
                    if let Some(f) = tail.strip_prefix('"') {
                        if let Some(end) = f.find('"') {
                            let path = &f[..end];
                            if let Ok(content) = self.vfs.read_to_string(path, &self.cwd) {
                                let lines: Vec<String> =
                                    content.lines().map(|l| l.to_string()).collect();
                                AWK_GETLINE
                                    .with(|g| g.borrow_mut().insert(path.to_string(), (lines, 0)));
                            }
                        }
                    }
                }
                rest = &after;
            }
        }

        let input = if files.is_empty() {
            // No files and no piped stdin: run with empty input so that
            // `BEGIN`/`END` blocks still execute (`awk 'BEGIN{...}'`).
            stdin.map(|s| s.to_string()).unwrap_or_default()
        } else {
            let mut content = String::new();
            for (i, file) in files.iter().enumerate() {
                if i > 0 {
                    content.push('\n'); // separator between files (like real awk)
                }
                match self.read_text_lossy(file) {
                    Ok(c) => content.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("awk: {}: {}\n", file, e), 1),
                }
            }
            content
        };

        let parsed = parse_awk_full(&prog);
        let mut output = String::new();
        let mut nr = 0usize;
        let mut arrays: HashMap<String, Vec<String>> = HashMap::new();

        // `-F` applies before BEGIN; BEGIN assignments (`FS=`/`OFS=`/`ORS=`) win.
        if let Some(fs) = &fs_arg {
            AWK_FS.with(|f| *f.borrow_mut() = Some(fs.clone()));
        }
        if let Some(ref begin) = parsed.begin {
            awk_apply_begin_specials(begin);
        }
        // Effective FS: `" "`/unset = whitespace; single char = literal; longer =
        // regex (POSIX).
        let fs_effective = AWK_FS.with(|f| f.borrow().clone());
        let fs_is_space = matches!(fs_effective.as_deref(), None | Some(" "));
        let fs_char = fs_effective.as_deref().and_then(|s| s.chars().next());
        let fs_re: Option<regex::Regex> = match fs_effective.as_deref() {
            Some(p) if p.chars().count() > 1 => regex::Regex::new(p).ok(),
            _ => None,
        };

        if let Some(ref begin) = parsed.begin {
            let (result, _) = exec_awk_action(begin, nr, 0, "", &[], &mut variables, &mut arrays);
            output.push_str(&result);
        }

        AWK_MAIN.with(|m| *m.borrow_mut() = (input.lines().map(|s| s.to_string()).collect(), 0));
        loop {
            let next = AWK_MAIN.with(|m| {
                let mut b = m.borrow_mut();
                if b.1 < b.0.len() {
                    let l = b.0[b.1].clone();
                    b.1 += 1;
                    Some(l)
                } else {
                    None
                }
            });
            let Some(next) = next else { break };
            nr += 1;
            let line = next.trim_end();
            let fields: Vec<&str> = if let Some(re) = &fs_re {
                re.split(line).collect()
            } else if fs_is_space {
                line.split_whitespace().collect()
            } else {
                match fs_char {
                    Some(c) => line.split(c).collect(),
                    None => line.split_whitespace().collect(),
                }
            };
            let nf = fields.len();

            let should_execute = match &parsed.condition {
                Some(cond) => eval_awk_cond(cond, nr, nf, line, &fields, &variables),
                None => true,
            };

            if should_execute {
                // No main action (e.g. only BEGIN/END) means no default print.
                if let Some(ref action) = parsed.action {
                    let (result, flow) =
                        exec_awk_action(action, nr, nf, line, &fields, &mut variables, &mut arrays);
                    output.push_str(&result);
                    match flow {
                        AwkFlow::Next => continue,
                        AwkFlow::Exit => break,
                        _ => {}
                    }
                }
            }
        }

        if let Some(ref end) = parsed.end_block {
            let (result, _) = exec_awk_action(end, nr, 0, "", &[], &mut variables, &mut arrays);
            output.push_str(&result);
        }

        CommandOutput::success(output)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum AwkFlow {
    Normal,
    Next,
    Break,
    Continue,
    Exit,
}

struct AwkFull {
    begin: Option<String>,
    condition: Option<String>,
    action: Option<String>,
    end_block: Option<String>,
}

fn parse_awk_full(prog: &str) -> AwkFull {
    let prog = prog.trim();
    let mut begin = None;
    let mut end_block = None;
    let mut condition = None;
    let mut action = None;
    let mut main_part = prog.to_string();

    if let Some(rest) = main_part.strip_prefix("BEGIN") {
        let rest = rest.trim();
        if let Some(body) = extract_braced_block(rest) {
            begin = Some(body);
            main_part = rest[rest.find('}').unwrap_or(0) + 1..].trim().to_string();
        }
    }

    if let Some(idx) = main_part.rfind("END") {
        let after_end = &main_part[idx..];
        if let Some(body) = extract_braced_block(&after_end[3..]) {
            end_block = Some(body);
            main_part = main_part[..idx].trim().to_string();
        }
    }

    let main_part = main_part.trim();
    if !main_part.is_empty() {
        if main_part.starts_with('{') {
            let inner = main_part
                .trim_matches('{')
                .trim_matches('}')
                .trim()
                .to_string();
            action = Some(inner);
        } else if main_part.contains('{') {
            if let Some(brace_pos) = main_part.find('{') {
                condition = Some(main_part[..brace_pos].trim().to_string());
                let rest = &main_part[brace_pos..];
                let inner = rest.trim_matches('{').trim_matches('}').trim().to_string();
                action = Some(inner);
            }
        } else {
            condition = Some(main_part.to_string());
            action = Some("print $0".to_string());
        }
    }

    AwkFull {
        begin,
        condition,
        action,
        end_block,
    }
}

fn extract_braced_block(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[1..i].trim().to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// True when `a` is a pure numeric expression (safe to evaluate as arithmetic).
/// True when `a` contains a BINARY arithmetic operator outside string literals
/// (`+ - * / % ^`), i.e. it should be numerically evaluated rather than printed
/// as a literal string. A bare `$1`/`var` returns false so non-numeric values
/// are preserved.
fn contains_arith_operator(a: &str) -> bool {
    let bytes = a.as_bytes();
    let mut in_str = false;
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'"' => in_str = !in_str,
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' if !in_str => {
                // skip unary +/- at the start or after another operator/space
                if (c == b'+' || c == b'-') && (i == 0 || b"+-*/%^( ".contains(&bytes[i - 1])) {
                    continue;
                }
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Operand string for a condition comparison: numerically evaluate when it
/// contains arithmetic (`$1%2`), else preserve the field/string value.
fn awk_cond_operand(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &HashMap<String, f64>,
) -> String {
    if contains_arith_operator(expr) {
        let v = eval_awk_expr(expr, nr, nf, line, fields, vars);
        if v.fract() == 0.0 {
            format!("{}", v as i64)
        } else {
            format!("{}", v)
        }
    } else {
        awk_value_ext(expr, nr, nf, line, fields, vars)
    }
}

fn awk_cond_truthy(
    cond: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> bool {
    // Conditions using `getline` or a top-level group need the full evaluator.
    if cond.contains("getline") || cond.trim_start().starts_with('(') {
        eval_awk_expr_full(cond, nr, nf, line, fields, vars, arrays) != 0.0
    } else {
        eval_awk_cond(cond, nr, nf, line, fields, vars)
    }
}

fn eval_awk_cond(
    cond: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &HashMap<String, f64>,
) -> bool {
    let cond = cond.trim();
    if cond.is_empty() {
        return true;
    }

    // Handle || (lowest precedence)
    if let Some(pos) = find_operator_outside_parens(cond, "||") {
        let left = cond[..pos].trim();
        let right = cond[pos + 2..].trim();
        if eval_awk_cond(left, nr, nf, line, fields, vars) {
            return true;
        }
        return eval_awk_cond(right, nr, nf, line, fields, vars);
    }

    // Handle && (medium precedence)
    if let Some(pos) = find_operator_outside_parens(cond, "&&") {
        let left = cond[..pos].trim();
        let right = cond[pos + 2..].trim();
        if !eval_awk_cond(left, nr, nf, line, fields, vars) {
            return false;
        }
        return eval_awk_cond(right, nr, nf, line, fields, vars);
    }

    // ==,!= as equality
    if let Some(pos) = find_operator_outside_parens(cond, "==") {
        let left = cond[..pos].trim();
        let right = cond[pos + 2..].trim();
        let lv = awk_cond_operand(left, nr, nf, line, fields, vars);
        let rv = awk_cond_operand(right, nr, nf, line, fields, vars);
        return lv == rv;
    }
    if let Some(pos) = find_operator_outside_parens(cond, "!=") {
        let left = cond[..pos].trim();
        let right = cond[pos + 2..].trim();
        let lv = awk_cond_operand(left, nr, nf, line, fields, vars);
        let rv = awk_cond_operand(right, nr, nf, line, fields, vars);
        return lv != rv;
    }

    if cond.starts_with('/') && cond.ends_with('/') && cond.len() > 2 {
        let pattern = &cond[1..cond.len() - 1];
        return line.contains(pattern);
    }

    // >= and <= need checked before > and <
    let operators = [">=", "<=", ">", "<", "~", "!~"];
    for op in &operators {
        if let Some(pos) = find_operator_outside_parens(cond, op) {
            let left = cond[..pos].trim();
            let right = cond[pos + op.len()..].trim();
            let left_val = awk_cond_operand(left, nr, nf, line, fields, vars);
            let right_val = awk_cond_operand(right, nr, nf, line, fields, vars);

            return match *op {
                ">=" => compare_awk(&left_val, &right_val) != std::cmp::Ordering::Less,
                "<=" => compare_awk(&left_val, &right_val) != std::cmp::Ordering::Greater,
                ">" => compare_awk(&left_val, &right_val) == std::cmp::Ordering::Greater,
                "<" => compare_awk(&left_val, &right_val) == std::cmp::Ordering::Less,
                "~" => right_val.contains(&left_val),
                "!~" => !right_val.contains(&left_val),
                _ => false,
            };
        }
    }

    if cond == "1" || cond.to_lowercase() == "true" {
        return true;
    }
    if cond == "0" || cond.to_lowercase() == "false" {
        return false;
    }

    // Evaluate as a generic expression (non-zero is true)
    let val = eval_awk_expr(cond, nr, nf, line, fields, vars);
    val != 0.0
}

fn find_operator_outside_parens(s: &str, op: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = 0;
    let bytes = s.as_bytes();
    let op_bytes = op.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        if depth == 0 && i + op_bytes.len() <= bytes.len() {
            if &bytes[i..i + op_bytes.len()] == op_bytes {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn split_awk_args(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.is_empty() {
        return vec![String::new()];
    }
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b',' if depth == 0 => {
                args.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    args.push(s[start..].trim().to_string());
    args
}

/// Substitute known awk array references (`name[idx]`, 1-based) with their
/// values so they work inside concatenations like `w[1]"-"w[2]`.
fn awk_expand_array_refs(expr: &str, arrays: &HashMap<String, Vec<String>>) -> String {
    let chars: Vec<char> = expr.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            if i < chars.len() && chars[i] == '[' {
                if let Some(rel) = chars[i + 1..].iter().position(|&c| c == ']') {
                    let close = i + 1 + rel;
                    let idx_s: String = chars[i + 1..close].iter().collect();
                    // Associative array first (string or numeric keys stored as
                    // strings); then the legacy numeric index array.
                    let key_t = idx_s.trim();
                    let key = if key_t.len() >= 2 && key_t.starts_with('"') && key_t.ends_with('"')
                    {
                        key_t[1..key_t.len() - 1].to_string()
                    } else {
                        key_t.to_string()
                    };
                    if let Some(v) = awk_assoc_get(&name, &key) {
                        out.push_str(&v);
                        i = close + 1;
                        continue;
                    }
                    if let Ok(n) = key_t.parse::<usize>() {
                        if let Some(arr) = arrays.get(&name) {
                            let v = if n == 0 {
                                arr.first().cloned()
                            } else {
                                arr.get(n - 1).cloned()
                            };
                            if let Some(v) = v {
                                out.push_str(&v);
                                i = close + 1;
                                continue;
                            }
                        }
                    }
                }
            }
            out.push_str(&name);
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn awk_value_ext(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &HashMap<String, f64>,
) -> String {
    let expr = expr.trim();
    if expr.is_empty() {
        return String::new();
    }

    // Associative array read `arr[subscript]` (keys are strings).
    if let Some((name, key_expr)) = parse_array_subscript(expr) {
        let key = awk_eval_key(&key_expr, nr, nf, line, fields, vars);
        return awk_assoc_get(&name, &key).unwrap_or_default();
    }

    // `split(s, arr, sep)` in value position → returns the field count.
    if expr.starts_with("split(") && expr.ends_with(')') {
        let inner = &expr[6..expr.len() - 1];
        let args = parse_function_args(inner);
        if args.len() >= 2 {
            let s = awk_value_ext(&args[0], nr, nf, line, fields, vars);
            let arr = args[1].trim().to_string();
            let sep = args
                .get(2)
                .map(|a| awk_eval_key(a, nr, nf, line, fields, vars));
            let n = awk_do_split(&s, &arr, sep.as_deref(), nr, nf, line, fields, vars);
            return n.to_string();
        }
    }

    // Ternary `cond ? a : b` (evaluated before concatenation / calls).
    if let Some(v) = try_awk_ternary(expr, nr, nf, line, fields, vars) {
        return v;
    }

    // String concatenation by juxtaposition: `NR": "$0`, `"a" $1 "b"`, ...
    if let Some(atoms) = split_concat_atoms(expr) {
        let mut out = String::new();
        for atom in &atoms {
            out.push_str(&awk_value_ext(atom, nr, nf, line, fields, vars));
        }
        return out;
    }

    if let Some(v) = AWK_STR_LOCALS.with(|m| m.borrow().get(expr).cloned()) {
        return v;
    }
    if let Some(val) = vars.get(expr) {
        return val.to_string();
    }

    match expr {
        "NR" => nr.to_string(),
        "NF" => nf.to_string(),
        "$0" => line.to_string(),
        // Bare `length` == `length($0)` (awk builtin, parens optional).
        "length" => line.len().to_string(),
        _ if expr.starts_with('$') => {
            let inner = expr[1..].trim();
            let idx = if inner == "NF" {
                Some(nf)
            } else if inner == "NR" {
                Some(nr)
            } else if let Ok(n) = inner.parse::<usize>() {
                Some(n)
            } else if inner.starts_with('(') && inner.ends_with(')') {
                let mut tmp_vars = vars.clone();
                Some(eval_awk_expr_full(
                    &inner[1..inner.len() - 1],
                    nr,
                    nf,
                    line,
                    fields,
                    &mut tmp_vars,
                    &mut HashMap::new(),
                ) as usize)
            } else {
                None
            };
            match idx {
                Some(n) if n > 0 && n <= nf => fields[n - 1].to_string(),
                Some(_) => String::new(),
                None => expr.to_string(),
            }
        }
        _ if expr.starts_with('"') && expr.ends_with('"') => awk_unescape(&expr[1..expr.len() - 1]),
        _ => {
            // Try to evaluate as a function call
            if let Some(result) = try_awk_function_call(expr, nr, nf, line, fields, vars) {
                return result;
            }
            expr.to_string()
        }
    }
}

/// Splits an awk expression into juxtaposed concatenation atoms
/// (string literals, `$N` field refs, variables/function calls, numbers).
/// Returns None when the expression contains operators or is a single atom,
/// so the caller can fall back to normal evaluation.
fn split_concat_atoms(expr: &str) -> Option<Vec<String>> {
    let b: Vec<char> = expr.trim().chars().collect();
    let n = b.len();
    let mut atoms: Vec<String> = Vec::new();
    let mut i = 0;
    while i < n {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            let mut atom = String::from("\"");
            let mut j = i + 1;
            let mut closed = false;
            while j < n {
                if b[j] == '\\' && j + 1 < n {
                    atom.push(b[j]);
                    atom.push(b[j + 1]);
                    j += 2;
                    continue;
                }
                atom.push(b[j]);
                if b[j] == '"' {
                    closed = true;
                    break;
                }
                j += 1;
            }
            if !closed {
                return None;
            }
            atoms.push(atom);
            i = j + 1;
        } else if c == '$' {
            let mut atom = String::from("$");
            let mut j = i + 1;
            while j < n && (b[j].is_ascii_alphanumeric() || b[j] == '_') {
                atom.push(b[j]);
                j += 1;
            }
            if atom.len() == 1 {
                return None;
            }
            atoms.push(atom);
            i = j;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i;
            while j < n && (b[j].is_ascii_alphanumeric() || b[j] == '_') {
                j += 1;
            }
            if j < n && b[j] == '(' {
                // Function call: take the balanced parenthesized part.
                let mut depth = 0i32;
                let mut k = j;
                while k < n {
                    if b[k] == '(' {
                        depth += 1;
                    } else if b[k] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    k += 1;
                }
                if k >= n {
                    return None;
                }
                atoms.push(b[i..=k].iter().collect());
                i = k + 1;
            } else {
                atoms.push(b[i..j].iter().collect());
                i = j;
            }
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < n && (b[j].is_ascii_digit() || b[j] == '.') {
                j += 1;
            }
            atoms.push(b[i..j].iter().collect());
            i = j;
        } else {
            // Operator or unsupported syntax — not pure concatenation.
            return None;
        }
    }
    if atoms.len() >= 2 {
        Some(atoms)
    } else {
        None
    }
}

fn compare_awk(a: &str, b: &str) -> std::cmp::Ordering {
    if let (Ok(na), Ok(nb)) = (a.parse::<f64>(), b.parse::<f64>()) {
        na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
    } else {
        a.cmp(b)
    }
}

fn is_next_stmt(stmt: &str) -> bool {
    stmt == "next"
        || stmt.starts_with("next ")
        || stmt.starts_with("next;")
        || stmt.starts_with("next}")
}

fn exec_awk_action(
    action: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> (String, AwkFlow) {
    let action = action.trim();
    let mut result = String::new();
    // `sub`/`gsub` modify $0 in place; keep a mutable copy for that.
    let mut cur_line = line.to_string();
    let stmts = split_statements(action);

    for stmt_str in &stmts {
        let stmt = stmt_str.trim();
        if stmt.is_empty() {
            continue;
        }

        if is_next_stmt(stmt) {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            return (result, AwkFlow::Next);
        }

        // Loop control and `exit`.
        match stmt {
            "break" => return (result, AwkFlow::Break),
            "continue" => return (result, AwkFlow::Continue),
            "exit" => return (result, AwkFlow::Exit),
            _ => {}
        }

        // Control flow: for / while / if (with optional else).
        if let Some((out, flow)) = awk_control_flow(stmt, nr, nf, &cur_line, fields, vars, arrays) {
            result.push_str(&out);
            if flow != AwkFlow::Normal {
                return (result, flow);
            }
            continue;
        }

        // `getline [var] < "file"` as a statement.
        if stmt.starts_with("getline") {
            let _ = eval_awk_expr_full(stmt, nr, nf, &cur_line, fields, vars, arrays);
            continue;
        }

        if stmt.starts_with("printf ") || stmt.starts_with("printf(") {
            let mut expr = stmt[7..].to_string();
            if expr.ends_with(')') {
                expr.pop();
            }
            let args = parse_function_args(&expr);
            if args.len() >= 1 {
                let fmt = awk_value_ext(&args[0], nr, nf, &cur_line, fields, vars);
                let arg_vals: Vec<String> = args[1..]
                    .iter()
                    .map(|a| {
                        // Evaluate arithmetic expressions in the argument list
                        // (`printf "%d\n", 3*4`), like `print` already does.
                        let a2 = awk_expand_array_refs(a, arrays);
                        if awk_use_full_eval(&a2) {
                            let v =
                                eval_awk_expr_full(&a2, nr, nf, &cur_line, fields, vars, arrays);
                            if v.fract() == 0.0 {
                                format!("{}", v as i64)
                            } else {
                                format!("{}", v)
                            }
                        } else {
                            awk_value_ext(&a2, nr, nf, &cur_line, fields, vars)
                        }
                    })
                    .collect();
                result.push_str(&awk_printf(&fmt, &arg_vals));
            }
        } else if stmt.starts_with("print(") && stmt.ends_with(')') {
            let inner = &stmt[6..stmt.len() - 1];
            let args = split_awk_args(inner);
            let __ofs = awk_ofs();
            let __ors = awk_ors();
            for (k, arg) in args.iter().enumerate() {
                if k > 0 {
                    result.push_str(&__ofs);
                }
                let a2 = awk_expand_array_refs(arg, arrays);
                if awk_use_full_eval(&a2) {
                    let v = eval_awk_expr_full(&a2, nr, nf, &cur_line, fields, vars, arrays);
                    result.push_str(&awk_fmt_num(v));
                } else {
                    result.push_str(&awk_value_ext(&a2, nr, nf, &cur_line, fields, vars));
                }
            }
            result.push_str(&__ors);
        } else if stmt.starts_with("print ") || stmt == "print" {
            let expr = if stmt == "print" { "$0" } else { &stmt[6..] };
            let args = split_awk_args(expr);
            let __ofs = awk_ofs();
            let __ors = awk_ors();
            for (k, arg) in args.iter().enumerate() {
                if k > 0 {
                    result.push_str(&__ofs);
                }
                let a2 = awk_expand_array_refs(arg, arrays);
                if awk_use_full_eval(&a2) {
                    let v = eval_awk_expr_full(&a2, nr, nf, &cur_line, fields, vars, arrays);
                    result.push_str(&awk_fmt_num(v));
                } else {
                    result.push_str(&awk_value_ext(&a2, nr, nf, &cur_line, fields, vars));
                }
            }
            result.push_str(&__ors);
        } else if stmt.starts_with("sub(") || stmt.starts_with("gsub(") {
            awk_sub_gsub(stmt, &mut cur_line);
        } else if stmt.starts_with("split(") && stmt.ends_with(')') {
            let _ = awk_value_ext(stmt, nr, nf, &cur_line, fields, vars);
        } else if let Some(rest) = stmt.strip_prefix("delete ") {
            if let Some((name, kexpr)) = parse_array_subscript(rest.trim()) {
                let key = awk_eval_key(&kexpr, nr, nf, &cur_line, fields, vars);
                awk_assoc_del(&name, &key);
            }
        } else if let Some((arr, kexpr, op, rhs)) = parse_assoc_assign(stmt) {
            let key = awk_eval_key(&kexpr, nr, nf, &cur_line, fields, vars);
            let cur: f64 = awk_assoc_get(&arr, &key)
                .unwrap_or_default()
                .trim()
                .parse()
                .unwrap_or(0.0);
            let new_val = match op.as_str() {
                "++" => awk_fmt_num(cur + 1.0),
                "--" => awk_fmt_num(cur - 1.0),
                "+=" => awk_fmt_num(
                    cur + eval_awk_expr_full(&rhs, nr, nf, &cur_line, fields, vars, arrays),
                ),
                "-=" => awk_fmt_num(
                    cur - eval_awk_expr_full(&rhs, nr, nf, &cur_line, fields, vars, arrays),
                ),
                "*=" => awk_fmt_num(
                    cur * eval_awk_expr_full(&rhs, nr, nf, &cur_line, fields, vars, arrays),
                ),
                "/=" => {
                    let d = eval_awk_expr_full(&rhs, nr, nf, &cur_line, fields, vars, arrays);
                    awk_fmt_num(if d != 0.0 { cur / d } else { 0.0 })
                }
                "%=" => {
                    let d = eval_awk_expr_full(&rhs, nr, nf, &cur_line, fields, vars, arrays);
                    awk_fmt_num(if d != 0.0 { cur % d } else { 0.0 })
                }
                _ => {
                    if awk_use_full_eval(&rhs) {
                        awk_fmt_num(eval_awk_expr_full(
                            &rhs, nr, nf, &cur_line, fields, vars, arrays,
                        ))
                    } else {
                        awk_value_ext(&rhs, nr, nf, &cur_line, fields, vars)
                    }
                }
            };
            awk_assoc_set(&arr, &key, &new_val);
        } else if is_incdec_stmt(stmt) {
            awk_apply_expr(stmt, nr, nf, &cur_line, fields, vars, arrays);
        } else if let Some((var, op, expr)) = parse_awk_assignment(stmt) {
            let rhs = eval_awk_expr_full(expr, nr, nf, &cur_line, fields, vars, arrays);
            let entry = vars.entry(var).or_insert(0.0);
            *entry = match op {
                "=" => rhs,
                "+=" => *entry + rhs,
                "-=" => *entry - rhs,
                "*=" => *entry * rhs,
                "/=" => {
                    if rhs != 0.0 {
                        *entry / rhs
                    } else {
                        0.0
                    }
                }
                "%=" => {
                    if rhs != 0.0 {
                        *entry % rhs
                    } else {
                        0.0
                    }
                }
                _ => rhs,
            };
        }
    }

    (result, AwkFlow::Normal)
}

/// Handle `sub(/re/, "rep")` / `gsub(/re/, "rep")` — regex replace of `$0`.
fn awk_sub_gsub(stmt: &str, line: &mut String) {
    let (is_global, rest) = if let Some(r) = stmt.strip_prefix("gsub") {
        (true, r)
    } else if let Some(r) = stmt.strip_prefix("sub") {
        (false, r)
    } else {
        return;
    };
    let rest = rest
        .trim_start_matches('(')
        .trim()
        .trim_end_matches(')')
        .trim();
    let args = split_awk_args(rest);
    if args.len() < 2 {
        return;
    }
    let re_str = args[0].trim();
    let re = if re_str.starts_with('/') {
        match re_str.rfind('/') {
            Some(pos) if pos > 0 => &re_str[1..pos],
            _ => re_str,
        }
    } else {
        re_str
    };
    let rep = args[1].trim().trim_matches('"');

    if let Ok(regex) = regex::Regex::new(re) {
        let replaced = if is_global {
            regex.replace_all(line, rep).to_string()
        } else {
            regex.replace(line, rep).to_string()
        };
        *line = replaced;
    }
}

/// Parse `var op expr` for `= += -= *= /= %=` (top-level operator, not `==`).
fn parse_awk_assignment(s: &str) -> Option<(String, &'static str, &str)> {
    let eq = find_toplevel_eq(s)?;
    let before = &s[..eq];
    let (var, op) = match before.chars().last() {
        Some('+') => (&before[..before.len() - 1], "+="),
        Some('-') => (&before[..before.len() - 1], "-="),
        Some('*') => (&before[..before.len() - 1], "*="),
        Some('/') => (&before[..before.len() - 1], "/="),
        Some('%') => (&before[..before.len() - 1], "%="),
        _ => (before, "="),
    };
    let var = var.trim();
    if var.is_empty() || !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((var.to_string(), op, s[eq + 1..].trim()))
}

/// Parse an associative-array assignment/incr:
/// `arr[key] = rhs`, `arr[key] += rhs`, `arr[key]++`, `arr[key]--`.
/// Returns `(array, key_expr, op, rhs)` where op ∈ {=,+=,-=,*=,/=,%=,++,--}.
fn parse_assoc_assign(s: &str) -> Option<(String, String, String, String)> {
    let t = s.trim();
    if let Some(core) = t.strip_suffix("++") {
        let (name, key) = parse_array_subscript(core.trim())?;
        return Some((name, key, "++".to_string(), String::new()));
    }
    if let Some(core) = t.strip_suffix("--") {
        let (name, key) = parse_array_subscript(core.trim())?;
        return Some((name, key, "--".to_string(), String::new()));
    }
    let eq = find_toplevel_eq(t)?;
    let before = &t[..eq];
    let (lhs, op) = match before.chars().last() {
        Some('+') => (&before[..before.len() - 1], "+="),
        Some('-') => (&before[..before.len() - 1], "-="),
        Some('*') => (&before[..before.len() - 1], "*="),
        Some('/') => (&before[..before.len() - 1], "/="),
        Some('%') => (&before[..before.len() - 1], "%="),
        _ => (before, "="),
    };
    let (name, key) = parse_array_subscript(lhs.trim())?;
    Some((name, key, op.to_string(), t[eq + 1..].trim().to_string()))
}

fn find_toplevel_eq(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'=' if depth == 0 => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    i += 1;
                } else {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_statements(action: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut depth = 0i32;
    let mut brace = 0i32;
    let mut in_string = false;
    let mut start = 0;
    let bytes = action.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' if !in_string => depth += 1,
            b')' if !in_string => depth -= 1,
            b'{' if !in_string => brace += 1,
            b'}' if !in_string => brace -= 1,
            b'"' if !in_string => {
                in_string = true;
            }
            b'"' if in_string => {
                in_string = false;
            }
            b'\\' if in_string => {
                i += 1;
            }
            b';' if depth == 0 && brace == 0 && !in_string => {
                stmts.push(action[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < action.len() {
        stmts.push(action[start..].to_string());
    }
    stmts
}

/// Evaluates `cond ? a : b` (top-level ternary). Returns None when there is no
/// top-level ternary, so the caller falls through to normal evaluation.
fn try_awk_ternary(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &HashMap<String, f64>,
) -> Option<String> {
    let b = expr.as_bytes();
    let n = b.len();
    let mut depth = 0i32;
    let mut in_s = false;
    let mut in_d = false;
    let mut qpos: Option<usize> = None;
    let mut i = 0;
    while i < n {
        let c = b[i];
        if in_s {
            if c == b'\'' {
                in_s = false;
            }
            i += 1;
            continue;
        }
        if in_d {
            if c == b'\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_d = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' => in_s = true,
            b'"' => in_d = true,
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'?' if depth == 0 && qpos.is_none() => qpos = Some(i),
            b':' if depth == 0 && qpos.is_some() => {
                let q = qpos.unwrap();
                let cond = awk_value_ext(expr[..q].trim(), nr, nf, line, fields, vars);
                let truthy = !(cond.is_empty() || cond == "0");
                let chosen = if truthy {
                    expr[q + 1..i].trim()
                } else {
                    expr[i + 1..].trim()
                };
                return Some(awk_value_ext(chosen, nr, nf, line, fields, vars));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn try_awk_function_call(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &HashMap<String, f64>,
) -> Option<String> {
    let expr = expr.trim();
    if !expr.ends_with(')') {
        return None;
    }
    let paren_pos = expr.find('(')?;
    // The call must span the whole expression (`f(x)`, not `f(x)+g(y)`).
    if awk_matching_paren(expr, paren_pos) != Some(expr.chars().count() - 1) {
        return None;
    }
    let func_name = expr[..paren_pos].trim();
    let args_str = &expr[paren_pos + 1..expr.len() - 1];
    let args = parse_function_args(args_str);

    let eval_args: Vec<String> = args
        .iter()
        .map(|a| {
            // Evaluate numeric expressions (`$1/2`, `n*2`) inside function args.
            let t = a.trim();
            let has_op = t.chars().any(|c| matches!(c, '+' | '-' | '*' | '/' | '%'));
            let has_digit = t.chars().any(|c| c.is_ascii_digit());
            if has_op && has_digit && !t.contains('"') && !t.contains('\'') {
                let mut tmp = vars.clone();
                eval_awk_expr_full(t, nr, nf, line, fields, &mut tmp, &mut HashMap::new())
                    .to_string()
            } else {
                awk_value_ext(t, nr, nf, line, fields, vars)
            }
        })
        .collect();

    // User-defined function?
    if let Some((params, body)) = AWK_FUNCS.with(|f| f.borrow().get(func_name).cloned()) {
        if AWK_FUNC_DEPTH.with(|d| d.get()) > 64 {
            return Some("0".to_string());
        }
        let mut local: HashMap<String, f64> = vars.clone();
        let saved_str = AWK_STR_LOCALS.with(|m| std::mem::take(&mut *m.borrow_mut()));
        {
            let mut mm = AWK_STR_LOCALS.with(|m| m.borrow().clone());
            for (p, v) in params.iter().zip(eval_args.iter()) {
                local.insert(p.clone(), v.trim().parse::<f64>().unwrap_or(0.0));
                mm.insert(p.clone(), v.clone());
            }
            AWK_STR_LOCALS.with(|m| *m.borrow_mut() = mm);
        }
        AWK_FUNC_DEPTH.with(|d| d.set(d.get() + 1));
        let mut arrays = HashMap::new();
        let r = eval_awk_func_body(&body, nr, nf, line, fields, &mut local, &mut arrays);
        AWK_FUNC_DEPTH.with(|d| d.set(d.get() - 1));
        AWK_STR_LOCALS.with(|m| *m.borrow_mut() = saved_str);
        return Some(r.unwrap_or_else(|| "0".to_string()));
    }

    match func_name {
        "match" => {
            let target = args
                .first()
                .map(|a| awk_value_ext(a.trim(), nr, nf, line, fields, vars))
                .unwrap_or_default();
            let re_raw = args.get(1).map(|s| s.trim()).unwrap_or("");
            let pat = if re_raw.starts_with('/') {
                match re_raw.rfind('/') {
                    Some(p) if p > 0 => re_raw[1..p].to_string(),
                    _ => re_raw.to_string(),
                }
            } else {
                awk_value_ext(re_raw, nr, nf, line, fields, vars)
            };
            let pos = if pat.is_empty() {
                0
            } else {
                regex::Regex::new(&pat)
                    .ok()
                    .and_then(|re| re.find(&target).map(|m| m.start() + 1))
                    .unwrap_or(0)
            };
            Some(pos.to_string())
        }
        "length" => {
            let s = eval_args.first().map(|s| s.as_str()).unwrap_or(line);
            Some(s.len().to_string())
        }
        "substr" => {
            let s = eval_args.first().map(|s| s.as_str()).unwrap_or("");
            let start: usize = eval_args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
            let len: Option<usize> = eval_args.get(2).and_then(|s| s.parse().ok());
            if start == 0 {
                return Some(String::new());
            }
            let start_idx = start.saturating_sub(1);
            if start_idx >= s.len() {
                return Some(String::new());
            }
            match len {
                Some(l) => Some(s[start_idx..].chars().take(l).collect()),
                None => Some(s[start_idx..].to_string()),
            }
        }
        "tolower" => {
            let s = eval_args.first().map(|s| s.as_str()).unwrap_or("");
            Some(s.to_lowercase())
        }
        "toupper" => {
            let s = eval_args.first().map(|s| s.as_str()).unwrap_or("");
            Some(s.to_uppercase())
        }
        "int" => {
            let v = eval_args.first().and_then(|s| s.trim().parse::<f64>().ok());
            v.map(|x| format!("{}", x.trunc() as i64))
        }
        "sqrt" => {
            let v = eval_args.first().and_then(|s| s.trim().parse::<f64>().ok());
            v.map(|x| {
                let r = x.sqrt();
                if r.fract() == 0.0 {
                    format!("{}", r as i64)
                } else {
                    format!("{r}")
                }
            })
        }
        "exp" | "log" => {
            let v = eval_args.first().and_then(|s| s.trim().parse::<f64>().ok());
            v.map(|x| format!("{}", if func_name == "exp" { x.exp() } else { x.ln() }))
        }
        "sprintf" => {
            // `sprintf(fmt, ...)` — reuse the printf formatter.
            let fmt = eval_args.first().cloned().unwrap_or_default();
            Some(awk_printf(&fmt, &eval_args[1..]))
        }
        "index" => {
            let s = eval_args.first().map(|s| s.as_str()).unwrap_or("");
            let sub = eval_args.get(1).map(|s| s.as_str()).unwrap_or("");
            match s.find(sub) {
                Some(pos) => Some((pos + 1).to_string()), // awk index is 1-based
                None => Some("0".to_string()),
            }
        }
        "match" => {
            let s = eval_args.first().map(|s| s.as_str()).unwrap_or("");
            let re = eval_args.get(1).map(|s| s.as_str()).unwrap_or("");
            if let Ok(regex) = regex::Regex::new(re) {
                match regex.find(s) {
                    Some(m) => Some((m.start() + 1).to_string()),
                    None => Some("0".to_string()),
                }
            } else {
                Some("0".to_string())
            }
        }
        "split" => None,
        _ => None,
    }
}

fn parse_function_args(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.is_empty() {
        return vec![String::new()];
    }
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' if !in_string => depth += 1,
            b')' if !in_string => depth -= 1,
            b'"' if !in_string => in_string = true,
            b'"' if in_string => in_string = false,
            b'\\' if in_string => {
                i += 1;
            }
            b',' if depth == 0 && !in_string => {
                args.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    args.push(s[start..].trim().to_string());
    args
}

fn awk_unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('/') => out.push('/'),
                Some('0') => out.push('\0'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn apply_printf_spec(
    flags: &str,
    width: usize,
    prec: Option<usize>,
    conv: char,
    arg: &str,
) -> String {
    let left = flags.contains('-');
    let zero = flags.contains('0') && !left;
    let body = match conv {
        'd' | 'i' => arg.parse::<i64>().unwrap_or(0).to_string(),
        'f' => format!(
            "{:.*}",
            prec.unwrap_or(6),
            arg.parse::<f64>().unwrap_or(0.0)
        ),
        'e' => format!("{:e}", arg.parse::<f64>().unwrap_or(0.0)),
        'g' => format!("{}", arg.parse::<f64>().unwrap_or(0.0)),
        'x' => format!("{:x}", arg.parse::<u64>().unwrap_or(0)),
        'X' => format!("{:X}", arg.parse::<u64>().unwrap_or(0)),
        'o' => format!("{:o}", arg.parse::<u64>().unwrap_or(0)),
        'c' => arg
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default(),
        _ => arg.to_string(),
    };
    if body.chars().count() >= width {
        return body;
    }
    let pad = width - body.chars().count();
    let pc = if zero { '0' } else { ' ' };
    if left {
        format!("{}{}", body, pc.to_string().repeat(pad))
    } else {
        format!("{}{}", pc.to_string().repeat(pad), body)
    }
}

fn awk_printf(fmt: &str, args: &[String]) -> String {
    let mut result = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut arg_idx = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            i += 1;
            if chars[i] == '%' {
                result.push('%');
                i += 1;
                continue;
            }
            // Parse `%[-+ 0#]*[width][.prec]type`.
            let mut flags = String::new();
            while i < chars.len() && "-+ 0#".contains(chars[i]) {
                flags.push(chars[i]);
                i += 1;
            }
            let mut width = 0usize;
            while i < chars.len() && chars[i].is_ascii_digit() {
                width = width * 10 + (chars[i] as usize - '0' as usize);
                i += 1;
            }
            let mut prec: Option<usize> = None;
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                let mut p = 0usize;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    p = p * 10 + (chars[i] as usize - '0' as usize);
                    i += 1;
                }
                prec = Some(p);
            }
            if i >= chars.len() {
                result.push('%');
                result.push_str(&flags);
                break;
            }
            let conv = chars[i];
            if arg_idx >= args.len() {
                result.push('%');
                result.push_str(&flags);
                if width > 0 {
                    result.push_str(&width.to_string());
                }
                result.push(conv);
                i += 1;
                continue;
            }
            result.push_str(&apply_printf_spec(
                &flags,
                width,
                prec,
                conv,
                &args[arg_idx],
            ));
            arg_idx += 1;
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Last index of `op` at top-level (outside parens/brackets/quotes).
fn awk_rfind_op(s: &str, op: char) -> Option<usize> {
    let c: Vec<char> = s.chars().collect();
    let mut d = 0i32;
    let mut in_s = false;
    let mut in_d = false;
    let mut last = None;
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if in_s {
            if ch == '\'' {
                in_s = false;
            }
            i += 1;
            continue;
        }
        if in_d {
            if ch == '"' {
                in_d = false;
            }
            i += 1;
            continue;
        }
        match ch {
            '\'' => in_s = true,
            '"' => in_d = true,
            '(' | '[' => d += 1,
            ')' | ']' => d -= 1,
            _ if ch == op && d == 0 => {
                last = Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    last
}

/// True when `pat` occurs at top level (outside quotes/parens).
fn awk_has_toplevel(s: &str, pat: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    if p.is_empty() {
        return false;
    }
    let mut d = 0i32;
    let mut in_s = false;
    let mut in_d = false;
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if in_s {
            if ch == '\'' {
                in_s = false;
            }
            i += 1;
            continue;
        }
        if in_d {
            if ch == '"' {
                in_d = false;
            }
            i += 1;
            continue;
        }
        match ch {
            '\'' => in_s = true,
            '"' => in_d = true,
            '(' | '[' => d += 1,
            ')' | ']' => d -= 1,
            _ => {}
        }
        if d == 0 && !in_s && !in_d && c[i..].starts_with(&p[..]) {
            return true;
        }
        i += 1;
    }
    false
}

/// True when an argument needs the numeric/relational evaluator (arithmetic,
/// top-level comparisons, a `getline`, or a fully-parenthesized expression)
/// rather than the string/concatenation path.
fn awk_use_full_eval(s: &str) -> bool {
    let t = s.trim();
    contains_arith_operator(t)
        || t == "getline"
        || t.starts_with("getline ")
        || t.starts_with("getline<")
        || awk_has_toplevel(t, "<")
        || awk_has_toplevel(t, ">")
        || awk_has_toplevel(t, "==")
        || awk_has_toplevel(t, "!=")
        || awk_has_toplevel(t, "&&")
        || awk_has_toplevel(t, "||")
        || (t.starts_with('(') && awk_matching_paren(t, 0) == Some(t.chars().count() - 1))
}

fn eval_awk_expr_full(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> f64 {
    let expr = expr.trim();
    if expr.is_empty() {
        return 0.0;
    }

    // Handle split assignment in expression (like split(...) used as rvalue)
    if expr.starts_with("split(") && expr.ends_with(')') {
        let args_str = &expr[6..expr.len() - 1];
        let args = parse_function_args(args_str);
        let eval_args: Vec<String> = args
            .iter()
            .map(|a| awk_value_ext(a, nr, nf, line, fields, vars))
            .collect();
        if eval_args.len() >= 2 {
            let s = &eval_args[0];
            let arr_name = args[1].trim().to_string();
            let sep = eval_args.get(2).map(|s| s.as_str()).unwrap_or(" ");
            let parts: Vec<String> = if sep.is_empty() {
                s.chars().map(|c| c.to_string()).collect()
            } else {
                s.split(sep).map(|t| t.to_string()).collect()
            };
            let count = parts.len() as f64;
            arrays.insert(arr_name, parts);
            return count;
        }
        return 0.0;
    }

    // Strip fully-enclosing parentheses: `(EXPR)`.
    if expr.starts_with('(') && awk_matching_paren(expr, 0) == Some(expr.chars().count() - 1) {
        return eval_awk_expr_full(&expr[1..expr.len() - 1], nr, nf, line, fields, vars, arrays);
    }

    // `getline [var] < "file"` — read the next line from a preloaded file.
    if expr == "getline" || expr.starts_with("getline ") || expr.starts_with("getline<") {
        let body = expr["getline".len()..].trim();
        let (target, file) = if let Some(lt) = body.find('<') {
            let t = body[..lt].trim();
            let f = body[lt + 1..].trim().trim_matches('"').to_string();
            (
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                },
                Some(f),
            )
        } else {
            (
                if body.is_empty() {
                    None
                } else {
                    Some(body.to_string())
                },
                None,
            )
        };
        if let Some(f) = file {
            let line = AWK_GETLINE.with(|g| {
                let mut m = g.borrow_mut();
                let e = m.entry(f).or_insert_with(|| (Vec::new(), 0));
                if e.1 < e.0.len() {
                    let l = e.0[e.1].clone();
                    e.1 += 1;
                    Some(l)
                } else {
                    None
                }
            });
            match line {
                Some(l) => {
                    if let Some(t) = &target {
                        vars.insert(t.clone(), l.trim().parse::<f64>().unwrap_or(0.0));
                        AWK_STR_LOCALS.with(|m| m.borrow_mut().insert(t.clone(), l));
                    }
                    return 1.0;
                }
                None => return 0.0,
            }
        }
        // Main-input `getline` — advance the shared record cursor.
        let line = AWK_MAIN.with(|m| {
            let mut b = m.borrow_mut();
            if b.1 < b.0.len() {
                let l = b.0[b.1].clone();
                b.1 += 1;
                Some(l)
            } else {
                None
            }
        });
        return match line {
            Some(l) => {
                if let Some(t) = &target {
                    vars.insert(t.clone(), l.trim().parse::<f64>().unwrap_or(0.0));
                    AWK_STR_LOCALS.with(|m| m.borrow_mut().insert(t.clone(), l));
                }
                1.0
            }
            None => 0.0,
        };
    }

    // Comparisons / logical (approximate precedence; enough for conditions).
    for op in ["==", "!=", ">=", "<=", "&&", "||"] {
        if let Some(pos) = expr.rfind(op) {
            if pos > 0 && pos + op.len() < expr.len() {
                let l = eval_awk_expr_full(&expr[..pos], nr, nf, line, fields, vars, arrays);
                let r =
                    eval_awk_expr_full(&expr[pos + op.len()..], nr, nf, line, fields, vars, arrays);
                let b = match op {
                    "==" => l == r,
                    "!=" => l != r,
                    ">=" => l >= r,
                    "<=" => l <= r,
                    "&&" => l != 0.0 && r != 0.0,
                    "||" => l != 0.0 || r != 0.0,
                    _ => false,
                };
                return if b { 1.0 } else { 0.0 };
            }
        }
    }
    for op in [">", "<"] {
        if let Some(pos) = expr.rfind(op) {
            let before = &expr[pos..];
            if pos > 0 && !before.starts_with(">=") && !before.starts_with("<=") {
                let l = eval_awk_expr_full(&expr[..pos], nr, nf, line, fields, vars, arrays);
                let r = eval_awk_expr_full(&expr[pos + 1..], nr, nf, line, fields, vars, arrays);
                let b = if op == ">" { l > r } else { l < r };
                return if b { 1.0 } else { 0.0 };
            }
        }
    }

    // Try function call that returns a value
    if let Some(result) = try_awk_function_call(expr, nr, nf, line, fields, vars) {
        return result.parse::<f64>().unwrap_or(0.0);
    }

    if let Some(pos) = awk_rfind_op(expr, '-') {
        if pos > 0 && !is_operator_at(&expr, pos, ">=") && !is_operator_at(&expr, pos, "<=") {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            return eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays)
                - eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
        }
    }

    if let Some(pos) = awk_rfind_op(expr, '+') {
        if pos > 0 {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            return eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays)
                + eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
        }
    }

    // Exponentiation (awk `^`)
    if let Some(pos) = awk_rfind_op(expr, '^') {
        if pos > 0 {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            let base = eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays);
            let exp = eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
            return base.powf(exp);
        }
    }

    // Multiplication
    if let Some(pos) = awk_rfind_op(expr, '*') {
        if pos > 0 && expr.as_bytes()[pos - 1] != b'*' {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            return eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays)
                * eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
        }
    }

    // Modulo (awk `%`)
    if let Some(pos) = awk_rfind_op(expr, '%') {
        if pos > 0 && !is_operator_at(&expr, pos, "%=") {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            let denom = eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
            if denom == 0.0 {
                return 0.0;
            }
            return eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays) % denom;
        }
    }

    // Division
    if let Some(pos) = awk_rfind_op(expr, '/') {
        if pos > 0 && !expr[pos - 1..].starts_with('/') {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            let denom = eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
            if denom == 0.0 {
                return 0.0;
            }
            return eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays) / denom;
        }
    }

    if let Some(val) = vars.get(expr) {
        return *val;
    }

    let val = awk_value_ext(expr, nr, nf, line, fields, vars);
    val.parse::<f64>().unwrap_or(0.0)
}

fn eval_awk_expr(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &HashMap<String, f64>,
) -> f64 {
    let mut vars_mut = vars.clone();
    let mut arrays: HashMap<String, Vec<String>> = HashMap::new();
    eval_awk_expr_full(expr, nr, nf, line, fields, &mut vars_mut, &mut arrays)
}

fn is_operator_at(s: &str, pos: usize, op: &str) -> bool {
    if pos >= op.len() - 1 {
        let start = pos + 1 - op.len();
        if start + op.len() <= s.len() {
            return &s[start..start + op.len()] == op;
        }
    }
    false
}

use std::cell::{Cell, RefCell};

thread_local! {
    static AWK_FUNCS: RefCell<HashMap<String, (Vec<String>, String)>> = RefCell::new(HashMap::new());
    static AWK_STR_LOCALS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static AWK_FUNC_DEPTH: Cell<usize> = const { Cell::new(0) };
    static AWK_GETLINE: RefCell<HashMap<String, (Vec<String>, usize)>> = RefCell::new(HashMap::new());
    static AWK_MAIN: RefCell<(Vec<String>, usize)> = RefCell::new((Vec::new(), 0));
    /// Field separator (`None`/`" "` = whitespace). May be a regex.
    static AWK_FS: RefCell<Option<String>> = const { RefCell::new(None) };
    static AWK_OFS: RefCell<String> = const { RefCell::new(String::new()) };
    static AWK_ORS: RefCell<String> = const { RefCell::new(String::new()) };
    /// Associative arrays: `name -> (string key -> string value)`.
    static AWK_ASSOC: RefCell<HashMap<String, HashMap<String, String>>> =
        RefCell::new(HashMap::new());
}

fn awk_assoc_get(arr: &str, key: &str) -> Option<String> {
    AWK_ASSOC.with(|a| a.borrow().get(arr).and_then(|m| m.get(key).cloned()))
}

fn awk_assoc_set(arr: &str, key: &str, val: &str) {
    AWK_ASSOC.with(|a| {
        a.borrow_mut()
            .entry(arr.to_string())
            .or_default()
            .insert(key.to_string(), val.to_string());
    });
}

fn awk_assoc_keys(arr: &str) -> Vec<String> {
    AWK_ASSOC.with(|a| {
        a.borrow()
            .get(arr)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    })
}

fn awk_assoc_del(arr: &str, key: &str) {
    AWK_ASSOC.with(|a| {
        if let Some(m) = a.borrow_mut().get_mut(arr) {
            m.remove(key);
        }
    });
}

/// Parse `name[subscript]` when it spans the whole (trimmed) expression.
fn parse_array_subscript(expr: &str) -> Option<(String, String)> {
    let e = expr.trim();
    let bytes = e.as_bytes();
    if bytes.is_empty() || !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return None;
    }
    let mut i = 0;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'[' {
        return None;
    }
    let name = e[..i].to_string();
    let open = i;
    // Find the matching closing bracket.
    let mut d = 0i32;
    let mut in_str = false;
    let mut j = open;
    while j < bytes.len() {
        match bytes[j] {
            b'"' => in_str = !in_str,
            b'[' if !in_str => d += 1,
            b']' if !in_str => {
                d -= 1;
                if d == 0 {
                    let key_expr = e[open + 1..j].to_string();
                    if j + 1 != bytes.len() {
                        return None;
                    }
                    return Some((name, key_expr));
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Evaluate an array subscript expression to its string key.
fn awk_eval_key(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &HashMap<String, f64>,
) -> String {
    let t = expr.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return awk_unescape(&t[1..t.len() - 1]);
    }
    awk_value_ext(t, nr, nf, line, fields, vars)
}

/// `split(s, arr, sep)` — populate the associative array `arr` with 1-based
/// keys and return the field count. `sep` defaults to FS (or whitespace).
fn awk_do_split(
    s: &str,
    arr: &str,
    sep: Option<&str>,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &HashMap<String, f64>,
) -> usize {
    let parts: Vec<String> = match sep {
        Some(sep) if sep.chars().count() == 1 => s
            .split(sep.chars().next().unwrap())
            .map(|x| x.to_string())
            .collect(),
        Some(sep) if !sep.is_empty() => match regex::Regex::new(sep) {
            Ok(re) => re.split(s).map(|x| x.to_string()).collect(),
            Err(_) => s.split(sep).map(|x| x.to_string()).collect(),
        },
        _ => {
            // Default: FS if multi-char regex, else whitespace.
            let fs = AWK_FS.with(|f| f.borrow().clone());
            match fs.as_deref() {
                Some(p) if p.chars().count() > 1 => match regex::Regex::new(p) {
                    Ok(re) => re.split(s).map(|x| x.to_string()).collect(),
                    Err(_) => s.split_whitespace().map(|x| x.to_string()).collect(),
                },
                Some(p) if p.chars().count() == 1 && p != " " => s
                    .split(p.chars().next().unwrap())
                    .map(|x| x.to_string())
                    .collect(),
                _ => s.split_whitespace().map(|x| x.to_string()).collect(),
            }
        }
    };
    let _ = (nr, nf, line, fields, vars);
    AWK_ASSOC.with(|a| {
        let mut b = a.borrow_mut();
        let entry = b.entry(arr.to_string()).or_default();
        entry.clear();
        for (i, p) in parts.iter().enumerate() {
            entry.insert((i + 1).to_string(), p.clone());
        }
    });
    parts.len()
}

/// Extract a `NAME="literal"` string assignment from `body` (word-boundary safe
/// so `FS` doesn't match inside `OFS`).
fn awk_assign_string(body: &str, name: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let n = name.len();
    let mut i = 0;
    while let Some(pos) = body[i..].find(name) {
        let at = i + pos;
        let before_ok =
            at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        let end = at + n;
        let after_ok =
            end >= bytes.len() || !(bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
        if before_ok && after_ok {
            let after = body[end..].trim_start();
            if let Some(a2) = after.strip_prefix('=') {
                let a3 = a2.trim_start();
                if let Some(q) = a3.strip_prefix('"') {
                    if let Some(e) = q.find('"') {
                        return Some(q[..e].to_string());
                    }
                }
                // Unquoted single-token value (e.g. FS=,).
                let v: String = a3
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != ';' && *c != '}')
                    .collect();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        i = end;
        if i >= bytes.len() {
            break;
        }
    }
    None
}

/// Output field separator (default single space).
fn awk_ofs() -> String {
    AWK_OFS.with(|o| {
        let v = o.borrow().clone();
        if v.is_empty() {
            " ".to_string()
        } else {
            v
        }
    })
}

/// Output record separator (default newline).
fn awk_ors() -> String {
    AWK_ORS.with(|o| {
        let v = o.borrow().clone();
        if v.is_empty() {
            "\n".to_string()
        } else {
            v
        }
    })
}

/// Apply `FS`/`OFS`/`ORS` assignments found in the BEGIN block.
fn awk_apply_begin_specials(begin: &str) {
    if let Some(v) = awk_assign_string(begin, "FS") {
        AWK_FS.with(|f| *f.borrow_mut() = Some(v));
    }
    if let Some(v) = awk_assign_string(begin, "OFS") {
        AWK_OFS.with(|f| *f.borrow_mut() = v);
    }
    if let Some(v) = awk_assign_string(begin, "ORS") {
        AWK_ORS.with(|f| *f.borrow_mut() = v);
    }
}

/// Extract `function NAME(params){body}` definitions; register them (thread
/// local) and return the program with the definitions removed.
fn awk_register_functions(prog: &str) -> String {
    AWK_FUNCS.with(|f| f.borrow_mut().clear());
    AWK_STR_LOCALS.with(|f| f.borrow_mut().clear());
    AWK_GETLINE.with(|g| g.borrow_mut().clear());
    AWK_FUNC_DEPTH.with(|d| d.set(0));
    AWK_FS.with(|f| *f.borrow_mut() = None);
    AWK_OFS.with(|f| *f.borrow_mut() = String::new());
    AWK_ORS.with(|f| *f.borrow_mut() = String::new());
    AWK_ASSOC.with(|a| a.borrow_mut().clear());
    let chars: Vec<char> = prog.chars().collect();
    let kw = ['f', 'u', 'n', 'c', 't', 'i', 'o', 'n'];
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let at_boundary = i == 0 || !(chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_');
        if at_boundary && i + 8 <= chars.len() && chars[i..i + 8] == kw {
            let mut j = i + 8;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let ns = j;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let name: String = chars[ns..j].iter().collect();
            if !name.is_empty() {
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '(' {
                    let mut k = j + 1;
                    let ps = k;
                    let mut d = 1;
                    while k < chars.len() && d > 0 {
                        match chars[k] {
                            '(' => d += 1,
                            ')' => d -= 1,
                            _ => {}
                        }
                        k += 1;
                    }
                    let params: Vec<String> = chars[ps..k.saturating_sub(1)]
                        .iter()
                        .collect::<String>()
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    while k < chars.len() && chars[k].is_whitespace() {
                        k += 1;
                    }
                    if k < chars.len() && chars[k] == '{' {
                        let mut m = k + 1;
                        let bs = m;
                        let mut d2 = 1;
                        while m < chars.len() && d2 > 0 {
                            match chars[m] {
                                '{' => d2 += 1,
                                '}' => d2 -= 1,
                                _ => {}
                            }
                            m += 1;
                        }
                        let body: String = chars[bs..m.saturating_sub(1)].iter().collect();
                        AWK_FUNCS.with(|f| f.borrow_mut().insert(name.clone(), (params, body)));
                        i = m;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn awk_fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

fn awk_matching_paren(s: &str, open: usize) -> Option<usize> {
    let c: Vec<char> = s.chars().collect();
    let mut d = 0i32;
    let mut i = open;
    while i < c.len() {
        match c[i] {
            '(' => d += 1,
            ')' => {
                d -= 1;
                if d == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split a function body on top-level `;`, honoring parens/braces/strings.
fn awk_split_stmts(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut d, mut br, mut in_s, mut in_d) = (0i32, 0i32, false, false);
    for &ch in &c {
        if in_s {
            cur.push(ch);
            if ch == '\'' {
                in_s = false;
            }
            continue;
        }
        if in_d {
            cur.push(ch);
            if ch == '"' {
                in_d = false;
            }
            continue;
        }
        match ch {
            '\'' => {
                in_s = true;
                cur.push(ch);
            }
            '"' => {
                in_d = true;
                cur.push(ch);
            }
            '(' => {
                d += 1;
                cur.push(ch);
            }
            ')' => {
                d -= 1;
                cur.push(ch);
            }
            '{' => {
                br += 1;
                cur.push(ch);
            }
            '}' => {
                br -= 1;
                cur.push(ch);
            }
            ';' | '\n' if d == 0 && br == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out
}

/// Parse the part after an `if (cond)`: returns (then, else, rest).
/// `then`/`else` are block/statement bodies; `rest` is the remainder after
/// the if-statement (so `if(c) stmt; more` falls through to `more`).
fn awk_split_if_else(s: &str) -> (String, String, String) {
    let take_body = |t: &str| -> (String, String) {
        let t = t.trim_start();
        if t.starts_with('{') {
            let c: Vec<char> = t.chars().collect();
            let mut d = 0i32;
            let mut i = 0;
            while i < c.len() {
                match c[i] {
                    '{' => d += 1,
                    '}' => {
                        d -= 1;
                        if d == 0 {
                            let body: String = c[1..i].iter().collect();
                            let rest: String = c[i + 1..].iter().collect();
                            return (body, rest);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            (t[1..].to_string(), String::new())
        } else {
            match awk_split_stmts(t).into_iter().next() {
                Some(first) => {
                    let consumed = first.len();
                    let rest = t[consumed..]
                        .trim_start_matches([';', ' ', '\t', '\n'])
                        .to_string();
                    (first, rest)
                }
                None => (String::new(), String::new()),
            }
        }
    };
    let (then_p, after_then) = take_body(s);
    let at = after_then.trim_start();
    if let Some(rem) = at.strip_prefix("else") {
        let (else_p, rest) = take_body(rem);
        (then_p, else_p, rest)
    } else {
        (then_p, String::new(), after_then)
    }
}

/// Execute a user-function body (subset: `return EXPR`, sequence, `if/else`).
fn eval_awk_func_body(
    body: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    local: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> Option<String> {
    let b = body.trim();
    let b = b.strip_prefix('{').unwrap_or(b);
    let b = b.strip_suffix('}').unwrap_or(b);
    let b = b.trim();
    if let Some(rest) = b.strip_prefix("if") {
        if let Some(open) = rest.find('(') {
            if let Some(close) = awk_matching_paren(rest, open) {
                let cond = eval_awk_expr_full(
                    rest[open + 1..close].trim(),
                    nr,
                    nf,
                    line,
                    fields,
                    local,
                    arrays,
                );
                let after = rest[close + 1..].trim();
                let (then_p, else_p, rest2) = awk_split_if_else(after);
                let chosen = if cond != 0.0 { then_p } else { else_p };
                if !chosen.trim().is_empty() {
                    if let Some(r) =
                        eval_awk_func_body(&chosen, nr, nf, line, fields, local, arrays)
                    {
                        return Some(r);
                    }
                }
                if !rest2.trim().is_empty() {
                    return eval_awk_func_body(&rest2, nr, nf, line, fields, local, arrays);
                }
                return None;
            }
        }
    }
    for stmt in awk_split_stmts(b) {
        let st = stmt.trim();
        if let Some(e) = st.strip_prefix("return") {
            let e = e.trim();
            // Arithmetic returns go through the numeric evaluator; anything
            // else (string literals, string functions, bare vars) is evaluated
            // with the string-aware path.
            let has_op = !e.contains('"')
                && e.chars().any(|c| {
                    matches!(
                        c,
                        '+' | '-' | '*' | '/' | '%' | '^' | '<' | '>' | '=' | '!' | '&' | '|'
                    )
                });
            if has_op {
                let v = eval_awk_expr_full(e, nr, nf, line, fields, local, arrays);
                return Some(awk_fmt_num(v));
            }
            return Some(awk_value_ext(e, nr, nf, line, fields, local));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_awk_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        Shell::new(setup_vfs())
    }

    #[test]
    fn test_awk_print_field() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{print $1}"], Some("hello world\nfoo bar\n"));
        assert!(out.stdout.contains("hello"));
        assert!(out.stdout.contains("foo"));
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_awk_condition_eq() {
        let shell = mk_shell();
        let out = shell.cmd_awk(
            &["$1 == \"hello\" {print $2}"],
            Some("hello world\nfoo bar\n"),
        );
        assert!(out.stdout.contains("world"));
        assert!(!out.stdout.contains("bar"));
    }

    #[test]
    fn test_awk_printf() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{printf(\"%s:%d\\n\", $1, NR)}"], Some("hello\nworld\n"));
        assert!(out.stdout.contains("hello:1"));
        assert!(out.stdout.contains("world:2"));
    }

    #[test]
    fn test_awk_length() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{print length($0)}"], Some("hello\nworld\n"));
        assert!(out.stdout.contains("5"));
    }

    #[test]
    fn test_awk_substr() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{print substr($0, 2, 3)}"], Some("hello\nworld\n"));
        assert!(out.stdout.contains("ell"));
        assert!(out.stdout.contains("orl"));
    }

    #[test]
    fn test_awk_tolower_toupper() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{print tolower($0)}"], Some("HELLO\n"));
        assert!(out.stdout.contains("hello"));
        let out = shell.cmd_awk(&["{print toupper($0)}"], Some("hello\n"));
        assert!(out.stdout.contains("HELLO"));
    }

    #[test]
    fn test_awk_next() {
        let shell = mk_shell();
        let out = shell.cmd_awk(
            &["{ print \"before\"; next; print \"after\" }"],
            Some("line1\nline2\n"),
        );
        assert!(out.stdout.contains("before"));
        assert!(!out.stdout.contains("after"));
        assert_eq!(out.stdout.trim().lines().count(), 2);
    }

    #[test]
    fn test_awk_v_flag() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["-v", "n=42", "{print n}"], Some("any\n"));
        assert!(out.stdout.contains("42"));
    }

    #[test]
    fn test_awk_and_or() {
        let shell = mk_shell();
        let out = shell.cmd_awk(
            &["$1 == \"a\" || $1 == \"b\" {print $0}"],
            Some("a\nb\nc\n"),
        );
        assert!(out.stdout.contains("a"));
        assert!(out.stdout.contains("b"));
        assert!(!out.stdout.contains("c"));

        let out = shell.cmd_awk(
            &["NR > 1 && NR < 4 {print $0}"],
            Some("line1\nline2\nline3\nline4\n"),
        );
        assert!(out.stdout.contains("line2"));
        assert!(out.stdout.contains("line3"));
        assert!(!out.stdout.contains("line1"));
        assert!(!out.stdout.contains("line4"));
    }

    #[test]
    fn test_awk_split() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{ n = split($0, a, \",\"); print n }"], Some("a,b,c\n"));
        assert!(out.stdout.contains("3"));
    }

    #[test]
    fn test_awk_concat_in_print() {
        let shell = mk_shell();
        // `NR": "$0` — concatenation with no spaces (agent line-numbering idiom)
        let out = shell.cmd_awk(&["{print NR\": \"$0}"], Some("alpha\nbeta\n"));
        assert_eq!(out.stdout, "1: alpha\n2: beta\n");
        // Concatenation with spaces
        let out = shell.cmd_awk(&["{print \"<\" $1 \">\"}"], Some("x y\n"));
        assert_eq!(out.stdout, "<x>\n");
    }

    #[test]
    fn test_awk_concat_function_atom() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{print \"len=\" length($0)}"], Some("hello\n"));
        assert_eq!(out.stdout, "len=5\n");
    }

    #[test]
    fn test_awk_help() {
        let mut shell = mk_shell();
        let out = shell.execute("awk", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_awk_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("awk", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_awk_sub() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{sub(/foo/, \"bar\"); print}"], Some("a foo b\n"));
        assert!(
            out.stdout.contains("a bar b"),
            "sub should replace first match: {}",
            out.stdout
        );
    }

    #[test]
    fn test_awk_gsub() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{gsub(/a/, \"x\"); print}"], Some("banana\n"));
        assert!(
            out.stdout.contains("bxnxnx"),
            "gsub should replace all matches: {}",
            out.stdout
        );
    }

    #[test]
    fn test_awk_index() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{print index($0, \"ll\")}"], Some("hello\n"));
        assert!(
            out.stdout.contains("3"),
            "index should be 1-based: {}",
            out.stdout
        );
    }

    #[test]
    fn test_awk_match() {
        let shell = mk_shell();
        let out = shell.cmd_awk(&["{print match($0, \"[0-9]+\")}"], Some("abc123def\n"));
        assert!(
            out.stdout.contains("4"),
            "match should return 1-based position: {}",
            out.stdout
        );
    }
}

// ── awk control flow: for / while / if ──────────────────────────────

fn awk_control_flow(
    stmt: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> Option<(String, AwkFlow)> {
    let st = stmt.trim();
    for kw in ["for", "while", "if"] {
        if let Some(rest) = st.strip_prefix(kw) {
            let rest = rest.trim_start();
            if rest.starts_with('(') {
                return Some(match kw {
                    "for" => awk_for(rest, nr, nf, line, fields, vars, arrays),
                    "while" => awk_while(rest, nr, nf, line, fields, vars, arrays),
                    _ => awk_if(rest, nr, nf, line, fields, vars, arrays),
                });
            }
        }
    }
    None
}

fn awk_close_paren(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut d = 0i32;
    let mut in_str = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_str = !in_str,
            b'(' if !in_str => d += 1,
            b')' if !in_str => {
                d -= 1;
                if d == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn awk_run_body(
    body: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> (String, AwkFlow) {
    let b = body.trim();
    let inner = if b.starts_with('{') && b.ends_with('}') {
        &b[1..b.len() - 1]
    } else {
        b
    };
    exec_awk_action(inner, nr, nf, line, fields, vars, arrays)
}

/// Applies an awk expression statement: `i++`, `i--`, `++i`, `--i`, assignment.
fn awk_apply_expr(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) {
    let e = expr.trim();
    if e.is_empty() {
        return;
    }
    for (suf, delta) in [("++", 1.0f64), ("--", -1.0)] {
        if let Some(name) = e.strip_suffix(suf) {
            let name = name.trim();
            if is_awk_name(name) {
                *vars.entry(name.to_string()).or_insert(0.0) += delta;
                return;
            }
        }
    }
    for (pre, delta) in [("++", 1.0f64), ("--", -1.0)] {
        if let Some(name) = e.strip_prefix(pre) {
            let name = name.trim();
            if is_awk_name(name) {
                *vars.entry(name.to_string()).or_insert(0.0) += delta;
                return;
            }
        }
    }
    if let Some((var, op, rhs)) = parse_awk_assignment(e) {
        let v = eval_awk_expr_full(rhs, nr, nf, line, fields, vars, arrays);
        let entry = vars.entry(var).or_insert(0.0);
        *entry = match op {
            "=" => v,
            "+=" => *entry + v,
            "-=" => *entry - v,
            "*=" => *entry * v,
            "/=" => {
                if v != 0.0 {
                    *entry / v
                } else {
                    0.0
                }
            }
            "%=" => {
                if v != 0.0 {
                    *entry % v
                } else {
                    0.0
                }
            }
            _ => v,
        };
        return;
    }
    let _ = eval_awk_expr_full(e, nr, nf, line, fields, vars, arrays);
}

fn awk_for(
    rest: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> (String, AwkFlow) {
    let close = match awk_close_paren(rest) {
        Some(c) => c,
        None => return (String::new(), AwkFlow::Normal),
    };
    let inside = &rest[1..close];
    let body = &rest[close + 1..];
    // `for (k in arr) …`: associative arrays are not modelled, so iterate the
    // array's values (bounded) instead of mis-parsing it as a C-style loop
    // (which used to run the body 100k times and flood the output).
    if !inside.contains(';') && inside.contains(" in ") {
        let mut it = inside.splitn(2, " in ");
        let var = it.next().unwrap_or("").trim().to_string();
        let arr = it.next().unwrap_or("").trim().to_string();
        let assoc = awk_assoc_keys(&arr);
        let keys: Vec<String> = if !assoc.is_empty() {
            assoc
        } else {
            arrays.get(&arr).cloned().unwrap_or_default()
        };
        let mut out = String::new();
        for k in keys {
            if var.is_empty() {
                continue;
            }
            AWK_STR_LOCALS.with(|m| m.borrow_mut().insert(var.clone(), k));
            let (o, flow) = awk_run_body(body, nr, nf, line, fields, vars, arrays);
            out.push_str(&o);
            match flow {
                AwkFlow::Normal => {}
                AwkFlow::Break => break,
                other => return (out, other),
            }
        }
        if !var.is_empty() {
            AWK_STR_LOCALS.with(|m| {
                m.borrow_mut().remove(&var);
            });
        }
        return (out, AwkFlow::Normal);
    }
    let parts: Vec<&str> = inside.splitn(3, ';').collect();
    let init = parts.first().copied().unwrap_or("");
    let cond = parts.get(1).copied().unwrap_or("");
    let incr = parts.get(2).copied().unwrap_or("");
    awk_apply_expr(init, nr, nf, line, fields, vars, arrays);
    let mut out = String::new();
    for _ in 0..100_000 {
        if !cond.trim().is_empty() && !awk_cond_truthy(cond, nr, nf, line, fields, vars, arrays) {
            break;
        }
        let (o, flow) = awk_run_body(body, nr, nf, line, fields, vars, arrays);
        out.push_str(&o);
        match flow {
            AwkFlow::Normal => {}
            AwkFlow::Continue => {
                awk_apply_expr(incr, nr, nf, line, fields, vars, arrays);
                continue;
            }
            AwkFlow::Break => break,
            other => return (out, other),
        }
        awk_apply_expr(incr, nr, nf, line, fields, vars, arrays);
    }
    (out, AwkFlow::Normal)
}

fn awk_while(
    rest: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> (String, AwkFlow) {
    let close = match awk_close_paren(rest) {
        Some(c) => c,
        None => return (String::new(), AwkFlow::Normal),
    };
    let cond = &rest[1..close];
    let body = &rest[close + 1..];
    let mut out = String::new();
    for _ in 0..100_000 {
        if !awk_cond_truthy(cond, nr, nf, line, fields, vars, arrays) {
            break;
        }
        let (o, flow) = awk_run_body(body, nr, nf, line, fields, vars, arrays);
        out.push_str(&o);
        match flow {
            AwkFlow::Normal | AwkFlow::Continue => {}
            AwkFlow::Break => break,
            other => return (out, other),
        }
    }
    (out, AwkFlow::Normal)
}

fn awk_if(
    rest: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut HashMap<String, f64>,
    arrays: &mut HashMap<String, Vec<String>>,
) -> (String, AwkFlow) {
    let close = match awk_close_paren(rest) {
        Some(c) => c,
        None => return (String::new(), AwkFlow::Normal),
    };
    let cond = &rest[1..close];
    let tail = rest[close + 1..].trim();
    let (then_part, else_part) = match find_toplevel_else(tail) {
        Some(p) => (tail[..p].trim(), Some(tail[p + 4..].trim())),
        None => (tail, None),
    };
    if awk_cond_truthy(cond, nr, nf, line, fields, vars, arrays) {
        awk_run_body(then_part, nr, nf, line, fields, vars, arrays)
    } else if let Some(e) = else_part {
        awk_run_body(e, nr, nf, line, fields, vars, arrays)
    } else {
        (String::new(), AwkFlow::Normal)
    }
}

fn find_toplevel_else(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut brace = 0i32;
    let mut in_str = false;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        match bytes[i] {
            b'"' => in_str = !in_str,
            b'{' if !in_str => brace += 1,
            b'}' if !in_str => brace -= 1,
            b'e' if !in_str
                && brace == 0
                && s[i..].starts_with("else")
                && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) =>
            {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// True for a standalone `i++` / `i--` / `++i` / `--i` statement.
fn is_incdec_stmt(s: &str) -> bool {
    let s = s.trim();
    if let Some(name) = s.strip_suffix("++").or_else(|| s.strip_suffix("--")) {
        return is_awk_name(name.trim());
    }
    if let Some(name) = s.strip_prefix("++").or_else(|| s.strip_prefix("--")) {
        return is_awk_name(name.trim());
    }
    false
}

fn is_awk_name(s: &str) -> bool {
    !s.is_empty()
        && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
