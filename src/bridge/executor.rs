// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::python::PythonEngine;
use crate::shell::{CommandOutput, Shell};
use std::sync::mpsc;

// ── Re-entrant shell bridge ─────────────────────────────────────────
// While `execute_python_code` runs, the embedded CPython VM may synchronously
// call back into the shell (e.g. subprocess.run → fastshell_python_shell_exec).
// The runtime Mutex (and the global SDK Mutex) is already held by this same
// thread, so re-locking would deadlock. Instead we expose the *current*
// Runtime to this thread via a thread-local raw pointer; the bridge reuses it
// directly. Execution is strictly serial (Python blocks awaiting the result),
// so no concurrent access to the Runtime occurs.
thread_local! {
    static REENTRANT_RT: std::cell::Cell<*mut Runtime> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

struct ReentrantGuard;
impl Drop for ReentrantGuard {
    fn drop(&mut self) {
        REENTRANT_RT.with(|c| c.set(std::ptr::null_mut()));
    }
}

/// Called by the shell-execute bridge. If a Python execution is active on this
/// thread, runs `input` on the current Runtime without re-locking, avoiding the
/// deadlock. Returns `None` when not inside a Python callback (normal path).
pub fn try_reentrant_execute(input: &str) -> Option<CommandOutput> {
    REENTRANT_RT.with(|c| {
        let ptr = c.get();
        if ptr.is_null() {
            return None;
        }
        // Temporarily clear so a shell command that itself spawns Python does
        // not recurse into this same borrow.
        c.set(std::ptr::null_mut());
        // SAFETY: `ptr` is set by the Python execution path (execute_python_code
        // / execute_python_inner) and cleared by ReentrantGuard on drop. It is
        // only dereferenced here while the guard is alive, so the Runtime
        // outlives the pointer. Execution is strictly serial — Python blocks
        // awaiting the shell result, so no concurrent access occurs.
        let out = {
            let rt = unsafe { &mut *ptr };
            rt.execute(input)
        };
        c.set(ptr);
        Some(out)
    })
}

pub struct Runtime {
    shell: Shell,
    python: Option<Box<dyn PythonEngine>>,
    /// Exit code of the previously executed segment (`$?`).
    last_exit: i32,
    /// Recursion guard for `$( ... )` command substitution.
    subst_depth: u8,
    /// Recursion guard for shell function calls.
    fn_call_depth: usize,
    psub_counter: u64,
    tmp_files: Vec<String>,
}

impl Runtime {
    pub fn new(shell: Shell, python: Option<Box<dyn PythonEngine>>) -> Self {
        // (c) 2025 xiefujin <490021684@qq.com>
        Runtime {
            shell,
            python,
            last_exit: 0,
            subst_depth: 0,
            fn_call_depth: 0,
            psub_counter: 0,
            tmp_files: Vec::new(),
        }
    }

    /// Top-level entry: splits the input into logical segments (newlines, `;`,
    /// `&&`, `||`, heredocs) and executes them with shell chaining semantics.
    pub fn execute(&mut self, input: &str) -> CommandOutput {
        let input = input.trim();

        // (c) 2025 xiefujin <490021684@qq.com>
        if input.is_empty() {
            return CommandOutput::success(String::new());
        }

        let segments = split_segments(input);
        if segments.is_empty() {
            return CommandOutput::success(String::new());
        }

        let segments = combine_if_segments(segments);
        let segments = combine_loop_segments(segments);
        let segments = combine_case_segments(segments);

        // Fast path: a single plain segment.
        if segments.len() == 1 {
            let seg = &segments[0];
            let text = seg.text.clone();
            let heredoc = seg.heredoc.clone();
            let out = self.execute_segment(&text, heredoc.as_deref(), seg.heredoc_expand);
            self.last_exit = out.exit_code;
            return out;
        }

        let mut agg_stdout = String::new();
        let mut agg_stderr = String::new();
        let mut exit_code = 0;
        for seg in &segments {
            match seg.connector {
                Connector::Always => {}
                Connector::AndIf => {
                    if exit_code != 0 {
                        continue;
                    }
                }
                Connector::OrIf => {
                    if exit_code == 0 {
                        continue;
                    }
                }
            }
            if seg.text.trim().is_empty() && seg.heredoc.is_none() {
                continue;
            }
            // Skip # comment lines (bash compatibility)
            if seg.text.trim().starts_with('#') && seg.heredoc.is_none() {
                continue;
            }
            let text = seg.text.clone();
            let heredoc = seg.heredoc.clone();
            let out = self.execute_segment(&text, heredoc.as_deref(), seg.heredoc_expand);
            exit_code = out.exit_code;
            self.last_exit = exit_code;
            if self.shell.errexit && exit_code != 0 {
                break;
            }
            agg_stdout.push_str(&out.stdout);
            agg_stderr.push_str(&out.stderr);
        }

        CommandOutput {
            stdout: agg_stdout,
            stderr: agg_stderr,
            exit_code,
        }
    }

    /// Executes `input` with `cwd` as the working directory, restoring the
    /// previous cwd afterwards. Used by hosts (e.g. the Android app UI) so
    /// concurrent callers don't depend on — or pollute — the shared cwd.
    pub fn execute_with_cwd(&mut self, cwd: &str, input: &str) -> CommandOutput {
        let saved = self.shell.cwd.clone();
        let cd = self.shell.execute("cd", &[cwd], None);
        if cd.exit_code != 0 {
            return cd;
        }
        let out = self.execute(input);
        self.shell.cwd = saved;
        out
    }

    /// Executes one logical command segment (no `;`/`&&`/`||`/newline inside).
    /// `heredoc` is the collected here-document body (used as stdin).
    fn execute_segment(
        &mut self,
        input: &str,
        heredoc: Option<&str>,
        heredoc_expand: bool,
    ) -> CommandOutput {
        let heredoc = heredoc.map(|body| {
            if heredoc_expand {
                self.expand_line(body)
            } else {
                body.to_string()
            }
        });
        let heredoc_ref: Option<&str> = heredoc.as_deref();

        let raw = input.trim();
        if raw.is_empty() && heredoc_ref.is_none() {
            return CommandOutput::success(String::new());
        }

        if self.shell.xtrace {
            eprintln!("+ {}", raw);
        }

        if let Some((name, body)) = try_parse_function_def(raw) {
            self.shell.functions.insert(name.to_string(), body.trim().to_string());
            return CommandOutput::success(String::new());
        }

        let first_word = raw.trim().split_whitespace().next().unwrap_or("");
        if first_word == "if" || first_word == "for" || first_word == "while" || first_word == "until" || first_word == "case" {
            return self.execute_block_construct(raw, heredoc_ref);
        }

        let expanded = self.expand_line(raw);
        let expanded = expand_braces(&expanded);
        let input = expanded.trim();
        if input.is_empty() {
            return CommandOutput::success(String::new());
        }

        // Variable assignments: `X=v`, `export X=v`, `X=v command ...`.
        let (input, assign_only) = self.consume_assignments(input);
        if assign_only {
            return CommandOutput::success(String::new());
        }
        let input = input.trim();
        if input.is_empty() {
            return CommandOutput::success(String::new());
        }

        // `python3 << 'EOF' ... EOF` — run the heredoc body as Python code.
        if let Some(body) = heredoc_ref {
            let bare = input.trim_end_matches(" -").trim();
            if bare == "python" || bare == "python3" {
                return self.execute_python_code(body);
            }
        }

        if is_python_command(input) {
            return self.execute_python_inner(input);
        }

        if input.contains('|') {
            return self.execute_pipeline(input, heredoc_ref);
        }

        let tokens = parse_command(input);
        if tokens.is_empty() {
            if heredoc_ref.is_some() {
                return CommandOutput::success(String::new());
            }
            return CommandOutput::success(String::new());
        }
        let parts = self.expand_globs(tokens);
        let (clean, spec) = self.extract_redirects(parts);

        if clean.is_empty() {
            // Only redirect operators given — write heredoc (or empty) to file
            let mut result = CommandOutput::success(String::new());
            if let Some(body) = heredoc_ref {
                result.stdout = body.to_string();
            }
            self.apply_redirects(&mut result, &spec);
            result.stdout.clear();
            return result;
        }

        let cmd = &clean[0];
        let args: Vec<&str> = clean[1..].iter().map(|s| s.as_str()).collect();

        // stdin: heredoc body takes precedence, then `< file`.
        let stdin_from_file = if heredoc_ref.is_none() {
            spec.stdin_file.as_ref().and_then(|path| {
                self.shell.vfs.read_to_string(path, &self.shell.cwd).ok()
            })
        } else {
            None
        };
        let stdin_ref = heredoc_ref.or(stdin_from_file.as_deref());

        let (cmd, args_vec, is_alias) = if let Some((new_cmd, new_args)) = self.shell.resolve_alias(cmd, &args) {
            (new_cmd, new_args, true)
        } else {
            (cmd.to_string(), args.iter().map(|s| s.to_string()).collect(), false)
        };
        let args_refs: Vec<&str> = if is_alias {
            args_vec.iter().map(|s| s.as_str()).collect()
        } else {
            args.to_vec()
        };

        let mut result = if let Some(func_body) = self.shell.functions.get(&cmd) {
            let func_body = func_body.clone();
            self.call_function(&cmd, &func_body, &args_refs, stdin_ref)
        } else {
            self.shell.execute(&cmd, &args_refs, stdin_ref)
        };
        if cmd == "eval" && result.exit_code == 0 && !result.stdout.is_empty() {
            let eval_text = result.stdout.clone();
            let evaled = self.execute(&eval_text);
            result = evaled;
        }
        if (cmd == "source" || cmd == ".") && result.exit_code == 0 && !result.stdout.is_empty() {
            let script = result.stdout.clone();
            result = self.execute(&script);
        }
        self.apply_redirects(&mut result, &spec);
        result
    }

    /// Strips leading `NAME=value` assignments (and an optional `export`
    /// prefix) from the segment, records them as session variables, and
    /// returns (rest_of_command, was_assignment_only).
    fn consume_assignments<'a>(&mut self, input: &'a str) -> (&'a str, bool) {
        let mut rest = input;
        if let Some(r) = rest.strip_prefix("export ") {
            rest = r.trim_start();
            // If first token is not a simple assignment (e.g. -n, -p, bare name),
            // leave it for the shell to handle as `cmd_export`.
            let first = rest.chars().next().unwrap_or(' ');
            if first == '-' || !rest.contains('=') {
                return (input, false);
            }
            // `export X=1 Y=2` — every token must be an assignment.
            let mut all = true;
            let mut cursor = rest;
            while let Some((name, value, after)) = take_assignment(cursor) {
                self.shell.vars.insert(name.clone(), value);
                self.shell.exported.insert(name);
                cursor = after.trim_start();
                if cursor.is_empty() {
                    break;
                }
                if take_assignment(cursor).is_none() {
                    all = false;
                    break;
                }
            }
            if all && cursor.is_empty() {
                return ("", true);
            }
            return (cursor, cursor.is_empty());
        }

        let mut consumed_any = false;
        let mut cursor = rest;
        while let Some((name, value, after)) = take_assignment(cursor) {
            self.shell.vars.insert(name, value);
            consumed_any = true;
            cursor = after.trim_start();
        }
        if consumed_any {
            (cursor, cursor.is_empty())
        } else {
            (input, false)
        }
    }

    fn execute_block_construct(&mut self, raw: &str, heredoc_ref: Option<&str>) -> CommandOutput {
        let input = raw.trim();
        let first = input.split_whitespace().next().unwrap_or("");
        match first {
            "if" => self.execute_if(input.trim_start_matches("if "), heredoc_ref),
            "for" => self.execute_for(input.trim_start_matches("for ")),
            "while" | "until" => {
                let cond_inverted = first == "until";
                self.execute_while(input.trim_start_matches("while ").trim_start_matches("until "), cond_inverted)
            }
            "case" => self.execute_case(input.trim_start_matches("case ")),
            _ => CommandOutput::error(format!("unknown block: {first}\n"), 127),
        }
    }

    fn execute_if(&mut self, input: &str, _stdin: Option<&str>) -> CommandOutput {
        let s = input.trim();
        let fi_idx = s.rfind("; fi").or_else(|| {
            if s.ends_with("fi") { Some(s.len() - 2) } else { None }
        });
        let fi_idx = match fi_idx {
            Some(x) => x,
            None => return CommandOutput::error("if: missing fi\n".to_string(), 1),
        };
        let before_fi = &s[..fi_idx].trim_end();
        let parsed = parse_if_branches(before_fi);
        for (cond, body) in &parsed {
            if cond.is_empty() {
                return self.execute(body);
            }
            let result = self.execute(cond);
            if result.exit_code == 0 {
                return self.execute(body);
            }
        }
        CommandOutput::success(String::new())
    }

    fn execute_for(&mut self, input: &str) -> CommandOutput {
        let s = input.trim();
        let do_pos = s.find("; do ").or_else(|| s.find("do ").and_then(|p| {
            if p == 0 || s.as_bytes().get(p.wrapping_sub(1)) == Some(&b' ') { Some(p) } else { None }
        }));
        let do_pos = match do_pos {
            Some(x) => x,
            None => return CommandOutput::error("for: missing do\n".to_string(), 1),
        };
        let after_do = if s[do_pos..].starts_with("; do ") {
            &s[do_pos + 5..]
        } else {
            &s[do_pos + 3..]
        };
        let done_idx = after_do.rfind("; done").or_else(|| {
            if after_do.ends_with("done") { Some(after_do.len() - 4) } else { None }
        });
        let body = match done_idx {
            Some(x) => after_do[..x].trim().to_string(),
            None => return CommandOutput::error("for: missing done\n".to_string(), 1),
        };

        let head = s[..do_pos].trim().to_string();
        let in_pos = head.rfind(" in ").or_else(|| head.rfind("; in "));
        let (var, words) = if let Some(pos) = in_pos {
            let sep = if head[pos..].starts_with("; in ") { 5 } else { 4 };
            (head[..pos].trim().to_string(), head[pos + sep..].trim().to_string())
        } else {
            (head.clone(), String::new())
        };

        let word_list: Vec<String> = if words.is_empty() {
            self.shell.vars.get("@").cloned()
                .map(|v| shell_words_parse(&v).unwrap_or_default())
                .unwrap_or_default()
        } else {
            shell_words_parse(&words).unwrap_or_else(|_| words.split_whitespace().map(|s| s.to_string()).collect())
        };

        let mut last_exit = 0;
        let mut agg_stdout = String::new();
        let mut agg_stderr = String::new();
        for word in &word_list {
            self.shell.vars.insert(var.clone(), word.clone());
            let out = self.execute(&body);
            agg_stdout.push_str(&out.stdout);
            if !out.stderr.is_empty() {
                agg_stderr.push_str(&out.stderr);
            }
            last_exit = out.exit_code;
        }
        self.shell.vars.remove(&var);
        CommandOutput { stdout: agg_stdout, stderr: agg_stderr, exit_code: last_exit }
    }

    fn execute_while(&mut self, input: &str, cond_inverted: bool) -> CommandOutput {
        let s = input.trim();
        let do_pos = s.find("; do ").or_else(|| s.find("do ").and_then(|p| {
            if p == 0 || s.as_bytes().get(p.wrapping_sub(1)) == Some(&b' ') { Some(p) } else { None }
        }));
        let do_pos = match do_pos {
            Some(x) => x,
            None => return CommandOutput::error("while: missing do\n".to_string(), 1),
        };
        let condition = s[..do_pos].trim().to_string();
        let after_do = if s[do_pos..].starts_with("; do ") {
            &s[do_pos + 5..]
        } else {
            &s[do_pos + 3..]
        };
        let done_idx = after_do.rfind("; done").or_else(|| {
            if after_do.ends_with("done") { Some(after_do.len() - 4) } else { None }
        });
        let body = match done_idx {
            Some(x) => after_do[..x].trim().to_string(),
            None => return CommandOutput::error("while: missing done\n".to_string(), 1),
        };

        let mut last_exit = 0;
        let mut agg_stdout = String::new();
        let mut agg_stderr = String::new();
        let max_iters: usize = 100_000;
        for _ in 0..max_iters {
            let cond = self.execute(&condition);
            let ok = if cond_inverted { cond.exit_code != 0 } else { cond.exit_code == 0 };
            if !ok {
                break;
            }
            let out = self.execute(&body);
            agg_stdout.push_str(&out.stdout);
            if !out.stderr.is_empty() {
                agg_stderr.push_str(&out.stderr);
            }
            last_exit = out.exit_code;
        }
        CommandOutput { stdout: agg_stdout, stderr: agg_stderr, exit_code: last_exit }
    }

    fn execute_case(&mut self, input: &str) -> CommandOutput {
        let s = input.trim();
        let in_pos = s.find(" in ").or_else(|| s.find("; in "));
        let (word, rest) = if let Some(pos) = in_pos {
            let sep = if s[pos..].starts_with("; in ") { 5 } else { 4 };
            (s[..pos].trim().to_string(), s[pos + sep..].trim().to_string())
        } else {
            return CommandOutput::error("case: missing 'in'\n".to_string(), 1);
        };

        let esac_idx = rest.rfind("; esac").or_else(|| {
            if rest.ends_with("esac") { Some(rest.len() - 4) } else { None }
        });
        let patterns_str = match esac_idx {
            Some(x) => rest[..x].trim().to_string(),
            None => return CommandOutput::error("case: missing esac\n".to_string(), 1),
        };

        let mut i = 0usize;
        let chars: Vec<char> = patterns_str.chars().collect();
        let n = chars.len();
        while i < n {
            let pat_start = i;
            let mut paren_pos = None;
            while i < n {
                if chars[i] == ')' && i + 1 < n && chars[i + 1] == ' ' {
                    paren_pos = Some(i);
                    i += 1;
                    break;
                }
                i += 1;
            }
            let paren_pos = match paren_pos {
                Some(x) => x,
                None => break,
            };
            let pattern = patterns_str[pat_start..paren_pos].trim().to_string();

            let body_start = i;
            let mut dq = false;
            let mut sq = false;
            while i < n {
                match chars[i] {
                    '"' => dq = !dq,
                    '\'' => sq = !sq,
                    ';' if !dq && !sq => {
                        i += 1;
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
            let body = patterns_str[body_start..i.min(n)].trim_end_matches(';').trim().to_string();

            if pattern.is_empty() && body.is_empty() {
                continue;
            }

            if case_pattern_match(&word, &pattern) {
                return self.execute(&body);
            }
        }
        CommandOutput::success(String::new())
    }

    fn call_function(
        &mut self,
        name: &str,
        body: &str,
        args: &[&str],
        stdin: Option<&str>,
    ) -> CommandOutput {
        if self.fn_call_depth >= 50 {
            return CommandOutput::error(
                format!("{}: maximum function call depth (50) exceeded\n", name),
                1,
            );
        }
        let saved_positional = self.shell.positional.clone();
        self.shell.positional.clear();
        self.shell.positional.push(name.to_string());
        for a in args {
            self.shell.positional.push(a.to_string());
        }
        self.fn_call_depth += 1;
        let result = self.execute(body);
        self.fn_call_depth -= 1;
        self.shell.positional = saved_positional;
        if let Some(s) = stdin {
            if !s.is_empty() {
                return CommandOutput {
                    stdout: s.to_string() + &result.stdout,
                    stderr: result.stderr,
                    exit_code: result.exit_code,
                };
            }
        }
        result
    }

    /// Expands `$(...)`, backticks, `$VAR`, `${VAR}`, `$?` and leading `~`
    /// within a command line, honoring single-quote protection.
    fn expand_line(&mut self, input: &str) -> String {
        let chars: Vec<char> = input.chars().collect();
        let n = chars.len();
        let mut out = String::with_capacity(n);
        let mut i = 0;
        let mut in_single = false;
        let mut in_double = false;

        while i < n {
            let c = chars[i];
            if in_single {
                out.push(c);
                if c == '\'' {
                    in_single = false;
                }
                i += 1;
                continue;
            }
            match c {
                '\'' if !in_double => {
                    in_single = true;
                    out.push(c);
                    i += 1;
                }
                '"' => {
                    in_double = !in_double;
                    out.push(c);
                    i += 1;
                }
                '\\' if i + 1 < n => {
                    if in_double {
                        match chars[i + 1] {
                            '$' | '`' | '"' | '\\' | '!' => {
                                out.push(chars[i + 1]);
                            }
                            _ => {
                                out.push(c);
                                out.push(chars[i + 1]);
                            }
                        }
                    } else {
                        out.push(chars[i + 1]);
                    }
                    i += 2;
                }
                '`' => {
                    // Backtick command substitution.
                    if let Some(close) = chars[i + 1..].iter().position(|&ch| ch == '`') {
                        let inner: String = chars[i + 1..i + 1 + close].iter().collect();
                        let subst = self.run_substitution(&inner);
                        if in_double {
                            out.push_str(&subst);
                        } else {
                            let words: Vec<&str> = subst.split_whitespace().collect();
                            out.push_str(&words.join(" "));
                        }
                        i += close + 2;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '$' if i + 2 < n && chars[i + 1] == '(' && chars[i + 2] == '(' => {
                    let mut depth: i32 = 0;
                    let mut j = i + 3;
                    let mut end: Option<usize> = None;
                    while j < n {
                        match chars[j] {
                            '(' => depth += 1,
                            ')' => {
                                if depth == 0 {
                                    end = Some(j);
                                    break;
                                }
                                depth -= 1;
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if let Some(end) = end {
                        let inner: String = chars[i + 3..end].iter().collect();
                        let expanded_inner = self.expand_line(&inner);
                        out.push_str(&self.eval_arithmetic(&expanded_inner).to_string());
                        i = end + 2;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '$' if i + 1 < n && chars[i + 1] == '(' => {
                    // $( ... ) with nesting support.
                    let mut depth = 0usize;
                    let mut j = i + 1;
                    let mut end = None;
                    while j < n {
                        match chars[j] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = Some(j);
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if let Some(end) = end {
                        let inner: String = chars[i + 2..end].iter().collect();
                        let subst = self.run_substitution(&inner);
                        if in_double {
                            out.push_str(&subst);
                        } else {
                            let words: Vec<&str> = subst.split_whitespace().collect();
                            out.push_str(&words.join(" "));
                        }
                        i = end + 1;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '$' if i + 1 < n && chars[i + 1] == '{' => {
                    let mut depth = 1usize;
                    let mut j = i + 2;
                    while j < n && depth > 0 {
                        match chars[j] {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        j += 1;
                    }
                    if depth == 0 {
                        let body: String = chars[i + 2..j - 1].iter().collect();
                        out.push_str(&self.expand_param(&body));
                        i = j;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '$' if i + 1 < n && chars[i + 1] == '?' => {
                    out.push_str(&self.last_exit.to_string());
                    i += 2;
                }
                '$' if i + 1 < n && chars[i + 1] == '$' => {
                    out.push_str(&self.shell.pid.to_string());
                    i += 2;
                }
                '$' if i + 1 < n && chars[i + 1] == '#' => {
                    let count = self.shell.positional.len().saturating_sub(1);
                    out.push_str(&count.to_string());
                    i += 2;
                }
                '$' if i + 1 < n && chars[i + 1].is_ascii_digit() => {
                    let d = chars[i + 1].to_digit(10).unwrap() as usize;
                    if d < self.shell.positional.len() {
                        out.push_str(&self.shell.positional[d]);
                    }
                    i += 2;
                }
                '$' if i + 1 < n && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_') => {
                    let mut j = i + 1;
                    while j < n && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    let name: String = chars[i + 1..j].iter().collect();
                    out.push_str(&self.lookup_var(&name));
                    i = j;
                }
                '~' if !in_double => {
                    let at_start = i == 0
                        || chars[i - 1].is_whitespace()
                        || chars[i - 1] == '='
                        || chars[i - 1] == ':';
                    let next_ok = i + 1 >= n
                        || chars[i + 1] == '/'
                        || chars[i + 1].is_whitespace();
                    if at_start && next_ok {
                        out.push_str(&self.home_dir());
                    } else {
                        out.push('~');
                    }
                    i += 1;
                }
                '<' if i + 1 < n && chars[i + 1] == '(' => {
                    let mut depth = 0usize;
                    let mut j = i + 1;
                    let mut end = None;
                    while j < n {
                        match chars[j] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = Some(j);
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if let Some(end) = end {
                        let inner: String = chars[i + 2..end].iter().collect();
                        let path = self.run_process_substitution(&inner);
                        out.push_str(&path);
                        i = end + 1;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '>' if i + 1 < n && chars[i + 1] == '(' => {
                    let mut depth = 0usize;
                    let mut j = i + 1;
                    let mut end = None;
                    while j < n {
                        match chars[j] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = Some(j);
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if let Some(end) = end {
                        let inner: String = chars[i + 2..end].iter().collect();
                        let path = self.run_process_substitution(&inner);
                        out.push_str(&path);
                        i = end + 1;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                _ => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        out
    }

    /// Runs `$( ... )` / backtick substitution: executes the inner command and
    /// splices its stdout (trailing newlines stripped, inner newlines become
    /// spaces, matching bash word-splitting for the common cases).
    fn run_substitution(&mut self, inner: &str) -> String {
        if self.subst_depth >= 8 {
            return String::new();
        }
        self.subst_depth += 1;
        let result = self.execute(inner);
        self.subst_depth -= 1;
        result
            .stdout
            .trim_end_matches('\n')
            .replace('\n', " ")
    }

    /// Runs `<(` / `>(` process substitution: executes the inner command,
    /// writes stdout to a temp file under VFS /tmp/, and returns the VFS path.
    fn run_process_substitution(&mut self, inner: &str) -> String {
        let result = self.execute(inner);
        let vfs_root = self.shell.vfs.root().to_path_buf();
        let tmp_dir = vfs_root.join("tmp");
        let _ = std::fs::create_dir_all(&tmp_dir);

        let counter = self.psub_counter;
        self.psub_counter += 1;
        let file_name = format!("psub_{}", counter);
        let file_path = tmp_dir.join(&file_name);
        let _ = std::fs::write(&file_path, result.stdout.as_bytes());

        let vfs_path = format!("/tmp/{}", file_name);
        self.tmp_files.push(vfs_path.clone());
        vfs_path
    }

    fn expand_param(&mut self, body: &str) -> String {
        for &op in &[":-", ":=", ":+", ":?"] {
            if let Some(pos) = body.find(op) {
                let name = &body[..pos];
                let word = &body[pos + 2..];
                let val = self.lookup_var(name);
                let is_set = self.shell.vars.contains_key(name);
                let is_empty = val.is_empty();
                match op {
                    ":-" => return if is_set && !is_empty { val } else { self.expand_word(word) },
                    ":=" => {
                        if !is_set || is_empty {
                            let w = self.expand_word(word);
                            self.shell.vars.insert(name.to_string(), w.clone());
                            return w;
                        }
                        return val;
                    }
                    ":+" => return if is_set && !is_empty { self.expand_word(word) } else { String::new() },
                    ":?" => {
                        if !is_set || is_empty {
                            let w = self.expand_word(word);
                            let msg = if w.is_empty() { "parameter null or not set" } else { &w };
                            return format!("{}: {}", name, msg);
                        }
                        return val;
                    }
                    _ => {}
                }
            }
        }
        if body.starts_with('#') && body.len() > 1 {
            let name = &body[1..];
            let val = self.lookup_var(name);
            return val.chars().count().to_string();
        }
        if let Some(pos) = body.find("##") {
            let name = &body[..pos];
            let pat = &body[pos + 2..];
            let val = self.lookup_var(name);
            if let Some(r) = glob_remove_prefix(&val, pat, true) { return r; }
            return val;
        }
        if let Some(pos) = body.find('#') {
            if pos > 0 && !body.starts_with('#') {
                let name = &body[..pos];
                let pat = &body[pos + 1..];
                let val = self.lookup_var(name);
                if let Some(r) = glob_remove_prefix(&val, pat, false) { return r; }
                return val;
            }
        }
        if let Some(pos) = body.rfind("%%") {
            let name = &body[..pos];
            let pat = &body[pos + 2..];
            let val = self.lookup_var(name);
            if let Some(r) = glob_remove_suffix(&val, pat, true) { return r; }
            return val;
        }
        if let Some(pos) = body.rfind('%') {
            if pos > 0 {
                let name = &body[..pos];
                let pat = &body[pos + 1..];
                let val = self.lookup_var(name);
                if let Some(r) = glob_remove_suffix(&val, pat, false) { return r; }
                return val;
            }
        }
        if let Some(slash) = body.find('/') {
            if slash > 0 {
                let name = &body[..slash];
                let rest = &body[slash..];
                let (rest, global) = if rest.starts_with("//") {
                    (&rest[2..], true)
                } else {
                    (&rest[1..], false)
                };
                let (pat, rep) = split_unescaped_slash(rest);
                let val = self.lookup_var(name);
                if global {
                    return val.replace(&pat, &rep);
                } else {
                    return val.replacen(&pat, &rep, 1);
                }
            }
        }
        if let Some(colon) = body.find(':') {
            if colon > 0 {
                let name = &body[..colon];
                let rest = &body[colon + 1..];
                if let Some(colon2) = rest.find(':') {
                    let o_str = &rest[..colon2];
                    let l_str = &rest[colon2 + 1..];
                    if let (Ok(offset), Ok(len)) = (o_str.parse::<i64>(), l_str.parse::<usize>()) {
                        let val = self.lookup_var(name);
                        return substr(&val, offset, Some(len));
                    }
                }
                if let Ok(offset) = rest.parse::<i64>() {
                    let val = self.lookup_var(name);
                    return substr(&val, offset, None);
                }
            }
        }
        self.lookup_var(body)
    }

    fn expand_word(&mut self, s: &str) -> String {
        let s = s.trim();
        if s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2 {
            return s[1..s.len() - 1].to_string();
        }
        self.expand_line(s)
    }

    fn lookup_var(&self, name: &str) -> String {
        if let Some(v) = self.shell.vars.get(name) {
            return v.clone();
        }
        match name {
            "PWD" => self.shell.cwd.clone(),
            "HOME" => self.home_dir(),
            "?" => self.last_exit.to_string(),
            _ => {
                if self.shell.nounset {
                    String::new()
                } else {
                    std::env::var(name).unwrap_or_default()
                }
            }
        }
    }

    /// The sandbox "home" is the VFS root.
    fn home_dir(&self) -> String {
        self.shell.vars.get("HOME").cloned().unwrap_or_else(|| "/".to_string())
    }
}

// ── Arithmetic expansion: $(( expr )) ─────────────────────────────────

fn tokenize_arithmetic(expr: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let bytes = expr.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            i += 1;
            continue;
        }
        match b {
            b'(' | b')' | b'?' | b':' | b'~' | b'!' | b'+' | b'-' | b'*' | b'/' | b'%' | b'^'
            | b'&' | b'|' => {
                tokens.push(expr[i..i + 1].to_string());
                i += 1;
            }
            b'<' => {
                if i + 1 < n && bytes[i + 1] == b'<' {
                    tokens.push(expr[i..i + 2].to_string());
                    i += 2;
                } else {
                    tokens.push(expr[i..i + 1].to_string());
                    i += 1;
                }
            }
            b'>' => {
                if i + 1 < n && bytes[i + 1] == b'>' {
                    tokens.push(expr[i..i + 2].to_string());
                    i += 2;
                } else {
                    tokens.push(expr[i..i + 1].to_string());
                    i += 1;
                }
            }
            b'0'..=b'9' => {
                let start = i;
                while i < n && bytes[i] >= b'0' && bytes[i] <= b'9' {
                    i += 1;
                }
                tokens.push(expr[start..i].to_string());
            }
            b'$' => {
                let start = i;
                i += 1;
                if i < n && bytes[i] == b'{' {
                    i += 1;
                    while i < n && bytes[i] != b'}' {
                        i += 1;
                    }
                    if i < n {
                        i += 1;
                    }
                } else {
                    while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                }
                tokens.push(expr[start..i].to_string());
            }
            _ if b.is_ascii_alphabetic() || b == b'_' => {
                let start = i;
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                tokens.push(expr[start..i].to_string());
            }
            _ => {
                i += 1;
            }
        }
    }
    tokens
}

struct ArithParser<'a> {
    tokens: &'a [String],
    pos: usize,
    rt: &'a Runtime,
}

impl<'a> ArithParser<'a> {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.pos).map(|s| s.as_str())
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn parse_expr(&mut self) -> i64 {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> i64 {
        let cond = self.parse_bitwise_or();
        if self.peek() == Some("?") {
            self.advance();
            let true_val = self.parse_expr();
            if self.peek() == Some(":") {
                self.advance();
            }
            let false_val = self.parse_expr();
            if cond != 0 { true_val } else { false_val }
        } else {
            cond
        }
    }

    fn parse_bitwise_or(&mut self) -> i64 {
        let mut left = self.parse_bitwise_xor();
        while self.peek() == Some("|") {
            self.advance();
            left |= self.parse_bitwise_xor();
        }
        left
    }

    fn parse_bitwise_xor(&mut self) -> i64 {
        let mut left = self.parse_bitwise_and();
        while self.peek() == Some("^") {
            self.advance();
            left ^= self.parse_bitwise_and();
        }
        left
    }

    fn parse_bitwise_and(&mut self) -> i64 {
        let mut left = self.parse_shift();
        while self.peek() == Some("&") {
            self.advance();
            left &= self.parse_shift();
        }
        left
    }

    fn parse_shift(&mut self) -> i64 {
        let mut left = self.parse_add();
        loop {
            match self.peek() {
                Some("<<") => {
                    self.advance();
                    let rhs = self.parse_add();
                    left = left.wrapping_shl(rhs.min(63) as u32);
                }
                Some(">>") => {
                    self.advance();
                    let rhs = self.parse_add();
                    left = left.wrapping_shr(rhs.min(63) as u32);
                }
                _ => break,
            }
        }
        left
    }

    fn parse_add(&mut self) -> i64 {
        let mut left = self.parse_mul();
        loop {
            match self.peek() {
                Some("+") => {
                    self.advance();
                    left = left.wrapping_add(self.parse_mul());
                }
                Some("-") => {
                    self.advance();
                    left = left.wrapping_sub(self.parse_mul());
                }
                _ => break,
            }
        }
        left
    }

    fn parse_mul(&mut self) -> i64 {
        let mut left = self.parse_unary();
        loop {
            match self.peek() {
                Some("*") => {
                    self.advance();
                    left = left.wrapping_mul(self.parse_unary());
                }
                Some("/") => {
                    self.advance();
                    let rhs = self.parse_unary();
                    left = if rhs == 0 { 0 } else { left / rhs };
                }
                Some("%") => {
                    self.advance();
                    let rhs = self.parse_unary();
                    left = if rhs == 0 { 0 } else { left % rhs };
                }
                _ => break,
            }
        }
        left
    }

    fn parse_unary(&mut self) -> i64 {
        match self.peek() {
            Some("+") => {
                self.advance();
                self.parse_unary()
            }
            Some("-") => {
                self.advance();
                -self.parse_unary()
            }
            Some("~") => {
                self.advance();
                !self.parse_unary()
            }
            Some("!") => {
                self.advance();
                if self.parse_unary() != 0 { 0 } else { 1 }
            }
            _ => self.parse_atom(),
        }
    }

    fn parse_atom(&mut self) -> i64 {
        let tok = match self.tokens.get(self.pos) {
            Some(t) => t.clone(),
            None => return 0,
        };
        if tok == "(" {
            self.pos += 1;
            let val = self.parse_expr();
            if self.pos < self.tokens.len() && self.tokens[self.pos] == ")" {
                self.pos += 1;
            }
            return val;
        }
        self.pos += 1;
        let name = if tok.starts_with('$') {
            let inner = &tok[1..];
            if inner.starts_with('{') && inner.ends_with('}') {
                &inner[1..inner.len() - 1]
            } else {
                inner
            }
        } else {
            tok.as_str()
        };
        if let Ok(n) = name.parse::<i64>() {
            return n;
        }
        let val = self.rt.lookup_var(name);
        val.parse::<i64>().unwrap_or(0)
    }
}

impl Runtime {
    fn eval_arithmetic(&self, expr: &str) -> i64 {
        let tokens = tokenize_arithmetic(expr);
        let mut parser = ArithParser {
            tokens: &tokens,
            pos: 0,
            rt: self,
        };
        let result = parser.parse_expr();
        result
    }
}

fn substr(s: &str, offset: i64, len: Option<usize>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let start = if offset >= 0 {
        (offset as usize).min(n)
    } else {
        n.saturating_sub((-offset) as usize)
    };
    if let Some(l) = len {
        chars[start..].iter().take(l).collect()
    } else {
        chars[start..].iter().collect()
    }
}

fn glob_remove_prefix(val: &str, pat: &str, longest: bool) -> Option<String> {
    let chars: Vec<char> = val.chars().collect();
    let n = chars.len();
    let mut best = None;
    for i in 0..=n {
        let prefix: String = chars[..i].iter().collect();
        if simple_glob_match(&prefix, pat) {
            let result: String = chars[i..].iter().collect();
            if longest { best = Some(result); } else { return Some(result); }
        }
    }
    best
}

fn glob_remove_suffix(val: &str, pat: &str, longest: bool) -> Option<String> {
    let chars: Vec<char> = val.chars().collect();
    let n = chars.len();
    let mut best = None;
    for i in (0..=n).rev() {
        let suffix: String = chars[i..].iter().collect();
        if simple_glob_match(&suffix, pat) {
            let result: String = chars[..i].iter().collect();
            if longest { best = Some(result); } else { return Some(result); }
        }
    }
    best
}

fn simple_glob_match(s: &str, pat: &str) -> bool {
    if !pat.contains('*') && !pat.contains('?') { return s == pat; }
    let sc: Vec<char> = s.chars().collect();
    let pc: Vec<char> = pat.chars().collect();
    let (n, m) = (sc.len(), pc.len());
    let mut dp = vec![false; m + 1];
    dp[0] = true;
    for j in 0..m { if pc[j] == '*' { dp[j + 1] = dp[j]; } else { break; } }
    for i in 0..n {
        let mut prev = dp[0];
        dp[0] = false;
        for j in 0..m {
            let old = dp[j + 1];
            dp[j + 1] = match pc[j] {
                '*' => dp[j] || old,
                '?' => prev,
                c => sc[i] == c && prev,
            };
            prev = old;
        }
    }
    dp[m]
}

fn split_unescaped_slash(s: &str) -> (String, String) {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' { i += 2; continue; }
        if chars[i] == '/' {
            let pat: String = chars[..i].iter().collect();
            let rep: String = chars[i + 1..].iter().collect();
            return (pat, rep);
        }
        i += 1;
    }
    (s.to_string(), String::new())
}

impl Runtime {
    /// Extracts redirect operators from parsed tokens and returns the
    /// (cleaned_args, redirect_spec). Handles: > >> < 2> 2>> 1> 1>> 2>&1 >& &>
    fn extract_redirects(&self, parts: Vec<String>) -> (Vec<String>, RedirectSpec) {
        if parts.is_empty() {
            return (parts, RedirectSpec::default());
        }
        // Build args from all tokens (including potential first redirect op)
        let args: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
        let (clean, spec) = parse_redirects(&args);
        (clean, spec)
    }

    /// Applies redirect_spec to a CommandOutput: writes stdout/stderr to files.
    /// When stdout and stderr target the same file (>& / &>), they are merged
    /// before writing to avoid the second write truncating the first.
    fn apply_redirects(
        &self,
        result: &mut CommandOutput,
        spec: &RedirectSpec,
    ) {
        // Merge stderr into stdout if requested (2>&1)
        if spec.merge_stderr_to_stdout && !result.stderr.is_empty() {
            if !result.stdout.is_empty() && !result.stdout.ends_with('\n') {
                result.stdout.push('\n');
            }
            result.stdout.push_str(&result.stderr);
            result.stderr.clear();
        }

        // Merge stdout into stderr if requested (1>&2)
        if spec.merge_stdout_to_stderr && !result.stdout.is_empty() {
            if !result.stderr.is_empty() && !result.stderr.ends_with('\n') {
                result.stderr.push('\n');
            }
            result.stderr.push_str(&result.stdout);
            result.stdout.clear();
        }

        // When both stdout and stderr target the same file (>& / &>), write
        // them together to avoid truncation.
        let same_file = match (&spec.stdout_file, &spec.stderr_file) {
            (Some((a, _)), Some((b, _))) => a == b,
            _ => false,
        };

        if same_file {
            if let Some((ref path, append)) = &spec.stdout_file {
                let mut content = if *append {
                    self.shell.vfs.read_to_string(path, &self.shell.cwd).unwrap_or_default()
                } else {
                    String::new()
                };
                if !result.stdout.is_empty() {
                    content.push_str(&result.stdout);
                }
                if !result.stderr.is_empty() {
                    if !content.is_empty() && !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(&result.stderr);
                }
                if let Err(e) = self.shell.vfs.write(path, &self.shell.cwd, &content) {
                    result.stderr = format!("redirect: {}: {}\n", path, e);
                    result.exit_code = 1;
                }
                result.stdout = String::new();
                result.stderr = String::new();
            }
            return;
        }

        // Write stdout to file
        if let Some((ref path, append)) = &spec.stdout_file {
            let final_stdout = if *append {
                let existing = self.shell.vfs.read_to_string(path, &self.shell.cwd).unwrap_or_default();
                existing + &result.stdout
            } else {
                result.stdout.clone()
            };
            if let Err(e) = self.shell.vfs.write(path, &self.shell.cwd, &final_stdout) {
                result.stderr = format!("redirect: {}: {}\n", path, e);
                result.exit_code = 1;
            }
            result.stdout = String::new();
        }

        // Write stderr to file
        if let Some((ref path, append)) = &spec.stderr_file {
            let final_stderr = if *append {
                let existing = self.shell.vfs.read_to_string(path, &self.shell.cwd).unwrap_or_default();
                existing + &result.stderr
            } else {
                result.stderr.clone()
            };
            if let Err(e) = self.shell.vfs.write(path, &self.shell.cwd, &final_stderr) {
                if result.stderr.is_empty() {
                    result.stderr = format!("redirect: {}: {}\n", path, e);
                } else {
                    result.stderr.push_str(&format!("redirect: {}: {}\n", path, e));
                }
                result.exit_code = 1;
            }
            result.stderr = String::new();
        }
    }

    fn expand_globs(&self, tokens: Vec<ParsedToken>) -> Vec<String> {
        let mut expanded = Vec::new();
        for (i, token) in tokens.into_iter().enumerate() {
            if i == 0 || token.quoted || !has_glob_chars(&token.value) {
                expanded.push(token.value);
                continue;
            }
            let matches = self.try_glob(&token.value);
            if matches.is_empty() {
                expanded.push(token.value);
            } else {
                expanded.extend(matches);
            }
        }
        expanded
    }

    fn execute_pipeline(&mut self, input: &str, init_stdin: Option<&str>) -> CommandOutput {
        // (c) 2025 xiefujin <490021684@qq.com>
        let stages = parse_pipeline(input);
        if stages.is_empty() {
            return CommandOutput::success(String::new());
        }

        let mut expanded_stages: Vec<Vec<ParsedToken>> = Vec::new();
        for mut stage in stages {
            let expanded = self.expand_globs(std::mem::take(&mut stage));
            expanded_stages.push(
                expanded
                    .into_iter()
                    .map(|s| ParsedToken::new(s, false))
                    .collect(),
            );
        }

        // Extract redirects from the last stage
        let last_idx = expanded_stages.len() - 1;
        let (pipeline_spec, clean_last) = if !expanded_stages.is_empty() {
            let last_flat: Vec<String> = expanded_stages[last_idx]
                .iter()
                .map(|t| t.value.clone())
                .collect();
            let (clean, spec) = self.extract_redirects(last_flat);
            if !clean.is_empty() {
                let clean_tokens: Vec<ParsedToken> = clean
                    .into_iter()
                    .map(|s| ParsedToken::new(s, false))
                    .collect();
                expanded_stages[last_idx] = clean_tokens;
            }
            (spec, expanded_stages[last_idx].clone())
        } else {
            (RedirectSpec::default(), vec![])
        };

        if expanded_stages.len() == 1 {
            if clean_last.is_empty() {
                let mut result = CommandOutput::success(String::new());
                self.apply_redirects(&mut result, &pipeline_spec);
                return result;
            }
            let cmd = &clean_last[0].value;
            let args: Vec<&str> = clean_last[1..].iter().map(|t| t.value.as_str()).collect();
            let stdin_from_file = if init_stdin.is_none() {
                pipeline_spec.stdin_file.as_ref().and_then(|path| {
                    self.shell.vfs.read_to_string(path, &self.shell.cwd).ok()
                })
            } else {
                None
            };
            let stdin = init_stdin.or(stdin_from_file.as_deref());
            let mut result = self.shell.execute(cmd, &args, stdin);
            self.apply_redirects(&mut result, &pipeline_spec);
            return result;
        }

        let saved_cwd = self.shell.cwd.clone();

        // stdin for the first stage: heredoc body or `< file`.
        let first_stdin: Option<String> = match init_stdin {
            Some(s) => Some(s.to_string()),
            None => pipeline_spec.stdin_file.as_ref().and_then(|path| {
                self.shell.vfs.read_to_string(path, &self.shell.cwd).ok()
            }),
        };

        let mut threads: Vec<std::thread::JoinHandle<std::thread::Result<(i32, String, String)>>> =
            Vec::new();
        let mut prev_rx: Option<mpsc::Receiver<Vec<u8>>> = None;

        for (i, stage) in expanded_stages.into_iter().enumerate() {
            if stage.is_empty() {
                continue;
            }
            let cmd = stage[0].value.clone();
            let args: Vec<String> = stage[1..].iter().map(|t| t.value.clone()).collect();
            let mut shell = self.shell.clone();
            let rx = prev_rx.take();
            let is_last = i == last_idx;
            let init = if i == 0 { first_stdin.clone() } else { None };

            let (tx, next_rx) = if !is_last {
                let (t, r) = mpsc::channel::<Vec<u8>>();
                (Some(t), Some(r))
            } else {
                (None, None)
            };

            let handle =
                std::thread::spawn(move || -> std::thread::Result<(i32, String, String)> {
                    // Pipeline stdin is received as Vec<u8> chunks.
                    // Shell commands currently accept stdin as &str (text-oriented design).
                    // Non-UTF-8 bytes are replaced with U+FFFD at the pipe boundary.
                    // Binary pipelines (e.g. `gzip -c | base64`) are not yet fully supported;
                    // use file redirection for binary workflows instead.
                    let mut stdin_buf = String::new();
                    let mut got_stdin = false;
                    if let Some(init) = init {
                        got_stdin = true;
                        stdin_buf.push_str(&init);
                    }
                    if let Some(rx) = rx {
                        while let Ok(chunk) = rx.recv() {
                            got_stdin = true;
                            stdin_buf.push_str(&String::from_utf8_lossy(&chunk));
                        }
                    }
                    let stdin = if got_stdin { Some(stdin_buf) } else { None };
                    let stdin_ref = stdin.as_deref();
                    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

                    // Alias resolution (pipeline thread has cloned Shell with aliases)
                    let (resolved_cmd, resolved_args) = if let Some((new_cmd, new_args)) = shell.resolve_alias(&cmd, &args_refs) {
                        (new_cmd, new_args)
                    } else {
                        (cmd.clone(), args_refs.iter().map(|s| s.to_string()).collect())
                    };
                    let resolved_refs: Vec<&str> = resolved_args.iter().map(|s| s.as_str()).collect();

                    let result = shell.execute(&resolved_cmd, &resolved_refs, stdin_ref);

                    if let Some(tx) = tx {
                        let _ = tx.send(result.stdout.as_bytes().to_vec());
                    }

                    Ok((result.exit_code, result.stdout, result.stderr))
                });

            threads.push(handle);
            prev_rx = next_rx;
        }

        let mut all_stderr = String::new();
        let mut final_exit_code = 0;
        let mut final_stdout = String::new();
        let mut panicked = false;

        let pipefail = self.shell.pipefail;

        for (i, handle) in threads.into_iter().enumerate() {
            match handle.join() {
                Ok(Ok((code, out, err))) => {
                    if !err.is_empty() {
                        if !all_stderr.is_empty() {
                            all_stderr.push('\n');
                        }
                        all_stderr.push_str(&err);
                    }
                    if pipefail {
                        if code != 0 {
                            final_exit_code = code;
                        } else if final_exit_code == 0 {
                            final_exit_code = code;
                        }
                    } else if i == last_idx {
                        final_exit_code = code;
                    }
                    if i == last_idx {
                        final_stdout = out;
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    panicked = true;
                }
            }
        }

        self.shell.cwd = saved_cwd;

        if panicked {
            return CommandOutput::error(format!("pipeline: thread panicked\n{}", all_stderr), 1);
        }

        let mut result = CommandOutput {
            stdout: final_stdout,
            stderr: all_stderr,
            exit_code: final_exit_code,
        };
        self.apply_redirects(&mut result, &pipeline_spec);
        result
    }

    fn try_glob(&self, pattern: &str) -> Vec<String> {
        if pattern.split('/').any(|seg| seg == "**") {
            return self.expand_recursive_glob(pattern);
        }

        let cwd_path = if self.shell.cwd == "/" {
            self.shell_root_dir()
        } else {
            self.shell_root_dir()
                .join(self.shell.cwd.trim_start_matches('/'))
        };
        let glob_pattern = cwd_path.join(pattern).to_string_lossy().to_string();
        match glob::glob(&glob_pattern) {
            Ok(paths) => paths
                .filter_map(|entry| entry.ok())
                .filter_map(|p| {
                    p.strip_prefix(&cwd_path)
                        .ok()
                        .map(|r| r.to_string_lossy().to_string())
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    fn expand_recursive_glob(&self, pattern: &str) -> Vec<String> {
        let cwd = &self.shell.cwd;

        let segments: Vec<&str> = pattern.split('/').collect();
        let globstar_idx = match segments.iter().position(|&s| s == "**") {
            Some(p) => p,
            None => return vec![pattern.to_string()],
        };

        let base_segments = &segments[..globstar_idx];
        let suffix_segments = &segments[globstar_idx + 1..];

        let base_path = base_segments.join("/");
        let suffix = suffix_segments.join("/");

        let vfs_base = if base_path.is_empty() {
            cwd.clone()
        } else if base_path.starts_with('/') {
            base_path.clone()
        } else if cwd == "/" {
            format!("/{}", base_path)
        } else {
            format!("{}/{}", cwd, base_path)
        };

        let vfs_base = normalize_vpath(&vfs_base);

        if !vfs_base.starts_with('/') || !self.shell.vfs.is_dir(&vfs_base, "") {
            return vec![];
        }

        let mut results = Vec::new();
        let mut dirs_to_visit: Vec<(String, usize)> = vec![(vfs_base.clone(), 0)];
        let max_depth = 32;
        let vfs_base_clean = vfs_base.trim_end_matches('/');

        while let Some((dir, depth)) = dirs_to_visit.pop() {
            if depth > max_depth {
                continue;
            }

            let entries = match self.shell.vfs.list_dir(&dir, "") {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in &entries {
                let child_path = if dir == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", dir, entry.name)
                };

                let rel_path = if child_path == vfs_base_clean {
                    String::new()
                } else {
                    match child_path.strip_prefix(vfs_base_clean) {
                        Some(p) => p.trim_start_matches('/').to_string(),
                        None => continue,
                    }
                };

                if entry.is_dir {
                    dirs_to_visit.push((child_path.clone(), depth + 1));
                }

                let matched = if suffix.is_empty() {
                    true
                } else {
                    path_matches_suffix(&rel_path, &suffix)
                };

                if matched {
                    let result_path = vfs_path_to_relative(&child_path, cwd);
                    if !result_path.is_empty() {
                        results.push(result_path);
                    }
                }
            }
        }

        results.sort();
        results
    }

    pub fn python_available(&self) -> bool {
        self.python.as_ref().map_or(false, |p| p.is_available())
    }

    pub fn python_engine_ref(&self) -> Option<&(dyn PythonEngine + '_)> {
        self.python.as_ref().map(|p| p.as_ref())
    }

    fn execute_python_inner(&mut self, input: &str) -> CommandOutput {
        // Route based on the kind of python invocation.
        match classify_python(input) {
            PyInvocation::Code(code) => {
                let cwd = self.vfs_cwd();
                let self_ptr: *mut Runtime = self;
                let python = match &mut self.python {
                    Some(p) => p,
                    None => {
                        return CommandOutput::error(
                            "Python engine not configured".to_string(),
                            127,
                        )
                    }
                };
                if !python.is_available() {
                    return CommandOutput::error("Python is not available".to_string(), 127);
                }
                REENTRANT_RT.with(|c| c.set(self_ptr));
                let _guard = ReentrantGuard;
                let result = python.execute(&code, &cwd);
                CommandOutput {
                    stdout: result.stdout,
                    stderr: result.stderr,
                    exit_code: result.exit_code,
                }
            }
            PyInvocation::Script(path, args) => {
                // Run a .py file. Extra args are exposed via sys.argv by wrapping
                // in a small bootstrap when args are present.
                if args.is_empty() {
                    self.execute_python_script(&path)
                } else {
                    let cwd = self.vfs_cwd();
                    // VFS-resolve so full physical paths aren't double-joined.
                    let full_path = self
                        .shell
                        .vfs
                        .resolve(&path, &self.shell.cwd)
                        .unwrap_or_else(|_| cwd.join(path.trim_start_matches('/')));
                    let argv_list = std::iter::once(path.clone())
                        .chain(args.iter().cloned())
                        .map(|a| format!("{:?}", a))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let code = format!(
                        "import sys, runpy\nsys.argv = [{argv}]\nrunpy.run_path({path:?}, run_name='__main__')",
                        argv = argv_list,
                        path = full_path.to_string_lossy(),
                    );
                    self.execute_python_code(&code)
                }
            }
            PyInvocation::Module(module, args) => {
                // python -m <module> [args]  → runpy.run_module
                let argv_list = std::iter::once(format!("-m {module}"))
                    .chain(args.iter().cloned())
                    .map(|a| format!("{:?}", a))
                    .collect::<Vec<_>>()
                    .join(", ");
                let code = format!(
                    "import sys, runpy\nsys.argv = [{argv}]\nrunpy.run_module({module:?}, run_name='__main__', alter_sys=True)",
                    argv = argv_list,
                    module = module,
                );
                self.execute_python_code(&code)
            }
            PyInvocation::Repl => CommandOutput::error(
                "interactive python REPL is not supported; use `python -c` or a script".to_string(),
                1,
            ),
        }
    }

    pub fn execute_python_code(&mut self, code: &str) -> CommandOutput {
        let cwd = self.vfs_cwd();
        let self_ptr: *mut Runtime = self;
        // (c) 2025 xiefujin <490021684@qq.com>
        let python = match &mut self.python {
            Some(p) => p,
            None => {
                return CommandOutput::error("Python engine not configured".to_string(), 127);
            }
        };

        if !python.is_available() {
            return CommandOutput::error("Python is not available".to_string(), 127);
        }

        REENTRANT_RT.with(|c| c.set(self_ptr));
        let _guard = ReentrantGuard;
        let result = python.execute(code, &cwd);

        CommandOutput {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        }
    }

    pub fn execute_python_script(&mut self, script_path: &str) -> CommandOutput {
        // Resolve through the VFS so both sandbox-relative paths and full
        // physical paths (e.g. from aacode-rs's execute_python tool) work.
        let full_path = self
            .shell
            .vfs
            .resolve(script_path, &self.shell.cwd)
            .unwrap_or_else(|_| {
                self.shell_root_dir()
                    .join(script_path.trim_start_matches('/'))
            });
        let cwd = self.vfs_cwd();
        let self_ptr: *mut Runtime = self;
        let python = match &mut self.python {
            Some(p) => p,
            None => {
                return CommandOutput::error("Python engine not configured".to_string(), 127);
            }
        };
        if !python.is_available() {
            return CommandOutput::error("Python is not available".to_string(), 127);
        }
        REENTRANT_RT.with(|c| c.set(self_ptr));
        let _guard = ReentrantGuard;
        let result = python.execute_script(&full_path, &cwd);

        CommandOutput {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        }
    }

    pub fn shell_root_dir(&self) -> std::path::PathBuf {
        self.shell.vfs.root().to_path_buf()
    }

    /// Returns the VFS absolute path for the current working directory.
    pub fn vfs_cwd(&self) -> std::path::PathBuf {
        let root = self.shell_root_dir();
        if self.shell.cwd == "/" {
            root
        } else {
            root.join(self.shell.cwd.trim_start_matches('/'))
        }
    }

    pub fn cwd(&self) -> &str {
        &self.shell.cwd
    }

    pub fn read_file(&self, path: &str) -> Result<String, String> {
        let cwd = self.cwd().to_string();
        self.shell
            .vfs
            .read_to_string(path, &cwd)
            .map_err(|e| format!("read_file: {}", e))
    }

    pub fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        let cwd = self.cwd().to_string();
        self.shell
            .vfs
            .write(path, &cwd, content)
            .map_err(|e| format!("write_file: {}", e))
    }

    pub fn list_dir(&self, path: &str) -> Result<Vec<crate::sdk::types::FileEntry>, String> {
        let cwd = self.cwd().to_string();
        let entries = self
            .shell
            .vfs
            .list_dir(path, &cwd)
            .map_err(|e| format!("list_dir: {}", e))?;
        Ok(entries
            .into_iter()
            .map(|de| {
                let de_name = de.name.clone();
                crate::sdk::types::FileEntry {
                    name: de.name,
                    path: if path.ends_with('/') {
                        format!("{}{}", path, de_name)
                    } else {
                        format!("{}/{}", path, de_name)
                    },
                    is_dir: de.is_dir,
                    size: de.size,
                }
            })
            .collect())
    }

    pub fn exists(&self, path: &str) -> bool {
        let cwd = self.cwd().to_string();
        self.shell.vfs.exists(path, &cwd)
    }

    pub fn is_dir(&self, path: &str) -> bool {
        let cwd = self.cwd().to_string();
        self.shell.vfs.is_dir(path, &cwd)
    }

    pub fn quick_execute(&mut self, command: &str) -> CommandOutput {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return CommandOutput::success(String::new());
        }
        let cmd = parts[0];
        let args = &parts[1..];
        self.shell.execute(cmd, args, None)
    }
}

fn is_python_command(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.starts_with("python3 ") || trimmed.starts_with("python ") {
        return true;
    }
    if trimmed == "python" || trimmed == "python3" {
        return true;
    }
    // Bare pytest is rewritten to `python -m pytest`.
    if trimmed == "pytest" || trimmed.starts_with("pytest ") {
        return true;
    }
    false
}

/// The kind of python invocation extracted from a command line.
#[derive(Debug, PartialEq)]
enum PyInvocation {
    /// `python -c "code"`
    Code(String),
    /// `python script.py [args...]`
    Script(String, Vec<String>),
    /// `python -m module [args...]`  (also bare `pytest ...`)
    Module(String, Vec<String>),
    /// bare `python` / `python3` REPL
    Repl,
}

/// Split a command into whitespace-separated tokens, honoring simple quoting.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in input.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    cur.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Classify a python command line into a `PyInvocation`.
fn classify_python(input: &str) -> PyInvocation {
    let trimmed = input.trim();

    // Bare pytest → python -m pytest.
    if trimmed == "pytest" {
        return PyInvocation::Module("pytest".to_string(), vec![]);
    }
    if let Some(rest) = trimmed.strip_prefix("pytest ") {
        let args = tokenize(rest);
        return PyInvocation::Module("pytest".to_string(), args);
    }

    // -c inline code (preserve original quote-stripping behavior).
    if trimmed.starts_with("python3 -c") || trimmed.starts_with("python -c") {
        return PyInvocation::Code(extract_python_code(trimmed));
    }

    let tokens = tokenize(trimmed);
    // tokens[0] is python/python3
    if tokens.len() <= 1 {
        return PyInvocation::Repl;
    }
    let rest = &tokens[1..];

    // -m module [args]
    if rest[0] == "-m" {
        if rest.len() >= 2 {
            let module = rest[1].clone();
            let args = rest[2..].to_vec();
            return PyInvocation::Module(module, args);
        }
        return PyInvocation::Repl;
    }

    // Skip leading interpreter flags we don't model (e.g. -u, -B); find the
    // first non-flag token as the script path.
    let mut idx = 0;
    while idx < rest.len() && rest[idx].starts_with('-') {
        idx += 1;
    }
    if idx < rest.len() {
        let script = rest[idx].clone();
        let args = rest[idx + 1..].to_vec();
        return PyInvocation::Script(script, args);
    }

    PyInvocation::Repl
}

fn extract_python_code(input: &str) -> String {
    let trimmed = input.trim();

    if let Some(code) = trimmed.strip_prefix("python3 -c ") {
        return strip_quotes(code);
    }
    if let Some(code) = trimmed.strip_prefix("python -c ") {
        return strip_quotes(code);
    }
    if let Some(code) = trimmed.strip_prefix("python3 -c") {
        return strip_quotes(code.trim());
    }
    if let Some(code) = trimmed.strip_prefix("python -c") {
        return strip_quotes(code.trim());
    }

    trimmed.to_string()
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let first = s.chars().next().unwrap();
        let last = s.chars().last().unwrap();
        if (first == '\'' && last == '\'') || (first == '"' && last == '"') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedToken {
    value: String,
    quoted: bool,
}

impl ParsedToken {
    fn new(value: String, quoted: bool) -> Self {
        ParsedToken { value, quoted }
    }
}

// ═══════════════════════════════════════════════════════════
// Logical command segmentation — newline / `;` / `&&` / `||`
// separators plus heredoc (`<< TAG`, `<<- TAG`) collection.
// ═══════════════════════════════════════════════════════════

/// How a segment chains onto the previous one.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Connector {
    /// `;` or newline — always run.
    Always,
    /// `&&` — run only if the previous segment succeeded.
    AndIf,
    /// `||` — run only if the previous segment failed.
    OrIf,
}

#[derive(Debug, Clone)]
struct Segment {
    connector: Connector,
    text: String,
    /// Collected here-document body (becomes the command's stdin).
    heredoc: Option<String>,
    /// Whether the heredoc delimiter was unquoted (should expand variables).
    heredoc_expand: bool,
}

/// Splits a (possibly multi-line) input into logical segments, honoring
/// single/double quotes. Heredoc bodies are collected verbatim and never
/// treated as commands.
#[allow(unused_assignments)]
fn split_segments(input: &str) -> Vec<Segment> {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut segments: Vec<Segment> = Vec::new();
    let mut current = String::new();
    let mut heredoc: Option<String> = None;
    let mut heredoc_expand_flag: bool = false;
    let mut pending_quoted: bool = false;
    let mut pending_tag: Option<(String, bool)> = None; // (tag, strip_tabs)
    let mut connector = Connector::Always;
    let mut next_connector = Connector::Always;
    let mut in_single = false;
    let mut in_double = false;
    let mut brace_depth = 0usize;
    let mut i = 0;

    macro_rules! flush {
        () => {
            if !current.trim().is_empty() || heredoc.is_some() {
                segments.push(Segment {
                    connector,
                    text: current.trim().to_string(),
                    heredoc: heredoc.take(),
                    heredoc_expand: heredoc_expand_flag,
                });
                heredoc_expand_flag = false;
            }
            current.clear();
            connector = next_connector;
            next_connector = Connector::Always;
        };
    }

    while i < n {
        let c = chars[i];
        if in_single {
            current.push(c);
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            current.push(c);
            if c == '"' {
                in_double = false;
            } else if c == '\\' && i + 1 < n {
                current.push(chars[i + 1]);
                i += 1;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                current.push(c);
                i += 1;
            }
            '"' => {
                in_double = true;
                current.push(c);
                i += 1;
            }
            '\\' if i + 1 < n => {
                // Line continuation: backslash-newline joins lines.
                if chars[i + 1] == '\n' {
                    current.push(' ');
                } else {
                    current.push(c);
                    current.push(chars[i + 1]);
                }
                i += 2;
            }
            '\n' => {
                if let Some((tag, strip_tabs)) = pending_tag.take() {
                    // Collect heredoc body lines until the terminator.
                    let mut body = String::new();
                    let mut j = i + 1;
                    loop {
                        // Read one line [j, line_end)
                        let mut line_end = j;
                        while line_end < n && chars[line_end] != '\n' {
                            line_end += 1;
                        }
                        let line: String = chars[j..line_end].iter().collect();
                        let probe = if strip_tabs {
                            line.trim_start_matches('\t').to_string()
                        } else {
                            line.clone()
                        };
                        if probe.trim_end_matches('\r') == tag {
                            j = if line_end < n { line_end + 1 } else { n };
                            break;
                        }
                        if strip_tabs {
                            body.push_str(line.trim_start_matches('\t'));
                        } else {
                            body.push_str(&line);
                        }
                        body.push('\n');
                        if line_end >= n {
                            j = n;
                            break;
                        }
                        j = line_end + 1;
                    }
                    heredoc = Some(body);
                    heredoc_expand_flag = !pending_quoted;
                    i = j;
                    flush!();
                } else {
                    if brace_depth == 0 {
                        flush!();
                    } else {
                        current.push(c);
                    }
                    i += 1;
                }
            }
            ';' => {
                if brace_depth == 0 {
                    flush!();
                } else {
                    current.push(c);
                }
                i += 1;
            }
            '&' => {
                if i + 1 < n && chars[i + 1] == '&' && brace_depth == 0 {
                    next_connector = Connector::AndIf;
                    flush!();
                    i += 2;
                } else if i + 1 < n && chars[i + 1] == '>' {
                    // `&>` redirect operator — keep literally.
                    current.push('&');
                    current.push('>');
                    i += 2;
                } else if i > 0 && chars[i - 1] == '>' {
                    // part of `2>&1` / `>&` — keep literally.
                    current.push(c);
                    i += 1;
                } else {
                    // Trailing background `&` — run synchronously.
                    flush!();
                    i += 1;
                }
            }
            '|' => {
                if i + 1 < n && chars[i + 1] == '|' && brace_depth == 0 {
                    next_connector = Connector::OrIf;
                    flush!();
                    i += 2;
                } else {
                    current.push(c);
                    i += 1;
                }
            }
            '<' if i + 1 < n && chars[i + 1] == '<' && i + 2 < n && chars[i + 2] == '<' => {
                // Here-string: <<< ["]text["]
                let mut j = i + 3;
                while j < n && (chars[j] == ' ' || chars[j] == '\t') {
                    j += 1;
                }
                let mut text = String::new();
                if j < n && chars[j] == '"' {
                    let mut k = j + 1;
                    while k < n && chars[k] != '"' {
                        text.push(chars[k]);
                        k += 1;
                    }
                    j = k + 1;
                } else {
                    while j < n && chars[j] != '\n' {
                        text.push(chars[j]);
                        j += 1;
                    }
                }
                heredoc = Some(text);
                heredoc_expand_flag = true;
                i = j;
            }
            '<' if i + 1 < n && chars[i + 1] == '<' => {
                // Heredoc operator: <<[-] [quoted]TAG
                let mut j = i + 2;
                let mut strip_tabs = false;
                if j < n && chars[j] == '-' {
                    strip_tabs = true;
                    j += 1;
                }
                while j < n && (chars[j] == ' ' || chars[j] == '\t') {
                    j += 1;
                }
                let mut tag = String::new();
                let mut quoted = false;
                if j < n && (chars[j] == '\'' || chars[j] == '"') {
                    quoted = true;
                    let q = chars[j];
                    j += 1;
                    while j < n && chars[j] != q {
                        tag.push(chars[j]);
                        j += 1;
                    }
                    j += 1; // closing quote
                } else {
                    while j < n && !chars[j].is_whitespace() {
                        tag.push(chars[j]);
                        j += 1;
                    }
                }
                if tag.is_empty() {
                    // Not a valid heredoc; keep literally.
                    current.push('<');
                    current.push('<');
                    i += 2;
                } else {
                    pending_tag = Some((tag, strip_tabs));
                    pending_quoted = quoted;
                    i = j;
                }
            }
            '{' => {
                brace_depth += 1;
                current.push(c);
                i += 1;
            }
            '}' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                }
                current.push(c);
                i += 1;
            }
            _ => {
                current.push(c);
                i += 1;
            }
        }
    }

    // Input ended: if a heredoc tag is still pending with no newline after it,
    // the body is empty.
    if pending_tag.take().is_some() && heredoc.is_none() {
        heredoc = Some(String::new());
    }
    flush!();

    segments
}

/// Attempts to parse a leading `NAME=value` assignment from `input`.
/// Returns (name, unquoted_value, rest_after_assignment).
fn take_assignment(input: &str) -> Option<(String, String, &str)> {
    let s = input.trim_start();
    let mut name_end = 0;
    for (idx, c) in s.char_indices() {
        if idx == 0 {
            if !(c.is_ascii_alphabetic() || c == '_') {
                return None;
            }
            name_end = c.len_utf8();
            continue;
        }
        if c == '=' {
            name_end = idx;
            break;
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        name_end = idx + c.len_utf8();
    }
    if name_end == 0 || !s[name_end..].starts_with('=') {
        return None;
    }
    let name = s[..name_end].to_string();
    let after_eq = &s[name_end + 1..];

    // Value: quoted string or bare word.
    let mut value = String::new();
    let mut rest_idx = after_eq.len();
    let mut chars = after_eq.char_indices().peekable();
    if let Some(&(_, first)) = chars.peek() {
        if first == '\'' || first == '"' {
            let q = first;
            chars.next();
            let mut closed = false;
            for (idx, c) in chars.by_ref() {
                if c == q {
                    rest_idx = idx + c.len_utf8();
                    closed = true;
                    break;
                }
                value.push(c);
            }
            if !closed {
                rest_idx = after_eq.len();
            }
        } else {
            for (idx, c) in chars {
                if c.is_whitespace() {
                    rest_idx = idx;
                    break;
                }
                value.push(c);
            }
        }
    } else {
        rest_idx = 0;
    }
    Some((name, value, &after_eq[rest_idx..]))
}

// ═══════════════════════════════════════════════════════════
// Shell redirect operators — parsed from command arguments.
// Supported: > >> < 2> 2>> 1> 1>> 2>&1 >& &>
//
// All file paths are resolved through the VFS, so redirects
// respect the sandbox (file contents stay within FASTSHELL_ROOT).
// ═══════════════════════════════════════════════════════════

#[derive(Debug, Clone, Default)]
struct RedirectSpec {
    stdout_file: Option<(String, bool)>, // (path, append)
    stderr_file: Option<(String, bool)>, // (path, append)
    stdin_file: Option<String>,
    merge_stderr_to_stdout: bool,        // 2>&1
    merge_stdout_to_stderr: bool,        // 1>&2
}

/// Parses redirect operators from `args` and returns (clean_args, spec).
/// Scans right-to-left so the rightmost redirect for a given fd wins.
fn parse_redirects(args: &[&str]) -> (Vec<String>, RedirectSpec) {
    let mut spec = RedirectSpec::default();
    let mut i: isize = args.len() as isize - 1;

    while i >= 0 {
        let token = args[i as usize];
        match token {
            // -- stdout truncate: > file  (or 1> file) --
            ">" | "1>" => {
                if i + 1 < args.len() as isize && spec.stdout_file.is_none() {
                    spec.stdout_file = Some((args[(i + 1) as usize].to_string(), false));
                    i -= 2; // skip operator + filename
                    continue;
                }
            }
            // -- stdout append: >> file  (or 1>> file) --
            ">>" | "1>>" => {
                if i + 1 < args.len() as isize && spec.stdout_file.is_none() {
                    spec.stdout_file = Some((args[(i + 1) as usize].to_string(), true));
                    i -= 2;
                    continue;
                }
            }
            // -- stderr truncate: 2> file --
            "2>" => {
                if i + 1 < args.len() as isize && spec.stderr_file.is_none() {
                    spec.stderr_file = Some((args[(i + 1) as usize].to_string(), false));
                    i -= 2;
                    continue;
                }
            }
            // -- stderr append: 2>> file --
            "2>>" => {
                if i + 1 < args.len() as isize && spec.stderr_file.is_none() {
                    spec.stderr_file = Some((args[(i + 1) as usize].to_string(), true));
                    i -= 2;
                    continue;
                }
            }
            // -- stdin from file: < file --
            "<" => {
                if i + 1 < args.len() as isize && spec.stdin_file.is_none() {
                    spec.stdin_file = Some(args[(i + 1) as usize].to_string());
                    i -= 2;
                    continue;
                }
            }
            "<>" => {
                if i + 1 < args.len() as isize && spec.stdin_file.is_none() {
                    spec.stdin_file = Some(args[(i + 1) as usize].to_string());
                    i -= 2;
                    continue;
                }
            }
            // -- merge stderr→stdout: 2>&1 --
            "2>&1" => {
                spec.merge_stderr_to_stdout = true;
                i -= 1;
                continue;
            }
            // -- merge stdout→stderr: 1>&2 --
            "1>&2" => {
                spec.merge_stdout_to_stderr = true;
                i -= 1;
                continue;
            }
            // -- redirect both: >& file  or  &> file --
            ">&" | "&>" => {
                if i + 1 < args.len() as isize
                    && spec.stdout_file.is_none()
                    && spec.stderr_file.is_none()
                {
                    let path = args[(i + 1) as usize].to_string();
                    spec.stdout_file = Some((path.clone(), false));
                    spec.stderr_file = Some((path, false));
                    i -= 2;
                    continue;
                }
            }
            "&>>" => {
                if i + 1 < args.len() as isize
                    && spec.stdout_file.is_none()
                    && spec.stderr_file.is_none()
                {
                    let path = args[(i + 1) as usize].to_string();
                    spec.stdout_file = Some((path.clone(), true));
                    spec.stderr_file = Some((path, true));
                    i -= 2;
                    continue;
                }
            }
            _ => {}
        }
        i -= 1;
    }

    // Build clean arg list: all args except the redirect operators and their file targets
    let mut clean = Vec::new();
    let all = args.to_vec();
    i = 0;
    while i < all.len() as isize {
        let t = all[i as usize];
        let is_op = matches!(
            t,
            ">" | "1>" | ">>" | "1>>" | "2>" | "2>>" | "<" | "<>" | "&>>" | ">&" | "&>" | "2>&1" | "1>&2"
        );
        if is_op {
            // Skip the operator
            i += 1;
            // Skip the filename for operators that consume the next token
            if matches!(t, ">" | "1>" | ">>" | "1>>" | "2>" | "2>>" | "<" | "<>" | "&>>" | ">&" | "&>") {
                i += 1;
            }
            // 2>&1 / 1>&2 don't consume a filename
        } else {
            clean.push(all[i as usize].to_string());
            i += 1;
        }
    }

    (clean, spec)
}

fn parse_command(input: &str) -> Vec<ParsedToken> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut quoted = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            } else {
                current.push(ch);
            }
        } else if in_double_quote {
            if ch == '"' {
                in_double_quote = false;
            } else if ch == '\\' && chars.peek().map_or(false, |&n| n == '"' || n == '\\' || n == '$' || n == '`' || n == '!') {
                current.push(chars.next().unwrap());
            } else {
                current.push(ch);
            }
        } else if ch == '\'' {
            in_single_quote = true;
            quoted = true;
        } else if ch == '"' {
            in_double_quote = true;
            quoted = true;
        } else if ch == ' ' || ch == '\t' {
            if !current.is_empty() {
                parts.push(ParsedToken::new(current.clone(), quoted));
                current.clear();
                quoted = false;
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        parts.push(ParsedToken::new(current, quoted));
    }

    parts
}

fn parse_pipeline(input: &str) -> Vec<Vec<ParsedToken>> {
    let mut stages: Vec<Vec<ParsedToken>> = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_single_quote {
            current.push(ch);
            if ch == '\'' {
                in_single_quote = false;
            }
        } else if in_double_quote {
            current.push(ch);
            if ch == '"' {
                in_double_quote = false;
            } else if ch == '\\' && chars.peek().map_or(false, |&n| n == '"' || n == '\\' || n == '$' || n == '`' || n == '!') {
                current.push(chars.next().unwrap());
            }
        } else if ch == '\'' {
            current.push(ch);
            in_single_quote = true;
        } else if ch == '"' {
            current.push(ch);
            in_double_quote = true;
        } else if ch == '|' {
            let merge_stderr = chars.peek().map_or(false, |&c| c == '&');
            if merge_stderr {
                chars.next();
                current.push_str(" 2>&1");
            }
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                stages.push(parse_command(&trimmed));
            }
            current.clear();
        } else {
            current.push(ch);
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        stages.push(parse_command(&trimmed));
    }

    stages
}

/// Brace expansion: `{a,b,c}` → `a b c`, `{1..5}` → `1 2 3 4 5`.
fn expand_braces(input: &str) -> String {
    if !input.contains('{') || !input.contains('}') {
        return input.to_string();
    }
    let results = brace_expand_inner(input);
    if results.is_empty() {
        input.to_string()
    } else {
        results.join(" ")
    }
}

fn brace_expand_inner(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut open = None;
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '{' if !in_single && !in_double => {
                if depth == 0 { open = Some(i); }
                depth += 1;
            }
            '}' if !in_single && !in_double => {
                if depth == 0 { return vec![input.to_string()]; }
                depth -= 1;
                if depth == 0 {
                    let prefix: String = chars[..open.unwrap()].iter().collect();
                    let body: String = chars[open.unwrap() + 1..i].iter().collect();
                    let suffix: String = chars[i + 1..].iter().collect();
                    let parts = split_brace_parts(&body);
                    let mut results = Vec::new();
                    for part in &parts {
                        let expanded_part = if is_range(part) {
                            expand_range(part)
                        } else {
                            vec![part.clone()]
                        };
                        for ep in &expanded_part {
                            let combined = format!("{prefix}{ep}{suffix}");
                            results.extend(brace_expand_inner(&combined));
                        }
                    }
                    return results;
                }
            }
            _ => {}
        }
    }
    vec![input.to_string()]
}

fn split_brace_parts(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let chars: Vec<char> = body.chars().collect();
    for &c in &chars {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '{' => { depth += 1; current.push(c); }
            '}' => { if depth > 0 { depth -= 1; } current.push(c); }
            ',' if depth == 0 && !in_single && !in_double => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    parts.push(current.trim().to_string());
    parts
}

fn is_range(s: &str) -> bool {
    s.contains("..") && !s.contains(',') && !s.contains('{')
}

fn expand_range(s: &str) -> Vec<String> {
    if let Some(pos) = s.find("..") {
        let start_str = &s[..pos];
        let end_str = &s[pos + 2..];
        if let (Ok(start), Ok(end)) = (start_str.parse::<i64>(), end_str.parse::<i64>()) {
            if start <= end {
                return (start..=end).map(|n| n.to_string()).collect();
            }
        }
    }
    vec![s.to_string()]
}

fn normalize_vpath(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(part),
        }
    }
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

fn vfs_path_to_relative(vfs_path: &str, cwd: &str) -> String {
    if cwd == "/" {
        vfs_path.trim_start_matches('/').to_string()
    } else {
        let prefix = format!("{}/", cwd);
        vfs_path
            .strip_prefix(&prefix)
            .unwrap_or_else(|| vfs_path.trim_start_matches('/'))
            .to_string()
    }
}

fn path_matches_suffix(rel_path: &str, suffix: &str) -> bool {
    if suffix.is_empty() {
        return true;
    }
    let path_segments: Vec<&str> = rel_path.split('/').collect();
    let suffix_segments: Vec<&str> = suffix.split('/').collect();

    if suffix_segments.len() > path_segments.len() {
        return false;
    }
    let offset = path_segments.len() - suffix_segments.len();
    for (i, suffix_seg) in suffix_segments.iter().enumerate() {
        if !simple_glob_match(path_segments[offset + i], suffix_seg) {
            return false;
        }
    }
    true
}

fn case_pattern_match(word: &str, pattern: &str) -> bool {
    for alt in pattern.split('|') {
        let p = alt.trim();
        if p == "*" {
            return true;
        }
        if p == word {
            return true;
        }
        if has_glob_chars(p) {
            if let Ok(re) = glob_to_regex(p) {
                if re.is_match(word) {
                    return true;
                }
            }
        }
    }
    false
}

fn glob_to_regex(pattern: &str) -> Result<regex::Regex, regex::Error> {
    let mut r = String::new();
    r.push('^');
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => r.push_str(".*"),
            '?' => r.push('.'),
            '[' => {
                r.push('[');
                i += 1;
                while i < chars.len() && chars[i] != ']' {
                    r.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    r.push(']');
                }
            }
            c if ".+()^${}|\\".contains(c) => {
                r.push('\\');
                r.push(c);
            }
            c => r.push(c),
        }
        i += 1;
    }
    r.push('$');
    regex::Regex::new(&r)
}

fn shell_words_parse(input: &str) -> Result<Vec<String>, ()> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_sq = false;
    let mut in_dq = false;
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_sq {
            if c == '\'' { in_sq = false; i += 1; continue; }
            current.push(c);
        } else if in_dq {
            if c == '"' { in_dq = false; i += 1; continue; }
            if c == '\\' && i + 1 < chars.len() {
                current.push(chars[i + 1]);
                i += 1;
            } else {
                current.push(c);
            }
        } else if c == '\'' {
            in_sq = true;
        } else if c == '"' {
            in_dq = true;
        } else if c == ' ' || c == '\t' {
            if !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
        } else {
            current.push(c);
        }
        i += 1;
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

fn combine_loop_segments(segments: Vec<Segment>) -> Vec<Segment> {
    let mut result = Vec::new();
    let segments_vec = segments;
    let mut i = 0;
    while i < segments_vec.len() {
        let text = segments_vec[i].text.trim();
        if text == "for" || text.starts_with("for ")
            || text == "while" || text.starts_with("while ")
            || text == "until" || text.starts_with("until ")
        {
            let mut combined = segments_vec[i].text.clone();
            let connector = segments_vec[i].connector;
            let heredoc = segments_vec[i].heredoc.clone();
            let heredoc_expand = segments_vec[i].heredoc_expand;
            let mut depth = 1usize;
            let mut j = i + 1;
            while j < segments_vec.len() {
                let mut txt = segments_vec[j].text.trim().to_string();
                // Strip leading "do"/"{ "/" do" from the first body segment.
                if j == i + 1 {
                    if let Some(rest) = txt.strip_prefix("do ").or_else(|| txt.strip_prefix("do")) {
                        txt = rest.trim().to_string();
                    }
                }
                let txt = txt.as_str();
                if txt == "if" || txt.starts_with("if ") {
                    depth += 1;
                } else if txt == "fi" {
                    depth -= 1;
                } else if txt == "for" || txt.starts_with("for ")
                    || txt == "while" || txt.starts_with("while ")
                    || txt == "until" || txt.starts_with("until ")
                {
                    depth += 1;
                } else if txt == "done" {
                    depth -= 1;
                    if depth == 0 {
                        combined.push_str("; ");
                        combined.push_str(&segments_vec[j].text);
                        j += 1;
                        break;
                    }
                }
                combined.push_str("; ");
                combined.push_str(&segments_vec[j].text);
                j += 1;
            }
            result.push(Segment {
                connector,
                text: combined,
                heredoc,
                heredoc_expand,
            });
            i = j;
        } else {
            result.push(segments_vec[i].clone());
            i += 1;
        }
    }
    result
}

fn combine_case_segments(segments: Vec<Segment>) -> Vec<Segment> {
    let mut result = Vec::new();
    let segments_vec = segments;
    let mut i = 0;
    while i < segments_vec.len() {
        let text = segments_vec[i].text.trim();
        if text == "case" || text.starts_with("case ") {
            let mut combined = segments_vec[i].text.clone();
            let connector = segments_vec[i].connector;
            let heredoc = segments_vec[i].heredoc.clone();
            let heredoc_expand = segments_vec[i].heredoc_expand;
            let mut depth = 1usize;
            let mut j = i + 1;
            while j < segments_vec.len() {
                let txt = segments_vec[j].text.trim();
                if txt == "case" || txt.starts_with("case ") {
                    depth += 1;
                } else if txt == "esac" {
                    depth -= 1;
                    if depth == 0 {
                        combined.push_str("; ");
                        combined.push_str(&segments_vec[j].text);
                        j += 1;
                        break;
                    }
                }
                combined.push_str("; ");
                combined.push_str(&segments_vec[j].text);
                j += 1;
            }
            result.push(Segment {
                connector,
                text: combined,
                heredoc,
                heredoc_expand,
            });
            i = j;
        } else {
            result.push(segments_vec[i].clone());
            i += 1;
        }
    }
    result
}

fn has_glob_chars(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

fn parse_if_branches(input: &str) -> Vec<(String, String)> {
    let mut branches: Vec<(String, String)> = Vec::new();
    let s = input.trim();
    let mut rest = s;
    let mut is_else = false;
    while !rest.is_empty() {
        if is_else {
            if let Some(fi_pos) = rest.find("; fi") {
                branches.push((String::new(), rest[..fi_pos].trim().to_string()));
                break;
            }
            if rest == "fi" {
                branches.push((String::new(), String::new()));
                break;
            }
            branches.push((String::new(), rest.trim().to_string()));
            break;
        }
        if let Some(then_pos) = rest.find("; then ") {
            let cond = rest[..then_pos].trim().to_string();
            rest = rest[then_pos + 7..].trim();
            if let Some(elif_pos) = rest.find("; elif ") {
                let body = rest[..elif_pos].trim().to_string();
                branches.push((cond, body));
                rest = rest[elif_pos + 7..].trim();
            } else if let Some(else_pos) = rest.find("; else ") {
                let body = rest[..else_pos].trim().to_string();
                branches.push((cond, body));
                rest = rest[else_pos + 7..].trim();
                is_else = true;
            } else if let Some(fi_pos) = rest.find("; fi") {
                let body = rest[..fi_pos].trim().to_string();
                branches.push((cond, body));
                break;
            } else if rest == "fi" {
                branches.push((cond, String::new()));
                break;
            } else {
                branches.push((cond, rest.to_string()));
                break;
            }
        } else {
            break;
        }
    }
    branches
}

fn combine_if_segments(segments: Vec<Segment>) -> Vec<Segment> {
    let mut result = Vec::new();
    let segments_vec = segments;
    let mut i = 0;
    while i < segments_vec.len() {
        let text = segments_vec[i].text.trim();
        if text == "if" || text.starts_with("if ") {
            let mut combined = segments_vec[i].text.clone();
            let connector = segments_vec[i].connector;
            let heredoc = segments_vec[i].heredoc.clone();
            let heredoc_expand = segments_vec[i].heredoc_expand;
            let mut depth = 1usize;
            let mut j = i + 1;
            while j < segments_vec.len() {
                let txt = segments_vec[j].text.trim();
                if txt == "fi" {
                    depth -= 1;
                    if depth == 0 {
                        combined.push_str("; ");
                        combined.push_str(&segments_vec[j].text);
                        j += 1;
                        break;
                    }
                } else if txt == "if" || txt.starts_with("if ") {
                    depth += 1;
                }
                combined.push_str("; ");
                combined.push_str(&segments_vec[j].text);
                j += 1;
            }
            result.push(Segment {
                connector,
                text: combined,
                heredoc,
                heredoc_expand,
            });
            i = j;
        } else {
            result.push(segments_vec[i].clone());
            i += 1;
        }
    }
    result
}

fn try_parse_function_def(input: &str) -> Option<(&str, &str)> {
    let s = input.trim();
    let paren_pos = s.find("()")?;
    let name = s[..paren_pos].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return None;
    }
    let after_paren = &s[paren_pos + 2..];
    let brace_start = after_paren.find('{')?;
    let body_start = paren_pos + 2 + brace_start + 1;
    let rest: &[u8] = s[body_start..].as_bytes();
    let mut depth = 0usize;
    let mut offset = 0;
    while offset < rest.len() {
        match rest[offset] {
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    let body = &s[body_start..body_start + offset];
                    return Some((name, body));
                }
                depth -= 1;
            }
            _ => {}
        }
        offset += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::python::SubprocessPython;
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static RT_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_rt() -> Runtime {
        let n = RT_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fastshell_rt_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        let shell = Shell::new(vfs);
        Runtime::new(shell, None)
    }

    #[test]
    fn classify_python_variants() {
        assert_eq!(
            classify_python("python3 -c \"print(1)\""),
            PyInvocation::Code("print(1)".to_string())
        );
        assert_eq!(
            classify_python("python3 test.py"),
            PyInvocation::Script("test.py".to_string(), vec![])
        );
        assert_eq!(
            classify_python("python script.py a b"),
            PyInvocation::Script("script.py".to_string(), vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            classify_python("python -m pytest tests/"),
            PyInvocation::Module("pytest".to_string(), vec!["tests/".to_string()])
        );
        assert_eq!(
            classify_python("pytest"),
            PyInvocation::Module("pytest".to_string(), vec![])
        );
        assert_eq!(
            classify_python("pytest -q test_x.py"),
            PyInvocation::Module("pytest".to_string(), vec!["-q".to_string(), "test_x.py".to_string()])
        );
        assert_eq!(classify_python("python3"), PyInvocation::Repl);
        // leading interpreter flags are skipped to find the script
        assert_eq!(
            classify_python("python -u run.py"),
            PyInvocation::Script("run.py".to_string(), vec![])
        );
    }

    #[test]
    fn is_python_command_recognizes_pytest() {
        assert!(is_python_command("pytest"));
        assert!(is_python_command("pytest tests/"));
        assert!(is_python_command("python3 test.py"));
        assert!(!is_python_command("ls -la"));
    }

    #[test]
    fn tokenize_handles_quotes() {
        assert_eq!(
            tokenize("python -c \"print('hi there')\""),
            vec!["python", "-c", "print('hi there')"]
        );
    }

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_runtime() -> Runtime {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fastshell_bridge_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        let shell = Shell::new(vfs);
        let python = Box::new(SubprocessPython::new());
        Runtime::new(shell, Some(python))
    }

    #[test]
    fn test_parse_command_simple() {
        let parts = parse_command("ls -la /tmp");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["ls", "-la", "/tmp"]);
    }

    #[test]
    fn test_parse_command_quotes() {
        let parts = parse_command("echo \"hello world\"");
        assert_eq!(parts[0].value, "echo");
        assert_eq!(parts[0].quoted, false);
        assert_eq!(parts[1].value, "hello world");
        assert_eq!(parts[1].quoted, true);
    }

    #[test]
    fn test_parse_command_single_quotes() {
        let parts = parse_command("echo 'foo bar'");
        assert_eq!(parts[1].value, "foo bar");
        assert_eq!(parts[1].quoted, true);
    }

    #[test]
    fn test_parse_command_empty() {
        let parts = parse_command("");
        assert!(parts.is_empty());
    }

    #[test]
    fn test_parse_command_glob_not_quoted() {
        let parts = parse_command("ls *.rs");
        assert_eq!(parts[1].value, "*.rs");
        assert_eq!(parts[1].quoted, false);
    }

    #[test]
    fn test_parse_command_glob_quoted() {
        let parts = parse_command("find . -name '*.txt'");
        let glob_token = &parts[3];
        assert_eq!(glob_token.value, "*.txt");
        assert_eq!(glob_token.quoted, true);
    }

    #[test]
    fn test_parse_pipeline() {
        let stages = parse_pipeline("ls -la | grep foo | wc -l");
        assert_eq!(stages.len(), 3);
        let s0: Vec<String> = stages[0].iter().map(|t| t.value.clone()).collect();
        let s1: Vec<String> = stages[1].iter().map(|t| t.value.clone()).collect();
        let s2: Vec<String> = stages[2].iter().map(|t| t.value.clone()).collect();
        assert_eq!(s0, vec!["ls", "-la"]);
        assert_eq!(s1, vec!["grep", "foo"]);
        assert_eq!(s2, vec!["wc", "-l"]);
    }

    #[test]
    fn test_parse_pipeline_quotes() {
        let stages = parse_pipeline("echo \"hello | world\" | cat");
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0][1].value, "hello | world");
        assert_eq!(stages[1][0].value, "cat");
    }

    #[test]
    fn test_is_python_command() {
        assert!(is_python_command("python -c 'print(1)'"));
        assert!(is_python_command("python3 -c 'print(1)'"));
        assert!(is_python_command("python script.py"));
        assert!(!is_python_command("ls -la"));
    }

    #[test]
    fn test_execute_shell_command() {
        let mut rt = setup_runtime();
        let result = rt.execute("echo hello");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello"));
    }

    #[test]
    fn test_execute_ls() {
        let mut rt = setup_runtime();
        let result = rt.execute("ls");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_glob() {
        let mut rt = setup_runtime();
        rt.execute("touch a.txt");
        rt.execute("touch b.txt");
        let result = rt.execute("echo *.txt");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("a.txt"));
        assert!(result.stdout.contains("b.txt"));
    }

    #[test]
    fn test_execute_glob_quoted() {
        let mut rt = setup_runtime();
        let result = rt.execute("echo '*.txt'");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "*.txt");
    }

    #[test]
    fn test_execute_pipeline() {
        let mut rt = setup_runtime();
        let result = rt.execute("echo hello world | wc -w");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.trim().contains("2"));
    }

    #[test]
    fn test_execute_pipeline_grep() {
        let mut rt = setup_runtime();
        let result = rt.execute("echo \"hello\nworld\nhello again\" | grep hello | wc -l");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "2");
    }

    #[test]
    fn test_execute_python() {
        let mut rt = setup_runtime();
        let result = rt.execute("python -c 'print(42)'");
        if rt.python_available() {
            assert_eq!(result.exit_code, 0);
            assert!(result.stdout.contains("42"));
        }
    }

    #[test]
    fn test_execute_python_code_direct() {
        let mut rt = setup_runtime();
        let result = rt.execute_python_code("print('direct call')");
        if rt.python_available() {
            assert_eq!(result.exit_code, 0);
            assert!(result.stdout.contains("direct call"));
        }
    }

    #[test]
    fn test_execute_empty() {
        let mut rt = setup_runtime();
        let result = rt.execute("");
        assert_eq!(result.exit_code, 0);
    }

    // ── Redirect tests ────────────────────────────────────────

    #[test]
    fn test_redirect_stdout_truncate() {
        let mut rt = setup_runtime();
        let r = rt.execute("echo hello > /out.txt");
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.is_empty());
        let content = rt.shell.vfs.read_to_string("/out.txt", &rt.shell.cwd).unwrap();
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_redirect_stdout_append() {
        let mut rt = setup_runtime();
        rt.execute("echo first > /append.txt");
        let r = rt.execute("echo second >> /append.txt");
        assert_eq!(r.exit_code, 0);
        let content = rt.shell.vfs.read_to_string("/append.txt", &rt.shell.cwd).unwrap();
        assert!(content.contains("first"));
        assert!(content.contains("second"));
    }

    #[test]
    fn test_redirect_stdin() {
        let mut rt = setup_runtime();
        rt.shell.vfs.write("/input.txt", &rt.shell.cwd, "hello stdin").unwrap();
        let r = rt.execute("cat < /input.txt");
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("hello stdin"));
    }

    #[test]
    fn test_redirect_stderr_to_file() {
        let mut rt = setup_runtime();
        // Use a command that definitely produces stderr
        let r = rt.execute("cat /nonexistent_path_xyz 2> /err.txt");
        assert!(r.stderr.is_empty(), "stderr should be empty after redirect, got: {:?}", r.stderr);
        let content = rt.shell.vfs.read_to_string("/err.txt", &rt.shell.cwd).unwrap_or_else(|_| String::new());
        assert!(!content.is_empty(), "err.txt should contain error message");
        assert!(content.contains("nonexistent") || content.contains("Not found"));
    }

    #[test]
    fn test_redirect_merge_stderr() {
        let mut rt = setup_runtime();
        let r = rt.execute("ls /nonexistent_path 2>&1");
        assert!(r.stderr.is_empty());
        assert!(r.stdout.contains("nonexistent") || r.stdout.contains("Not found"));
    }

    #[test]
    fn test_redirect_both_and() {
        let mut rt = setup_runtime();
        let r = rt.execute("echo merged >& /both.txt");
        assert_eq!(r.exit_code, 0);
        let content = rt.shell.vfs.read_to_string("/both.txt", &rt.shell.cwd).unwrap();
        assert_eq!(content.trim(), "merged");
    }

    #[test]
    fn test_redirect_pipeline_with_redirect() {
        let mut rt = setup_runtime();
        let r = rt.execute("echo hello | grep hello > /pipe_out.txt");
        assert_eq!(r.exit_code, 0);
        let content = rt.shell.vfs.read_to_string("/pipe_out.txt", &rt.shell.cwd).unwrap();
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_redirect_only_redirect_no_command() {
        let mut rt = setup_runtime();
        // Just a redirect with no command — should not crash
        let r = rt.execute("> /empty.txt");
        assert_eq!(r.exit_code, 0);
        assert!(rt.shell.vfs.exists("/empty.txt", &rt.shell.cwd));
    }

    #[test]
    fn test_redirect_stdout_and_stderr_separate() {
        let mut rt = setup_runtime();
        // Redirect stdout to file A, stderr to file B with different targets
        rt.execute("echo stdout_msg 1> /stdout.txt");
        rt.execute("cat /nonexistent_path_xyz 2> /stderr.txt");
        let out = rt.shell.vfs.read_to_string("/stdout.txt", &rt.shell.cwd).unwrap();
        let err = rt.shell.vfs.read_to_string("/stderr.txt", &rt.shell.cwd).unwrap();
        assert!(out.contains("stdout_msg"));
        assert!(!err.is_empty(), "stderr file should contain error message");
    }

    #[test]
    fn test_parse_redirects_stdout() {
        let args: Vec<&str> = vec!["echo", "hello", ">", "file.txt"];
        let (clean, spec) = parse_redirects(&args);
        assert_eq!(clean, vec!["echo", "hello"]);
        assert_eq!(spec.stdout_file, Some(("file.txt".to_string(), false)));
    }

    #[test]
    fn test_parse_redirects_append() {
        let args: Vec<&str> = vec!["echo", "hello", ">>", "file.txt"];
        let (_clean, spec) = parse_redirects(&args);
        assert_eq!(spec.stdout_file, Some(("file.txt".to_string(), true)));
    }

    #[test]
    fn test_parse_redirects_rightmost_wins() {
        let args: Vec<&str> = vec!["cmd", ">", "first.txt", ">", "second.txt"];
        let (clean, spec) = parse_redirects(&args);
        assert_eq!(clean, vec!["cmd"]);
        assert_eq!(spec.stdout_file, Some(("second.txt".to_string(), false)));
    }

    #[test]
    fn test_parse_redirects_no_redirects() {
        let args: Vec<&str> = vec!["ls", "-la"];
        let (clean, spec) = parse_redirects(&args);
        assert_eq!(clean, vec!["ls", "-la"]);
        assert!(spec.stdout_file.is_none());
        assert!(spec.stderr_file.is_none());
        assert!(spec.stdin_file.is_none());
        assert!(!spec.merge_stderr_to_stdout);
        assert!(!spec.merge_stdout_to_stderr);
    }

    // ─────────────── logical segmentation / shell syntax ───────────────

    #[test]
    fn test_semicolon_chaining() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo one; echo two");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "one\ntwo\n");
    }

    #[test]
    fn test_and_if_chaining() {
        let mut rt = setup_runtime();
        let out = rt.execute("mkdir -p sub && echo ok");
        assert_eq!(out.stdout, "ok\n");
        // Failure short-circuits &&
        let out = rt.execute("false && echo skipped");
        assert!(out.stdout.is_empty());
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_or_if_chaining() {
        let mut rt = setup_runtime();
        let out = rt.execute("false || echo fallback");
        assert_eq!(out.stdout, "fallback\n");
        assert_eq!(out.exit_code, 0);
        let out = rt.execute("true || echo skipped");
        assert!(out.stdout.is_empty());
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_heredoc_creates_file() {
        let mut rt = setup_runtime();
        let out = rt.execute("cat > h.py << 'EOF'\nprint('hi')\nline2\nEOF");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        let out = rt.execute("cat h.py");
        assert_eq!(out.stdout, "print('hi')\nline2\n");
    }

    #[test]
    fn test_heredoc_dash_strips_tabs() {
        let mut rt = setup_runtime();
        let out = rt.execute("cat <<- EOF\n\tindented\n\tEOF");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "indented\n");
    }

    #[test]
    fn test_heredoc_as_pipeline_stdin() {
        let mut rt = setup_runtime();
        let out = rt.execute("cat << 'EOF' | wc -l\na\nb\nc\nEOF");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout.trim(), "3");
    }

    #[test]
    fn test_command_substitution() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo $(echo nested)");
        assert_eq!(out.stdout, "nested\n");
        // Backticks
        let out = rt.execute("echo `echo tick`");
        assert_eq!(out.stdout, "tick\n");
        // Single quotes protect
        let out = rt.execute("echo '$(echo nested)'");
        assert_eq!(out.stdout, "$(echo nested)\n");
    }

    #[test]
    fn test_variable_assignment_and_expansion() {
        let mut rt = setup_runtime();
        let out = rt.execute("X=42");
        assert_eq!(out.exit_code, 0);
        let out = rt.execute("echo $X");
        assert_eq!(out.stdout, "42\n");
        let out = rt.execute("echo ${X}!");
        assert_eq!(out.stdout, "42!\n");
        // export form
        rt.execute("export Y=hello");
        let out = rt.execute("echo $Y world");
        assert_eq!(out.stdout, "hello world\n");
        // Single quotes protect
        let out = rt.execute("echo '$X'");
        assert_eq!(out.stdout, "$X\n");
    }

    #[test]
    fn test_exit_code_variable() {
        let mut rt = setup_runtime();
        rt.execute("false");
        let out = rt.execute("echo $?");
        assert_eq!(out.stdout, "1\n");
        rt.execute("true");
        let out = rt.execute("echo $?");
        assert_eq!(out.stdout, "0\n");
    }

    #[test]
    fn test_tilde_expansion() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo ~");
        assert_eq!(out.stdout, "/\n");
        // Not expanded mid-word
        let out = rt.execute("echo a~b");
        assert_eq!(out.stdout, "a~b\n");
    }

    #[test]
    fn test_multiline_input_as_commands() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo first\necho second");
        assert_eq!(out.stdout, "first\nsecond\n");
    }

    #[test]
    fn test_var_assignment_with_substitution() {
        let mut rt = setup_runtime();
        rt.execute("D=$(pwd)");
        let out = rt.execute("echo $D");
        assert_eq!(out.stdout, "/\n");
    }

    #[test]
    fn test_chain_with_cd_state() {
        let mut rt = setup_runtime();
        let out = rt.execute("mkdir -p dir1 && cd dir1 && pwd");
        assert_eq!(out.stdout.trim(), "/dir1");
    }

    #[test]
    fn test_redirect_with_heredoc_and_chain() {
        let mut rt = setup_runtime();
        let out =
            rt.execute("cat > f.txt << 'EOF'\ndata\nEOF\ncat f.txt && echo done");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("data"));
        assert!(out.stdout.contains("done"));
    }

    #[test]
    fn test_two_amp_in_quoted_string_untouched() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo 'a && b'");
        assert_eq!(out.stdout, "a && b\n");
        let out = rt.execute("echo \"x || y\"");
        assert_eq!(out.stdout, "x || y\n");
    }

    #[test]
    fn test_stderr_merge_still_works() {
        let mut rt = setup_runtime();
        // 2>&1 must not be treated as a chain separator
        let out = rt.execute("ls /nonexistent 2>&1");
        assert!(out.stdout.contains("nonexistent") || out.stderr.is_empty());
    }

    #[test]
    fn test_python_heredoc_body() {
        let mut rt = setup_runtime();
        if !rt.python_available() {
            return;
        }
        let out = rt.execute("python3 << 'EOF'\nprint(6*7)\nEOF");
        assert!(out.stdout.contains("42"), "stdout={}", out.stdout);
    }

    #[test]
    fn test_take_assignment_parsing() {
        let (n, v, rest) = take_assignment("X=5").unwrap();
        assert_eq!((n.as_str(), v.as_str(), rest), ("X", "5", ""));
        let (n, v, rest) = take_assignment("NAME='a b' cmd").unwrap();
        assert_eq!((n.as_str(), v.as_str(), rest.trim()), ("NAME", "a b", "cmd"));
        assert!(take_assignment("ls -la").is_none());
        assert!(take_assignment("dd if=/dev/zero").is_none());
        assert!(take_assignment("3X=5").is_none());
    }

    #[test]
    fn test_split_segments_shapes() {
        let segs = split_segments("a; b && c || d");
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0].connector, Connector::Always);
        assert_eq!(segs[1].connector, Connector::Always);
        assert_eq!(segs[2].connector, Connector::AndIf);
        assert_eq!(segs[3].connector, Connector::OrIf);
        // Pipes are not split
        let segs = split_segments("a | b");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "a | b");
        // Heredoc collected
        let segs = split_segments("cat << EOF\nbody\nEOF\necho after");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].heredoc.as_deref(), Some("body\n"));
        assert_eq!(segs[1].text, "echo after");
    }

    // ── Arithmetic expansion tests ────────────────────────────

    #[test]
    fn test_arithmetic_basic_add() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("1 + 2"), 3);
    }

    #[test]
    fn test_arithmetic_mul() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("3 * 4"), 12);
    }

    #[test]
    fn test_arithmetic_div() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("10 / 3"), 3);
    }

    #[test]
    fn test_arithmetic_mod() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("10 % 3"), 1);
    }

    #[test]
    fn test_arithmetic_precedence_mul_first() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("1 + 2 * 3"), 7);
    }

    #[test]
    fn test_arithmetic_precedence_parens() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("(1 + 2) * 3"), 9);
    }

    #[test]
    fn test_arithmetic_bitwise_or() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("1 | 2"), 3);
    }

    #[test]
    fn test_arithmetic_bitwise_and() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("3 & 1"), 1);
    }

    #[test]
    fn test_arithmetic_bitwise_xor() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("1 ^ 3"), 2);
    }

    #[test]
    fn test_arithmetic_shift_left() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("4 << 1"), 8);
    }

    #[test]
    fn test_arithmetic_shift_right() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("8 >> 1"), 4);
    }

    #[test]
    fn test_arithmetic_ternary_true() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("1 ? 10 : 20"), 10);
    }

    #[test]
    fn test_arithmetic_ternary_false() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("0 ? 10 : 20"), 20);
    }

    #[test]
    fn test_arithmetic_nested_parens() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("(1 + 2) * (3 + 4)"), 21);
    }

    #[test]
    fn test_arithmetic_unary_neg() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("-3 + 5"), 2);
    }

    #[test]
    fn test_arithmetic_unary_bitnot() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("~0"), -1);
    }

    #[test]
    fn test_arithmetic_unary_lognot() {
        let rt = setup_runtime();
        assert_eq!(rt.eval_arithmetic("!0"), 1);
        assert_eq!(rt.eval_arithmetic("!5"), 0);
    }

    #[test]
    fn test_arithmetic_execute_echo() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo $((1 + 2))");
        assert_eq!(out.stdout, "3\n");
    }

    #[test]
    fn test_arithmetic_execute_with_var() {
        let mut rt = setup_runtime();
        rt.execute("X=5");
        let out = rt.execute("echo $((X + 3))");
        assert_eq!(out.stdout, "8\n");
    }

    #[test]
    fn test_arithmetic_execute_dollar_var() {
        let mut rt = setup_runtime();
        rt.execute("Y=10");
        let out = rt.execute("echo $(( $Y / 2 ))");
        assert_eq!(out.stdout, "5\n");
    }

    #[test]
    fn test_arithmetic_execute_complex() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo $(( (10 + 2) * (8 - 3) / 2 ))");
        assert_eq!(out.stdout, "30\n");
    }

    // ── Recursive glob (**) tests ──────────────────────────────

    #[test]
    fn test_recursive_glob_basic() {
        let mut rt = setup_runtime();
        rt.execute("mkdir -p /src/sub/deep");
        rt.execute("touch /src/main.rs");
        rt.execute("touch /src/lib.rs");
        rt.execute("touch /src/sub/mod.rs");
        rt.execute("touch /src/sub/deep/util.rs");
        let out = rt.execute("echo src/**/*.rs");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("src/main.rs"));
        assert!(out.stdout.contains("src/lib.rs"));
        assert!(out.stdout.contains("src/sub/mod.rs"));
        assert!(out.stdout.contains("src/sub/deep/util.rs"));
    }

    #[test]
    fn test_recursive_glob_single_file() {
        let mut rt = setup_runtime();
        rt.execute("mkdir -p /a/b/c");
        rt.execute("touch /a/b/c/test.txt");
        let out = rt.execute("echo **/test.txt");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("a/b/c/test.txt"));
    }

    #[test]
    fn test_recursive_glob_no_match() {
        let mut rt = setup_runtime();
        rt.execute("mkdir -p /src");
        rt.execute("touch /src/main.rs");
        let out = rt.execute("echo src/**/*.py");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(!out.stdout.contains(".rs"));
        let out_trimmed = out.stdout.trim();
        assert!(out_trimmed.is_empty() || out_trimmed == "src/**/*.py",
            "expected empty or literal, got: {}", out_trimmed);
    }

    #[test]
    fn test_recursive_glob_from_subdir() {
        let mut rt = setup_runtime();
        rt.execute("mkdir -p /project/src/sub");
        rt.execute("touch /project/src/lib.rs");
        rt.execute("touch /project/src/sub/mod.rs");
        rt.execute("touch /project/readme.md");
        rt.execute("cd /project");
        let out = rt.execute("echo src/**/*.rs");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("src/lib.rs"));
        assert!(out.stdout.contains("src/sub/mod.rs"));
        assert!(!out.stdout.contains("readme.md"));
    }

    #[test]
    fn test_recursive_glob_only_txt() {
        let mut rt = setup_runtime();
        rt.execute("mkdir -p /dir/sub");
        rt.execute("touch /dir/a.txt");
        rt.execute("touch /dir/b.rs");
        rt.execute("touch /dir/sub/c.txt");
        rt.execute("touch /dir/sub/d.md");
        let out = rt.execute("echo dir/**/*.txt");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("dir/a.txt"));
        assert!(out.stdout.contains("dir/sub/c.txt"));
        assert!(!out.stdout.contains(".rs"));
        assert!(!out.stdout.contains(".md"));
    }

    // ── Process substitution tests ─────────────────────────────

    #[test]
    fn test_process_substitution_input_basic() {
        let mut rt = setup_runtime();
        let out = rt.execute("cat <(echo hello)");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout, "hello\n");
    }

    #[test]
    fn test_process_substitution_input_file() {
        let mut rt = setup_runtime();
        let out = rt.execute("wc -w <(echo hello world)");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("2"));
    }

    #[test]
    fn test_process_substitution_two_inputs() {
        let mut rt = setup_runtime();
        let out = rt.execute("cat <(echo a) <(echo b)");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout, "a\nb\n");
    }

    #[test]
    fn test_process_substitution_with_pipe() {
        let mut rt = setup_runtime();
        let out = rt.execute("cat <(echo hello world | wc -c)");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("12"));
    }

    #[test]
    fn test_process_substitution_output_basic() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo hello > >(cat)");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
    }

    #[test]
    fn test_process_substitution_in_single_quotes_untouched() {
        let mut rt = setup_runtime();
        let out = rt.execute("echo '<(hello)'");
        assert_eq!(out.stdout, "<(hello)\n");
    }

    #[test]
    fn test_process_substitution_records_temp_files() {
        let mut rt = setup_runtime();
        assert!(rt.tmp_files.is_empty());
        rt.execute("cat <(echo hello)");
        assert_eq!(rt.tmp_files.len(), 1);
        assert!(rt.tmp_files[0].starts_with("/tmp/psub_"));
    }

    #[test]
    fn test_process_substitution_with_redirect() {
        let mut rt = setup_runtime();
        rt.execute("cat <(echo hello) > /out.txt");
        let content = rt.shell.vfs.read_to_string("/out.txt", &rt.shell.cwd).unwrap();
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_process_substitution_empty_output() {
        let mut rt = setup_runtime();
        let out = rt.execute("cat <(true)");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout, "");
    }

    // ── Function tests ─────────────────────────────────────────

    #[test]
    fn test_function_define_and_call() {
        let mut rt = setup_runtime();
        let out = rt.execute("hello() { echo Hello; }; hello");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout, "Hello\n");
    }

    #[test]
    fn test_function_with_arguments() {
        let mut rt = setup_runtime();
        let out = rt.execute("greet() { echo $1; }; greet World");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout, "World\n");
    }

    #[test]
    fn test_function_with_multiple_args() {
        let mut rt = setup_runtime();
        let out = rt.execute("add() { echo $(( $1 + $2 )); }; add 3 5");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout, "8\n");
    }

    #[test]
    fn test_recursive_function() {
        let mut rt = setup_runtime();
        let out = rt.execute(
            "count() { if [ \"$1\" -gt 0 ]; then echo $1; count $(( $1 - 1 )); fi; }; count 3",
        );
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout, "3\n2\n1\n");
    }

    #[test]
    fn test_function_not_found() {
        let mut rt = setup_runtime();
        let out = rt.execute("nonexistent_func");
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_try_parse_function_def_simple() {
        let r = try_parse_function_def("myfunc() { echo hello; }");
        assert!(r.is_some());
        let (name, body) = r.unwrap();
        assert_eq!(name, "myfunc");
        assert_eq!(body, " echo hello; ");
    }

    #[test]
    fn test_try_parse_function_def_nested_braces() {
        let r = try_parse_function_def("count() { if true; then echo hi; fi; }");
        assert!(r.is_some());
        let (name, body) = r.unwrap();
        assert_eq!(name, "count");
        assert!(body.contains("if true"));
    }

    #[test]
    fn test_try_parse_function_def_not_a_function() {
        assert!(try_parse_function_def("ls -la").is_none());
        assert!(try_parse_function_def("echo()").is_none());
        assert!(try_parse_function_def("func()").is_none());
    }

    // ── for / while / case tests ─────────────────────────────────

    #[test]
    fn test_for_loop_literal_echo() {
        let mut rt = mk_rt();
        let out = rt.execute("for f in 1 2; do echo hi; done");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "hi\nhi");
    }

    #[test]
    fn test_for_loop_basic() {
        let mut rt = mk_rt();
        let out = rt.execute("for f in a b c; do echo $f; done");
        eprintln!("FOR DEBUG: stdout='{}' stderr='{}' exit={}", out.stdout, out.stderr, out.exit_code);
        assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
        // At minimum the loop should execute and produce output
        assert!(!out.stdout.is_empty(), "stdout is empty: stderr='{}'", out.stderr);
    }

    #[test]
    fn test_for_loop_single_word() {
        let mut rt = mk_rt();
        let out = rt.execute("for x in hello; do echo $x; done");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "hello");
    }

    #[test]
    fn test_for_missing_done_errors() {
        let mut rt = mk_rt();
        let out = rt.execute("for x in a; do echo x");
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_while_loop() {
        let mut rt = mk_rt();
        let out = rt.execute("i=0; while test $i -lt 3; do i=$((i+1)); echo $i; done");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("1"));
        assert!(out.stdout.contains("3"));
    }

    #[test]
    fn test_while_never_true() {
        let mut rt = mk_rt();
        let out = rt.execute("while false; do echo unreachable; done");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn test_until_loop() {
        let mut rt = mk_rt();
        let out = rt.execute("i=3; until test $i -le 0; do i=$((i-1)); echo $i; done");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("2"));
        assert!(out.stdout.contains("0"));
    }

    #[test]
    fn test_case_match_first() {
        let mut rt = mk_rt();
        let out = rt.execute("case apple in apple) echo found;; banana) echo no;; esac");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "found");
    }

    #[test]
    fn test_case_match_second() {
        let mut rt = mk_rt();
        let out = rt.execute("case banana in apple) echo a;; banana) echo b;; *) echo other;; esac");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "b");
    }

    #[test]
    fn test_case_wildcard_fallback() {
        let mut rt = mk_rt();
        let out = rt.execute("case orange in apple) echo a;; *) echo fallback;; esac");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "fallback");
    }

    #[test]
    fn test_case_pattern_with_pipe() {
        let mut rt = mk_rt();
        let out = rt.execute("case dog in cat|dog) echo pet;; *) echo other;; esac");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "pet");
    }

    #[test]
    fn test_for_with_if_inside() {
        let mut rt = mk_rt();
        let out = rt.execute("for f in a b; do if test $f = b; then echo found; else echo skip; fi; done");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("skip"));
        assert!(out.stdout.contains("found"));
    }

    #[test]
    fn test_while_with_for_inside() {
        let mut rt = mk_rt();
        let out = rt.execute("i=0; while test $i -lt 1; do for x in hello; do echo $x; done; i=1; done");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("hello"));
    }

    #[test]
    fn test_combine_loop_segments_passthrough() {
        let segs = vec![
            Segment { connector: Connector::Always, text: "echo hi".into(), heredoc: None, heredoc_expand: false },
        ];
        let out = combine_loop_segments(segs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "echo hi");
    }

    #[test]
    fn test_pipeline_alias_resolved() {
        let mut rt = mk_rt();
        rt.shell.aliases.insert("ll".into(), "ls -la".into());
        let out = rt.execute("echo hello | ll");
        assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    }

    #[test]
    fn test_pipeline_alias_with_args() {
        let mut rt = mk_rt();
        rt.execute("echo hello > h.txt");
        rt.shell.aliases.insert("grepx".into(), "grep x".into());
        // grep for "x" in "hello" should not match (empty stdout)
        let out = rt.execute("cat h.txt | grepx");
        // May fail if grep returns no-match exit code
        assert!(out.exit_code == 0 || out.exit_code == 1,
            "unexpected exit {}: stderr={}", out.exit_code, out.stderr);
    }

    #[test]
    fn test_pipeline_without_alias_still_works() {
        let mut rt = mk_rt();
        let out = rt.execute("echo hello | tr a-z A-Z");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "HELLO");
    }

    #[test]
    fn test_shell_function_in_pipeline_not_yet_supported() {
        // Pipeline threads bypass Runtime-level function resolution;
        // only aliases are resolved.  Shell functions in pipelines
        // fall through to Shell::execute() as unrecognised commands.
        let mut rt = mk_rt();
        let out = rt.execute("myfn() { tr a-z A-Z; }; echo hello | myfn");
        assert_ne!(out.exit_code, 0, "shell functions are not yet supported in pipelines");
    }
}
