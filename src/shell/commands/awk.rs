use crate::shell::{Shell, CommandOutput};
use std::collections::HashMap;

impl Shell {
    pub fn cmd_awk(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut program: Option<String> = None;
        let mut files = Vec::new();
        let mut field_sep = ' ';
        let mut variables: HashMap<String, f64> = HashMap::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-F" => {
                    if i + 1 < args.len() {
                        field_sep = args[i + 1].chars().next().unwrap_or(' ');
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
                }
                arg if !arg.starts_with('-') && program.is_none() => {
                    program = Some(arg.to_string());
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let prog = match program {
            Some(p) => p,
            None => return CommandOutput::error("awk: missing program\n".to_string(), 1),
        };

        let input = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("awk: missing input\n".to_string(), 1),
            }
        } else {
            let mut content = String::new();
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
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

        if let Some(ref begin) = parsed.begin {
            let (result, _) = exec_awk_action(begin, nr, 0, "", &[], &mut variables, &mut arrays);
            output.push_str(&result);
        }

        for line in input.lines() {
            nr += 1;
            let line = line.trim_end();
            let fields: Vec<&str> = if field_sep == ' ' {
                line.split_whitespace().collect()
            } else {
                line.split(field_sep).collect()
            };
            let nf = fields.len();

            let should_execute = match &parsed.condition {
                Some(cond) => eval_awk_cond(cond, nr, nf, line, &fields, &variables),
                None => true,
            };

            if should_execute {
                if let Some(ref action) = parsed.action {
                    let (result, next) =
                        exec_awk_action(action, nr, nf, line, &fields, &mut variables, &mut arrays);
                    output.push_str(&result);
                    if next {
                        continue;
                    }
                } else {
                    output.push_str(line);
                    output.push('\n');
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
            let inner = main_part.trim_matches('{').trim_matches('}').trim().to_string();
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
            action = Some("{ print $0 }".to_string());
        }
    }

    AwkFull { begin, condition, action, end_block }
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
        let lv = awk_value_ext(left, nr, nf, line, fields, vars);
        let rv = awk_value_ext(right, nr, nf, line, fields, vars);
        return lv == rv;
    }
    if let Some(pos) = find_operator_outside_parens(cond, "!=") {
        let left = cond[..pos].trim();
        let right = cond[pos + 2..].trim();
        let lv = awk_value_ext(left, nr, nf, line, fields, vars);
        let rv = awk_value_ext(right, nr, nf, line, fields, vars);
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
            let left_val = awk_value_ext(left, nr, nf, line, fields, vars);
            let right_val = awk_value_ext(right, nr, nf, line, fields, vars);

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

    if let Some(val) = vars.get(expr) {
        return val.to_string();
    }

    match expr {
        "NR" => nr.to_string(),
        "NF" => nf.to_string(),
        "$0" => line.to_string(),
        _ if expr.starts_with('$') => {
            if let Ok(n) = expr[1..].parse::<usize>() {
                if n > 0 && n <= nf {
                    fields[n - 1].to_string()
                } else {
                    String::new()
                }
            } else {
                expr.to_string()
            }
        }
        _ if expr.starts_with('"') && expr.ends_with('"') => {
            expr[1..expr.len() - 1].to_string()
        }
        _ => {
            // Try to evaluate as a function call
            if let Some(result) = try_awk_function_call(expr, nr, nf, line, fields, vars) {
                return result;
            }
            expr.to_string()
        }
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
) -> (String, bool) {
    let action = action.trim();
    let mut result = String::new();
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
            return (result, true);
        }

        if stmt.starts_with("printf ") || stmt.starts_with("printf(") {
            let mut expr = stmt[7..].to_string();
            if expr.ends_with(')') {
                expr.pop();
            }
            let args = parse_function_args(&expr);
            if args.len() >= 1 {
                let fmt = awk_value_ext(&args[0], nr, nf, line, fields, vars);
                let arg_vals: Vec<String> = args[1..]
                    .iter()
                    .map(|a| awk_value_ext(a, nr, nf, line, fields, vars))
                    .collect();
                result.push_str(&awk_printf(&fmt, &arg_vals));
            }
        } else if stmt.starts_with("print(") && stmt.ends_with(')') {
            let inner = &stmt[6..stmt.len() - 1];
            let args = split_awk_args(inner);
            if !result.is_empty() {
                result.push('\n');
            }
            for (k, arg) in args.iter().enumerate() {
                if k > 0 {
                    result.push(' ');
                }
                result.push_str(&awk_value_ext(arg, nr, nf, line, fields, vars));
            }
        } else if stmt.starts_with("print ") || stmt == "print" {
            let expr = if stmt == "print" {
                "$0"
            } else {
                &stmt[6..]
            };
            if !result.is_empty() {
                result.push('\n');
            }
            let args = split_awk_args(expr);
            for (k, arg) in args.iter().enumerate() {
                if k > 0 {
                    result.push(' ');
                }
                result.push_str(&awk_value_ext(arg, nr, nf, line, fields, vars));
            }
        } else if stmt.contains('=') && !stmt.starts_with("print") && !stmt.starts_with("printf") {
            if let Some(eq_pos) = find_toplevel_eq(stmt) {
                let var = stmt[..eq_pos].trim().to_string();
                let expr = stmt[eq_pos + 1..].trim();
                let val = eval_awk_expr_full(expr, nr, nf, line, fields, vars, arrays);
                vars.insert(var, val);
            }
        }
    }

    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    (result, false)
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
    let mut in_string = false;
    let mut start = 0;
    let bytes = action.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' if !in_string => depth += 1,
            b')' if !in_string => depth -= 1,
            b'"' if !in_string => {
                in_string = true;
            }
            b'"' if in_string => {
                in_string = false;
            }
            b'\\' if in_string => {
                i += 1;
            }
            b';' if depth == 0 && !in_string => {
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
    let func_name = expr[..paren_pos].trim();
    let args_str = &expr[paren_pos + 1..expr.len() - 1];
    let args = parse_function_args(args_str);

    let eval_args: Vec<String> = args
        .iter()
        .map(|a| awk_value_ext(a, nr, nf, line, fields, vars))
        .collect();

    match func_name {
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
        "split" => {
            None
        }
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
            if arg_idx >= args.len() {
                result.push('%');
                result.push(chars[i]);
                i += 1;
                continue;
            }
            match chars[i] {
                's' => {
                    result.push_str(&args[arg_idx]);
                    arg_idx += 1;
                }
                'd' => {
                    let val: i64 = args[arg_idx].parse().unwrap_or(0);
                    result.push_str(&format!("{}", val));
                    arg_idx += 1;
                }
                'f' => {
                    let val: f64 = args[arg_idx].parse().unwrap_or(0.0);
                    result.push_str(&format!("{}", val));
                    arg_idx += 1;
                }
                'x' => {
                    if let Ok(val) = args[arg_idx].parse::<u64>() {
                        result.push_str(&format!("{:x}", val));
                    } else if let Ok(val) = args[arg_idx].parse::<i64>() {
                        result.push_str(&format!("{:x}", val as u64));
                    } else {
                        result.push('0');
                    }
                    arg_idx += 1;
                }
                _ => {
                    result.push('%');
                    result.push(chars[i]);
                }
            }
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
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

    // Try function call that returns a value
    if let Some(result) = try_awk_function_call(expr, nr, nf, line, fields, vars) {
        return result.parse::<f64>().unwrap_or(0.0);
    }

    if let Some(pos) = expr.rfind('-') {
        if pos > 0 && !is_operator_at(&expr, pos, ">=") && !is_operator_at(&expr, pos, "<=") {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            return eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays)
                - eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
        }
    }

    if let Some(pos) = expr.find('+') {
        if pos > 0 {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            return eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays)
                + eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
        }
    }

    // Multiplication (new)
    if let Some(pos) = expr.rfind('*') {
        if pos > 0 && expr.as_bytes()[pos - 1] != b'*' {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            return eval_awk_expr_full(left, nr, nf, line, fields, vars, arrays)
                * eval_awk_expr_full(right, nr, nf, line, fields, vars, arrays);
        }
    }

    // Division (new)
    if let Some(pos) = expr.rfind('/') {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_awk_test_{}_{}", std::process::id(), n));
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
        let out = shell.cmd_awk(&["$1 == \"hello\" {print $2}"], Some("hello world\nfoo bar\n"));
        assert!(out.stdout.contains("world"));
        assert!(!out.stdout.contains("bar"));
    }

    #[test]
    fn test_awk_printf() {
        let shell = mk_shell();
        let out = shell.cmd_awk(
            &["{printf(\"%s:%d\\n\", $1, NR)}"],
            Some("hello\nworld\n"),
        );
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
        let out = shell.cmd_awk(
            &["{ n = split($0, a, \",\"); print n }"],
            Some("a,b,c\n"),
        );
        assert!(out.stdout.contains("3"));
    }
}
