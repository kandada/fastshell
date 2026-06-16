use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_awk(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut program: Option<String> = None;
        let mut files = Vec::new();
        let mut field_sep = ' ';

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-F" => {
                    if i + 1 < args.len() {
                        field_sep = args[i + 1].chars().next().unwrap_or(' ');
                        i += 1;
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
        let mut variables: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

        if let Some(ref begin) = parsed.begin {
            let result = exec_awk_action(begin, nr, 0, "", &[], &mut variables);
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
                Some(cond) => eval_awk_cond(cond, nr, nf, line, &fields),
                None => true,
            };

            if should_execute {
                if let Some(ref action) = parsed.action {
                    let result = exec_awk_action(action, nr, nf, line, &fields, &mut variables);
                    output.push_str(&result);
                } else {
                    output.push_str(line);
                    output.push('\n');
                }
            }
        }

        if let Some(ref end) = parsed.end_block {
            let result = exec_awk_action(end, nr, 0, "", &[], &mut variables);
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

fn eval_awk_cond(cond: &str, nr: usize, nf: usize, line: &str, fields: &[&str]) -> bool {
    let cond = cond.trim();

    if cond.starts_with('/') && cond.ends_with('/') && cond.len() > 2 {
        let pattern = &cond[1..cond.len() - 1];
        return line.contains(pattern);
    }

    let operators = ["==", "!=", ">=", "<=", ">", "<", "~", "!~"];
    for op in &operators {
        if let Some(pos) = cond.find(op) {
            let left = cond[..pos].trim();
            let right = cond[pos + op.len()..].trim();
            let left_val = awk_value(left, nr, nf, line, fields);
            let right_val = awk_value(right, nr, nf, line, fields);

            return match *op {
                "==" => left_val == right_val,
                "!=" => left_val != right_val,
                ">=" => compare_awk(&left_val, &right_val) >= std::cmp::Ordering::Equal,
                "<=" => compare_awk(&left_val, &right_val) <= std::cmp::Ordering::Equal,
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

    true
}

fn awk_value(expr: &str, nr: usize, nf: usize, line: &str, fields: &[&str]) -> String {
    let expr = expr.trim();
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
        _ => expr.to_string(),
    }
}

fn compare_awk(a: &str, b: &str) -> std::cmp::Ordering {
    if let (Ok(na), Ok(nb)) = (a.parse::<f64>(), b.parse::<f64>()) {
        na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
    } else {
        a.cmp(b)
    }
}

fn exec_awk_action(
    action: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &mut std::collections::HashMap<String, f64>,
) -> String {
    let action = action.trim();
    let mut result = String::new();

    for stmt_str in action.split(';') {
        let stmt = stmt_str.trim();
        if stmt.is_empty() {
            continue;
        }

        if stmt.contains('=') && !stmt.starts_with("print") {
            let parts: Vec<&str> = stmt.splitn(2, '=').collect();
            if parts.len() == 2 {
                let var = parts[0].trim().to_string();
                let expr = parts[1].trim();
                let val = eval_awk_expr(expr, nr, nf, line, fields, vars);
                vars.insert(var, val);
            }
        } else if stmt.starts_with("print ") || stmt == "print" {
            let expr = if stmt == "print" { "$0" } else { &stmt[6..] };
            if !result.is_empty() {
                result.push('\n');
            }
            let items: Vec<&str> = expr.split(',').map(|s| s.trim()).collect();
            for (k, item) in items.iter().enumerate() {
                if k > 0 {
                    result.push(' ');
                }
                result.push_str(&awk_value(item, nr, nf, line, fields));
            }
        }
    }

    if !result.is_empty() {
        result.push('\n');
    }
    result
}

fn eval_awk_expr(
    expr: &str,
    nr: usize,
    nf: usize,
    line: &str,
    fields: &[&str],
    vars: &std::collections::HashMap<String, f64>,
) -> f64 {
    let expr = expr.trim();

    if let Some(pos) = expr.rfind('-') {
        if pos > 0 {
            let left = &expr[..pos];
            let right = &expr[pos + 1..];
            return eval_awk_expr(left, nr, nf, line, fields, vars)
                - eval_awk_expr(right, nr, nf, line, fields, vars);
        }
    }

    if let Some(pos) = expr.find('+') {
        let left = &expr[..pos];
        let right = &expr[pos + 1..];
        return eval_awk_expr(left, nr, nf, line, fields, vars)
            + eval_awk_expr(right, nr, nf, line, fields, vars);
    }

    if let Some(val) = vars.get(expr) {
        return *val;
    }

    let val = awk_value(expr, nr, nf, line, fields);
    val.parse::<f64>().unwrap_or(0.0)
}
