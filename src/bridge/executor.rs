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
    /// Session shell variables (set via `X=..`, `export X=..`; read via `$X`).
    vars: std::collections::HashMap<String, String>,
    /// Exit code of the previously executed segment (`$?`).
    last_exit: i32,
    /// Recursion guard for `$( ... )` command substitution.
    subst_depth: u8,
}

impl Runtime {
    pub fn new(shell: Shell, python: Option<Box<dyn PythonEngine>>) -> Self {
        // (c) 2025 xiefujin <490021684@qq.com>
        Runtime {
            shell,
            python,
            vars: std::collections::HashMap::new(),
            last_exit: 0,
            subst_depth: 0,
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

        // Fast path: a single plain segment.
        if segments.len() == 1 {
            let seg = &segments[0];
            let text = seg.text.clone();
            let heredoc = seg.heredoc.clone();
            let out = self.execute_segment(&text, heredoc.as_deref());
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
            let text = seg.text.clone();
            let heredoc = seg.heredoc.clone();
            let out = self.execute_segment(&text, heredoc.as_deref());
            exit_code = out.exit_code;
            self.last_exit = exit_code;
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
    fn execute_segment(&mut self, input: &str, heredoc: Option<&str>) -> CommandOutput {
        let raw = input.trim();
        if raw.is_empty() && heredoc.is_none() {
            return CommandOutput::success(String::new());
        }

        // Expand $(...), `...`, $VAR, ${VAR}, $? and ~ (quote-aware).
        let expanded = self.expand_line(raw);
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
        if let Some(body) = heredoc {
            let bare = input.trim_end_matches(" -").trim();
            if bare == "python" || bare == "python3" {
                return self.execute_python_code(body);
            }
        }

        if is_python_command(input) {
            return self.execute_python_inner(input);
        }

        if input.contains('|') {
            return self.execute_pipeline(input, heredoc);
        }

        let tokens = parse_command(input);
        if tokens.is_empty() {
            if heredoc.is_some() {
                return CommandOutput::success(String::new());
            }
            return CommandOutput::success(String::new());
        }
        let parts = self.expand_globs(tokens);
        let (clean, spec) = self.extract_redirects(parts);

        if clean.is_empty() {
            // Only redirect operators given — write heredoc (or empty) to file
            let mut result = CommandOutput::success(String::new());
            if let Some(body) = heredoc {
                result.stdout = body.to_string();
            }
            self.apply_redirects(&mut result, &spec);
            result.stdout.clear();
            return result;
        }

        let cmd = &clean[0];
        let args: Vec<&str> = clean[1..].iter().map(|s| s.as_str()).collect();

        // stdin: heredoc body takes precedence, then `< file`.
        let stdin_from_file = if heredoc.is_none() {
            spec.stdin_file.as_ref().and_then(|path| {
                self.shell.vfs.read_to_string(path, &self.shell.cwd).ok()
            })
        } else {
            None
        };
        let stdin_ref = heredoc.or(stdin_from_file.as_deref());

        let mut result = self.shell.execute(cmd, &args, stdin_ref);
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
            // `export X=1 Y=2` — every token must be an assignment.
            let mut all = true;
            let mut cursor = rest;
            while let Some((name, value, after)) = take_assignment(cursor) {
                self.vars.insert(name, value);
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
            self.vars.insert(name, value);
            consumed_any = true;
            cursor = after.trim_start();
        }
        if consumed_any {
            (cursor, cursor.is_empty())
        } else {
            (input, false)
        }
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
                    out.push(c);
                    out.push(chars[i + 1]);
                    i += 2;
                }
                '`' => {
                    // Backtick command substitution.
                    if let Some(close) = chars[i + 1..].iter().position(|&ch| ch == '`') {
                        let inner: String = chars[i + 1..i + 1 + close].iter().collect();
                        out.push_str(&self.run_substitution(&inner));
                        i += close + 2;
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
                        out.push_str(&self.run_substitution(&inner));
                        i = end + 1;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '$' if i + 1 < n && chars[i + 1] == '{' => {
                    if let Some(close) = chars[i + 2..].iter().position(|&ch| ch == '}') {
                        let name: String = chars[i + 2..i + 2 + close].iter().collect();
                        out.push_str(&self.lookup_var(&name));
                        i += close + 3;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '$' if i + 1 < n && chars[i + 1] == '?' => {
                    out.push_str(&self.last_exit.to_string());
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
                        || chars[i - 1] == '=';
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

    fn lookup_var(&self, name: &str) -> String {
        if let Some(v) = self.vars.get(name) {
            return v.clone();
        }
        match name {
            "PWD" => self.shell.cwd.clone(),
            "HOME" => self.home_dir(),
            "?" => self.last_exit.to_string(),
            _ => std::env::var(name).unwrap_or_default(),
        }
    }

    /// The sandbox "home" is the VFS root.
    fn home_dir(&self) -> String {
        self.vars.get("HOME").cloned().unwrap_or_else(|| "/".to_string())
    }

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
                    let result = shell.execute(&cmd, &args_refs, stdin_ref);

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

        for (i, handle) in threads.into_iter().enumerate() {
            match handle.join() {
                Ok(Ok((code, out, err))) => {
                    if !err.is_empty() {
                        if !all_stderr.is_empty() {
                            all_stderr.push('\n');
                        }
                        all_stderr.push_str(&err);
                    }
                    if i == last_idx {
                        final_exit_code = code;
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
                let cwd = self.shell_root_dir();
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
                    let cwd = self.shell_root_dir();
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
        let cwd = self.shell_root_dir();
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
        let cwd = self.shell_root_dir();
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
    let mut pending_tag: Option<(String, bool)> = None; // (tag, strip_tabs)
    let mut connector = Connector::Always;
    let mut next_connector = Connector::Always;
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;

    macro_rules! flush {
        () => {
            if !current.trim().is_empty() || heredoc.is_some() {
                segments.push(Segment {
                    connector,
                    text: current.trim().to_string(),
                    heredoc: heredoc.take(),
                });
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
                    i = j;
                    flush!();
                } else {
                    flush!();
                    i += 1;
                }
            }
            ';' => {
                flush!();
                i += 1;
            }
            '&' => {
                if i + 1 < n && chars[i + 1] == '&' {
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
                if i + 1 < n && chars[i + 1] == '|' {
                    next_connector = Connector::OrIf;
                    flush!();
                    i += 2;
                } else {
                    current.push(c);
                    i += 1;
                }
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
                if j < n && (chars[j] == '\'' || chars[j] == '"') {
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
                    i = j;
                }
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
            // -- merge stderr→stdout: 2>&1 --
            "2>&1" => {
                spec.merge_stderr_to_stdout = true;
                i -= 1;
                continue;
            }
            // -- merge stdout→stderr: 1>&2 --
            "1>&2" => {
                // Redirect stdout to stderr — swap them
                spec.merge_stderr_to_stdout = true;
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
            ">" | "1>" | ">>" | "1>>" | "2>" | "2>>" | "<" | ">&" | "&>" | "2>&1" | "1>&2"
        );
        if is_op {
            // Skip the operator
            i += 1;
            // Skip the filename for operators that consume the next token
            if matches!(t, ">" | "1>" | ">>" | "1>>" | "2>" | "2>>" | "<" | ">&" | "&>") {
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
            } else if ch == '\\' && chars.peek().map_or(false, |&n| n == '"' || n == '\\') {
                chars.next();
                current.push(chars.next().unwrap_or(ch));
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
            } else if ch == '\\' && chars.peek().map_or(false, |&n| n == '"' || n == '\\') {
                current.push(chars.next().unwrap());
            }
        } else if ch == '\'' {
            current.push(ch);
            in_single_quote = true;
        } else if ch == '"' {
            current.push(ch);
            in_double_quote = true;
        } else if ch == '|' {
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

fn has_glob_chars(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::python::SubprocessPython;
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
}
