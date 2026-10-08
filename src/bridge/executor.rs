// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::python::PythonEngine;
use crate::shell::{CommandOutput, Shell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

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
    /// Cooperative cancellation flag — shared with the SDK.
    cancel: Arc<AtomicBool>,
    /// `break N` / `continue N` levels still to consume (innermost first).
    break_levels: usize,
    continue_levels: usize,
    /// `return N` signal from inside a function call.
    return_code: Option<i32>,
    /// `exit N` signal: unwinds all nested execution.
    exit_signal: Option<i32>,
    /// Saved `(name, old_value)` for `local` declarations of the current call.
    local_saves: Vec<Vec<(String, Option<String>)>>,
    /// Deferred error from a readonly-variable assignment.
    assign_error: Option<String>,
    /// Deferred `failglob` error (set during glob expansion).
    glob_error: std::cell::Cell<Option<String>>,
    /// PRNG state for `$RANDOM`.
    rand_state: std::cell::Cell<u64>,
    /// Shell start time for `$SECONDS`.
    start_time: std::time::Instant,
}

impl Runtime {
    pub fn new(shell: Shell, python: Option<Box<dyn PythonEngine>>) -> Self {
        // (c) 2025 xiefujin <490021684@qq.com>
        let cancel = shell.cancel.clone();
        Runtime {
            shell,
            python,
            last_exit: 0,
            subst_depth: 0,
            fn_call_depth: 0,
            psub_counter: 0,
            tmp_files: Vec::new(),
            cancel,
            break_levels: 0,
            continue_levels: 0,
            return_code: None,
            exit_signal: None,
            local_saves: Vec::new(),
            assign_error: None,
            glob_error: std::cell::Cell::new(None),
            rand_state: std::cell::Cell::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0x9E3779B97F4A7C15),
            ),
            start_time: std::time::Instant::now(),
        }
    }

    /// Clears per-invocation control-flow signals. A stray `exit`/`break`/
    /// `continue` (e.g. leaked from a script or function body) used to persist
    /// on the long-lived `Runtime`, so every later multi-command input ran only
    /// its first segment. Host entry points must call this before each command.
    pub fn reset_transient(&mut self) {
        self.break_levels = 0;
        self.continue_levels = 0;
        self.return_code = None;
        self.exit_signal = None;
        self.subst_depth = 0;
        self.fn_call_depth = 0;
        self.assign_error = None;
        self.glob_error.set(None);
    }

    /// Runs `script` in an isolated subshell: cwd / vars / arrays / aliases /
    /// exports / functions and shell options are restored afterwards, and an
    /// `exit` inside terminates only the subshell (not the parent). Used for
    /// `( ... )` and for `sh -c` / `bash -c`.
    fn execute_subshell(&mut self, script: &str) -> CommandOutput {
        let saved_cwd = self.shell.cwd.clone();
        let saved_exit = self.exit_signal.take();
        let saved_vars = self.shell.vars.clone();
        let saved_arrays = self.shell.arrays.clone();
        let saved_aliases = self.shell.aliases.clone();
        let saved_exported = self.shell.exported.clone();
        let saved_functions = self.shell.functions.clone();
        let saved_errexit = self.shell.errexit;
        let saved_xtrace = self.shell.xtrace;
        let saved_nounset = self.shell.nounset;
        let saved_noclobber = self.shell.noclobber;
        let saved_break = self.break_levels;
        let saved_continue = self.continue_levels;
        let saved_return = self.return_code.take();
        let result = self.execute(script);
        self.shell.cwd = saved_cwd;
        self.shell.vars = saved_vars;
        self.shell.arrays = saved_arrays;
        self.shell.aliases = saved_aliases;
        self.shell.exported = saved_exported;
        self.shell.functions = saved_functions;
        self.shell.errexit = saved_errexit;
        self.shell.xtrace = saved_xtrace;
        self.shell.nounset = saved_nounset;
        self.shell.noclobber = saved_noclobber;
        self.break_levels = saved_break;
        self.continue_levels = saved_continue;
        self.return_code = saved_return;
        self.exit_signal = saved_exit;
        result
    }

    /// Top-level command entry for hosts: resets leaked control-flow state,
    /// then executes. Nesting uses [`Self::execute`] directly.
    pub fn execute_top(&mut self, input: &str) -> CommandOutput {
        self.reset_transient();
        let mut out = self.execute(input);
        // `trap 'CMD' ERR` (approximated): run once when the top-level command
        // exits non-zero. `INT` fires when the host cancelled the command.
        if out.exit_code == 143 {
            if let Some(handler) = self.shell.traps.get("INT").cloned() {
                if !handler.is_empty() {
                    self.reset_transient();
                    let t = self.execute(&handler);
                    out.stdout.push_str(&t.stdout);
                    out.stderr.push_str(&t.stderr);
                }
            }
        }
        // `trap 'CMD' EXIT` runs when the top-level command finishes (bash
        // runs it when the shell exits; here each `execute` is a script).
        let trap = self.shell.traps.get("EXIT").cloned();
        self.reset_transient();
        if let Some(handler) = trap {
            if !handler.is_empty() {
                let t = self.execute(&handler);
                out.stdout.push_str(&t.stdout);
                out.stderr.push_str(&t.stderr);
                if t.exit_code != 0 {
                    out.exit_code = t.exit_code;
                }
            }
        }
        out
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
            let mut out = self.run_segment_text(&text, heredoc.as_deref(), seg.heredoc_expand);
            self.last_exit = out.exit_code;
            if out.exit_code != 0 && self.exit_signal.is_none() {
                self.run_err_trap(&mut out);
            }
            return out;
        }

        let mut agg_stdout = String::new();
        let mut agg_stderr = String::new();
        let mut exit_code = 0;
        for (si, seg) in segments.iter().enumerate() {
            if self.cancel.load(Ordering::SeqCst) {
                return CommandOutput {
                    stdout: agg_stdout,
                    stderr: format!("{}cancelled\n", agg_stderr),
                    exit_code: 143,
                };
            }
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
            let out = self.run_segment_text(&text, heredoc.as_deref(), seg.heredoc_expand);
            exit_code = out.exit_code;
            self.last_exit = exit_code;
            // Append output BEFORE the `set -e` break so the failing command's
            // output is not lost.
            agg_stdout.push_str(&out.stdout);
            agg_stderr.push_str(&out.stderr);
            // `trap ... ERR`: fire on an untested failing command.
            if exit_code != 0
                && !matches!(seg.connector, Connector::AndIf | Connector::OrIf)
                && self.exit_signal.is_none()
            {
                let tested_by_next = segments
                    .get(si + 1)
                    .map(|n| matches!(n.connector, Connector::AndIf | Connector::OrIf))
                    .unwrap_or(false);
                if !tested_by_next {
                    let mut tmp = CommandOutput::success(String::new());
                    self.run_err_trap(&mut tmp);
                    agg_stdout.push_str(&tmp.stdout);
                    agg_stderr.push_str(&tmp.stderr);
                }
            }
            if self.shell.errexit && exit_code != 0 {
                break;
            }
            // `exit` unwinds every remaining segment; `break`/`continue` stop
            // the rest of the current loop body.
            if self.exit_signal.is_some() || self.break_levels > 0 || self.continue_levels > 0 {
                break;
            }
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
        // Shell options set inside a call (`set -e`, `set -x`, …) must not leak
        // into the next isolated call (each `run_shell` is independent).
        let saved_errexit = self.shell.errexit;
        let saved_xtrace = self.shell.xtrace;
        let saved_nounset = self.shell.nounset;
        let saved_noclobber = self.shell.noclobber;
        let cd = self.shell.execute("cd", &[cwd], None);
        if cd.exit_code != 0 {
            return cd;
        }
        let out = self.execute_top(input);
        self.shell.cwd = saved;
        self.shell.errexit = saved_errexit;
        self.shell.xtrace = saved_xtrace;
        self.shell.nounset = saved_nounset;
        self.shell.noclobber = saved_noclobber;
        out
    }

    /// Executes one logical command segment (no `;`/`&&`/`||`/newline inside).
    /// Runs the `ERR` trap (if any) and appends its output to `out`.
    fn run_err_trap(&mut self, out: &mut CommandOutput) {
        if let Some(h) = self.shell.traps.get("ERR").cloned() {
            if !h.is_empty() {
                self.reset_transient();
                let t = self.execute(&h);
                out.stdout.push_str(&t.stdout);
                out.stderr.push_str(&t.stderr);
            }
        }
    }

    /// Runs one segment, honoring a trailing background `&`: the command runs
    /// synchronously (in-process) and is then recorded as a job so `$!`,
    /// `jobs` and `wait` behave sensibly.
    fn run_segment_text(
        &mut self,
        text: &str,
        heredoc: Option<&str>,
        heredoc_expand: bool,
    ) -> CommandOutput {
        let trimmed = text.trim_end();
        if trimmed.ends_with('&') && !trimmed.ends_with("&&") {
            let cmd = trimmed[..trimmed.len() - 1].trim_end().to_string();
            let out = self.execute_segment(&cmd, heredoc, heredoc_expand);
            self.record_background(&cmd);
            out
        } else {
            self.execute_segment(trimmed, heredoc, heredoc_expand)
        }
    }

    fn record_background(&mut self, cmd: &str) {
        self.shell.job_counter += 1;
        let pid = format!(
            "{}",
            self.shell.pid.wrapping_add(self.shell.job_counter as u32)
        );
        self.shell.jobs.push(crate::shell::Job {
            pid: pid.clone(),
            cmd: cmd.to_string(),
            status: "Done".to_string(),
        });
        self.shell.last_bg_pid = pid;
    }

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
        // A bare comment line is a no-op (comments are normally stripped during
        // splitting; this guards the single-segment fast path).
        if raw.starts_with('#') {
            return CommandOutput::success(String::new());
        }

        if self.shell.xtrace {
            eprintln!("+ {}", raw);
        }

        if let Some((name, body)) = try_parse_function_def(raw) {
            self.shell
                .functions
                .insert(name.to_string(), body.trim().to_string());
            return CommandOutput::success(String::new());
        }

        // `( … ) > f 2>&1` / `{ …; } > f`: a group followed by redirects. The
        // checks below require the segment to *end* with `)`/`}`; here we split
        // the leading group from the trailing redirect text, run the group, then
        // apply the redirects to its output.
        if raw.starts_with('(') || raw.starts_with('{') {
            if let Some((construct, trailing)) = split_leading_group(raw) {
                // Only when the trailing text is redirects — a `|` means this is
                // a pipeline (`{ …; } | cmd`), which the pipe path below handles.
                if !trailing.trim().is_empty()
                    && !trailing.contains('|')
                    && is_group_construct(&construct)
                {
                    let mut out = self.execute_segment(&construct, heredoc.as_deref(), false);
                    let toks = self.expand_globs(parse_command(trailing.trim()));
                    let (_, spec) = self.extract_redirects(toks);
                    self.apply_redirects(&mut out, &spec);
                    return out;
                }
            }
        }

        // Subshell `( cmd1; cmd2 )` — run inner commands with cwd isolated.
        // `(( ... ))` is an arithmetic command, not a subshell.
        if raw.starts_with('(') && raw.ends_with(')') && !raw.starts_with("((") {
            let inner = raw[1..raw.len() - 1].trim();
            if !inner.is_empty() {
                return self.execute_subshell(inner);
            }
        }

        // Command group `{ cmd1; cmd2; }` — run inner commands in this shell
        // (cwd changes propagate). Distinguish from brace expansion `{a,b}`.
        if raw.starts_with('{') && raw.ends_with('}') && raw.len() > 2 {
            let inner = raw[1..raw.len() - 1].trim();
            let is_group = inner.contains(';')
                || inner.contains("&&")
                || inner.contains("||")
                || inner.contains('\n');
            if is_group && !inner.is_empty() {
                return self.execute(inner);
            }
        }

        let first_word = raw.trim().split_whitespace().next().unwrap_or("");
        // `[[ ... ]]` conditional expression.
        if raw.starts_with("[[") && raw.ends_with("]]") && raw.len() >= 4 {
            let inner = raw[2..raw.len() - 2].trim().to_string();
            let inner = self.expand_line(&inner);
            let ok = self.eval_dbracket(&inner);
            return CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: if ok { 0 } else { 1 },
            };
        }
        // `(( ... ))` arithmetic command (applies side effects like `i++`).
        if raw.starts_with("((") && raw.ends_with("))") && raw.len() >= 4 {
            let inner = raw[2..raw.len() - 2].trim();
            let v = self.eval_arith_command(inner);
            return CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: if v != 0 { 0 } else { 1 },
            };
        }

        // `let expr...` arithmetic builtin: evaluates each expression (applying
        // assignments) and returns 0 when the last one is non-zero.
        {
            let t = raw.trim_start();
            if let Some(rest) = t.strip_prefix("let") {
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    let expr = self.expand_line(rest.trim());
                    let mut last: i64 = 0;
                    let mut any = false;
                    for part in expr.split_whitespace() {
                        let p = part.trim_matches(|c| c == '\'' || c == '"');
                        if p.is_empty() {
                            continue;
                        }
                        last = self.eval_arith_command(p);
                        any = true;
                    }
                    let ok = any && last != 0;
                    return CommandOutput {
                        stdout: String::new(),
                        stderr: String::new(),
                        exit_code: if ok { 0 } else { 1 },
                    };
                }
            }
        }

        // `! cmd` — negate the exit status of the following command.
        {
            let t = raw.trim_start();
            if t == "!" {
                return CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 1,
                };
            }
            let negated = t.strip_prefix("! ").or_else(|| t.strip_prefix("!\t"));
            if let Some(rest) = negated {
                let rest = rest.trim_start();
                if !rest.is_empty() {
                    let out = self.execute(rest);
                    return CommandOutput {
                        stdout: out.stdout,
                        stderr: out.stderr,
                        exit_code: if out.exit_code == 0 { 1 } else { 0 },
                    };
                }
            }
        }

        // `nohup CMD` — fastshell has no SIGHUP/job control, so run the command
        // in-process. This makes builtins (echo/python3/…) work on mobile,
        // where there is no external `nohup` binary.
        if let Some(rest) = raw.trim_start().strip_prefix("nohup") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                let rest = rest.trim_start();
                if rest.is_empty() {
                    return CommandOutput::error("nohup: missing command\n".to_string(), 1);
                }
                return self.execute(rest);
            }
        }

        // `time CMD` — run the command and report elapsed wall time on stderr.
        if let Some(rest) = raw.trim_start().strip_prefix("time") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                let rest = rest.trim_start();
                if !rest.is_empty() {
                    let start = std::time::Instant::now();
                    let mut out = self.execute(rest);
                    out.stderr = format!(
                        "{}\nreal\t{:.3}s\n",
                        out.stderr,
                        start.elapsed().as_secs_f64()
                    );
                    return out;
                }
            }
        }
        if first_word == "if"
            || first_word == "for"
            || first_word == "while"
            || first_word == "until"
            || first_word == "case"
        {
            return self.execute_block_construct(raw, heredoc_ref);
        }

        // `PRODUCER | while read VAR...; do BODY; done` (common idiom the
        // threaded pipeline path can't express).
        if let Some(out) = self.try_pipe_into_while(raw) {
            return out;
        }

        // `PRODUCER | { read VAR...; BODY; }` — read one line, run the body.
        if let Some(out) = self.try_pipe_into_group(raw) {
            return out;
        }

        // `PRODUCER | python3 -c '...'` — feed the producer to Python's stdin.
        if let Some(out) = self.try_python_into_pipe(raw) {
            return out;
        }
        if let Some(out) = self.try_construct_into_pipe(raw) {
            return out;
        }
        if let Some(out) = self.try_shell_script(raw) {
            return out;
        }
        if let Some(out) = self.try_pipe_into_shell(raw) {
            return out;
        }
        if let Some(out) = self.try_pipe_into_python(raw) {
            return out;
        }

        let expanded = self.expand_line(raw);
        let input = expanded.trim();
        if input.is_empty() {
            return CommandOutput::success(String::new());
        }

        // Variable assignments: `X=v`, `export X=v`, `X=v command ...`.
        let (input, assign_only) = self.consume_assignments(input);
        if let Some(err) = self.assign_error.take() {
            return CommandOutput::error(err, 1);
        }
        if assign_only {
            return CommandOutput::success(String::new());
        }
        let input = input.trim();
        if input.is_empty() {
            return CommandOutput::success(String::new());
        }

        // `python3 << 'EOF' ... EOF` — run the heredoc body as Python code.
        // Also `python3 - [args] << 'EOF' ... EOF`: the heredoc is the program
        // and the remaining args go to sys.argv (like `python3 - a b`).
        if let Some(body) = heredoc_ref {
            // Strip output redirects first so `python3 << EOF > out` also works.
            let tokens = parse_command(input);
            let parts = self.expand_globs(tokens);
            if let Some(e) = self.glob_error.take() {
                return CommandOutput::error(e, 1);
            }
            let (clean, spec) = self.extract_redirects(parts);
            if let Some(first) = clean.first().map(|t| t.value.as_str()) {
                if matches!(first, "python" | "python2" | "python2.7" | "python3" | "py") {
                    let rest = &clean[1..];
                    // `python3 << EOF` or `python3 - [args] << EOF` — the heredoc
                    // is the program (extra args go to sys.argv).
                    if rest.is_empty() || rest[0].value == "-" {
                        let args: Vec<&str> = if rest.is_empty() {
                            Vec::new()
                        } else {
                            rest[1..].iter().map(|t| t.value.as_str()).collect()
                        };
                        let code = if args.is_empty() {
                            body.to_string()
                        } else {
                            let argv = std::iter::once(first.to_string())
                                .chain(args.iter().map(|s| s.to_string()))
                                .map(|a| format!("{a:?}"))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("import sys\nsys.argv = [{argv}]\n{body}")
                        };
                        let mut result = self.execute_python_code(&code);
                        self.apply_redirects(&mut result, &spec);
                        return result;
                    }
                }
            }
        }

        if is_python_command(input) {
            // Align with a real shell: `python3 -c "..." 2>&1` / `... > file`
            // must strip the redirect before passing the rest to the Python
            // engine. Otherwise the redirect text ends up inside the `-c`
            // code and produces a SyntaxError.
            let tokens = parse_command(input);
            let parts = self.expand_globs(tokens);
            if let Some(e) = self.glob_error.take() {
                return CommandOutput::error(e, 1);
            }
            let (clean, spec) = self.extract_redirects(parts);
            // Pass arg TOKENS (not a rejoined string) so `python3 -c CODE args`
            // keeps CODE intact and exposes the trailing args via sys.argv.
            let clean_tokens: Vec<String> = clean.iter().map(|t| t.value.clone()).collect();
            let mut result = self.execute_python_argv(&clean_tokens);
            self.apply_redirects(&mut result, &spec);
            return result;
        }

        if has_unquoted_pipe(input) {
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
        if let Some(e) = self.glob_error.take() {
            return CommandOutput::error(e, 1);
        }
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

        let cmd = &clean[0].value;
        let args: Vec<&str> = clean[1..].iter().map(|t| t.value.as_str()).collect();

        // Shell control-flow keywords (must run before alias/function dispatch).
        match cmd.as_str() {
            "break" => {
                let n = args
                    .first()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                self.break_levels = n;
                return CommandOutput::success(String::new());
            }
            "continue" => {
                let n = args
                    .first()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                self.continue_levels = n;
                return CommandOutput::success(String::new());
            }
            "return" => {
                let code = args
                    .first()
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(self.last_exit);
                self.return_code = Some(code);
                return CommandOutput::success(String::new());
            }
            "exit" => {
                let code = args
                    .first()
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(self.last_exit);
                self.exit_signal = Some(code);
                return CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: code,
                };
            }
            "shift" => {
                let n = args
                    .first()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                if self.shell.positional.len() > 1 {
                    let end = (1 + n).min(self.shell.positional.len());
                    self.shell.positional.drain(1..end);
                }
                return CommandOutput::success(String::new());
            }
            "set" if args.first() == Some(&"--") => {
                let zero = self.shell.positional.first().cloned().unwrap_or_default();
                self.shell.positional = std::iter::once(zero)
                    .chain(args[1..].iter().map(|s| s.to_string()))
                    .collect();
                return CommandOutput::success(String::new());
            }
            "local" => {
                if let Some(scope) = self.local_saves.last_mut() {
                    for a in &args {
                        let (name, val) = match a.find('=') {
                            Some(eq) => (&a[..eq], Some(a[eq + 1..].to_string())),
                            None => (*a, None),
                        };
                        let old = self.shell.vars.get(name).cloned();
                        scope.push((name.to_string(), old));
                        match val {
                            Some(v) => {
                                self.shell.vars.insert(name.to_string(), v);
                            }
                            None => {
                                self.shell.vars.remove(name);
                            }
                        }
                    }
                }
                return CommandOutput::success(String::new());
            }
            _ => {}
        }

        // stdin: heredoc body takes precedence, then `< file`. Read `< file`
        // as BYTES so binary files reach binary-aware commands intact.
        let _ = self.shell.take_binary_in();
        let mut stdin_from_file: Option<String> = None;
        if heredoc_ref.is_none() {
            if let Some(path) = spec.stdin_file.as_ref() {
                if let Ok(bytes) = self.shell.vfs.read(path, &self.shell.cwd) {
                    if !bytes.is_empty() {
                        self.shell.set_binary_in(bytes.clone());
                    }
                    stdin_from_file = Some(String::from_utf8_lossy(&bytes).to_string());
                }
            }
        }
        let stdin_ref = heredoc_ref.or(stdin_from_file.as_deref());

        let (cmd, args_vec, is_alias) =
            if let Some((new_cmd, new_args)) = self.shell.resolve_alias(cmd, &args) {
                (new_cmd, new_args, true)
            } else {
                (
                    cmd.to_string(),
                    args.iter().map(|s| s.to_string()).collect(),
                    false,
                )
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
        // `env VAR=x cmd ...` is equivalent to `VAR=x cmd ...` (bash).
        if let Some(r) = rest.strip_prefix("env ") {
            let mut r = r.trim_start();
            // `env -u VAR` (repeatable): unset variables.
            while let Some(r2) = r.strip_prefix("-u ") {
                let r2 = r2.trim_start();
                let var_end = r2.find(char::is_whitespace).unwrap_or(r2.len());
                let var = &r2[..var_end];
                self.shell.vars.remove(var);
                r = r2[var_end..].trim_start();
            }
            // `env -i`: start with an empty environment.
            if let Some(r2) = r.strip_prefix("-i") {
                self.shell.vars.clear();
                r = r2.trim_start();
            }
            // Only strip when what follows is an assignment (or we already
            // processed -u/-i above); a bare `env` falls through to cmd_env.
            if take_assignment(r).is_some() || (r != input && !r.is_empty()) {
                rest = r;
            }
        }
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
            while let Some((name, value, after, append)) = take_assignment(cursor) {
                if self.shell.readonly.contains(&name) {
                    self.assign_error = Some(format!("{}: readonly variable\n", name));
                    return ("", true);
                }
                let value = if append {
                    self.shell.vars.get(&name).cloned().unwrap_or_default() + &value
                } else {
                    value
                };
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
        loop {
            if let Some((name, key, value, after, append)) = take_element_assignment(cursor) {
                // A name declared `declare -A` (or a non-numeric subscript)
                // selects an associative array; a numeric subscript on an
                // ordinary variable selects an indexed array.
                let is_assoc = self.shell.assoc.contains_key(&name)
                    || key.is_empty()
                    || !key
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false);
                if is_assoc {
                    let entry = self.shell.assoc.entry(name).or_default();
                    let newv = if append {
                        entry.get(&key).cloned().unwrap_or_default() + &value
                    } else {
                        value
                    };
                    entry.insert(key, newv);
                } else if let Ok(idx) = key.parse::<usize>() {
                    let arr = self.shell.arrays.entry(name).or_default();
                    while arr.len() <= idx {
                        arr.push(String::new());
                    }
                    arr[idx] = if append {
                        arr[idx].clone() + &value
                    } else {
                        value
                    };
                }
                consumed_any = true;
                cursor = after.trim_start();
                continue;
            }
            if let Some((name, items, after, append)) = take_array_assignment(cursor) {
                if append {
                    self.shell.arrays.entry(name).or_default().extend(items);
                } else {
                    self.shell.arrays.insert(name, items);
                }
                consumed_any = true;
                cursor = after.trim_start();
                continue;
            }
            if let Some((name, value, after, append)) = take_assignment(cursor) {
                let name = self.resolve_nameref(&name);
                if self.shell.readonly.contains(&name) {
                    self.assign_error = Some(format!("{}: readonly variable\n", name));
                    return ("", true);
                }
                let value = if self.shell.integer.contains(&name) {
                    self.eval_arith_command(&value).to_string()
                } else if append {
                    self.shell.vars.get(&name).cloned().unwrap_or_default() + &value
                } else {
                    value
                };
                self.shell.vars.insert(name, value);
                consumed_any = true;
                cursor = after.trim_start();
                continue;
            }
            break;
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
                self.execute_while(
                    input
                        .trim_start_matches("while ")
                        .trim_start_matches("until "),
                    cond_inverted,
                    heredoc_ref,
                )
            }
            "case" => self.execute_case(input.trim_start_matches("case ")),
            _ => CommandOutput::error(format!("unknown block: {first}\n"), 127),
        }
    }

    fn execute_if(&mut self, input: &str, _stdin: Option<&str>) -> CommandOutput {
        let s = input.trim();
        let fi_idx = match rfind_keyword(s, "fi") {
            Some(x) => x,
            None => return CommandOutput::error("if: missing fi\n".to_string(), 1),
        };
        let before_fi = s[..fi_idx].trim_end();
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
        let do_pos = match find_do(s) {
            Some(x) => x,
            None => return CommandOutput::error("for: missing do\n".to_string(), 1),
        };
        let after_do = s[do_pos + 2..].trim_start();
        let after_do = after_do.strip_prefix(';').unwrap_or(after_do).trim_start();
        let body = match rfind_keyword(after_do, "done") {
            Some(x) => after_do[..x].trim().to_string(),
            None => return CommandOutput::error("for: missing done\n".to_string(), 1),
        };

        let head = s[..do_pos].trim().trim_end_matches(';').trim().to_string();
        let (var, words, has_in) = if let Some(pos) = find_keyword(&head, "in") {
            (
                head[..pos].trim().trim_end_matches(';').trim().to_string(),
                head[pos + 2..].trim().to_string(),
                true,
            )
        } else {
            (head.clone(), String::new(), false)
        };

        let word_list: Vec<String> = if words.is_empty() {
            if has_in {
                // `for x in; do` iterates over nothing.
                Vec::new()
            } else if self.shell.positional.len() > 1 {
                // Bare `for x; do` iterates over the positional parameters.
                self.shell.positional[1..].to_vec()
            } else {
                self.shell
                    .vars
                    .get("@")
                    .cloned()
                    .map(|v| shell_words_parse(&v).unwrap_or_default())
                    .unwrap_or_default()
            }
        } else if matches!(words.trim(), "\"$@\"" | "$@" | "\"$*\"" | "$*") {
            // `for x in "$@"` iterates over the positional parameters as
            // separate words (bash semantics).
            if self.shell.positional.len() > 1 {
                self.shell.positional[1..].to_vec()
            } else {
                Vec::new()
            }
        } else if let Some(name) = array_words_name(words.trim()) {
            self.shell.arrays.get(name).cloned().unwrap_or_default()
        } else {
            // Block constructs are dispatched with the raw segment (before the
            // non-block `expand_line`), so expand the word list here: `$VAR`,
            // `$(...)`, backticks, `~` … Then tokenize (keeping quote info) and
            // glob-expand each unquoted word so `for f in *.jpg` iterates over
            // the matches.
            let words = self.expand_line(&words);
            self.expand_glob_words(parse_command(&words))
        };

        let mut last_exit = 0;
        let mut agg_stdout = String::new();
        let mut agg_stderr = String::new();
        for word in &word_list {
            if self.cancel.load(Ordering::SeqCst) {
                return CommandOutput {
                    stdout: agg_stdout,
                    stderr: format!("{}cancelled\n", agg_stderr),
                    exit_code: 143,
                };
            }
            self.shell.vars.insert(var.clone(), word.clone());
            let out = self.execute(&body);
            agg_stdout.push_str(&out.stdout);
            if !out.stderr.is_empty() {
                agg_stderr.push_str(&out.stderr);
            }
            last_exit = out.exit_code;
            if self.exit_signal.is_some() {
                break;
            }
            if self.break_levels > 0 {
                self.break_levels -= 1;
                break;
            }
            if self.continue_levels > 0 {
                self.continue_levels -= 1;
                if self.continue_levels > 0 {
                    break;
                }
                continue;
            }
        }
        // bash leaves the loop variable set to its last value after the loop.
        CommandOutput {
            stdout: agg_stdout,
            stderr: agg_stderr,
            exit_code: last_exit,
        }
    }

    fn execute_while(
        &mut self,
        input: &str,
        cond_inverted: bool,
        heredoc: Option<&str>,
    ) -> CommandOutput {
        let s = input.trim();
        let do_pos = match find_do(s) {
            Some(x) => x,
            None => return CommandOutput::error("while: missing do\n".to_string(), 1),
        };
        let condition = s[..do_pos].trim().trim_end_matches(';').trim().to_string();
        let after_do = s[do_pos + 2..].trim_start();
        let after_do = after_do.strip_prefix(';').unwrap_or(after_do).trim_start();
        let done_pos = match rfind_keyword(after_do, "done") {
            Some(x) => x,
            None => return CommandOutput::error("while: missing done\n".to_string(), 1),
        };
        let body = after_do[..done_pos].trim().to_string();
        // `while read VAR...; do ...; done < FILE` — iterate the input lines
        // (bash's canonical read loop) instead of re-running `read` against an
        // empty stdin forever.
        let tail = after_do[done_pos + 4..].trim().to_string();
        if let Some(out) = self.read_file_loop(&condition, &body, &tail, heredoc) {
            return out;
        }

        let mut last_exit = 0;
        let mut agg_stdout = String::new();
        let mut agg_stderr = String::new();
        let max_iters: usize = 100_000;
        for _ in 0..max_iters {
            if self.cancel.load(Ordering::SeqCst) {
                return CommandOutput {
                    stdout: agg_stdout,
                    stderr: format!("{}cancelled\n", agg_stderr),
                    exit_code: 143,
                };
            }
            let cond = self.execute(&condition);
            let ok = if cond_inverted {
                cond.exit_code != 0
            } else {
                cond.exit_code == 0
            };
            if !ok {
                break;
            }
            let out = self.execute(&body);
            agg_stdout.push_str(&out.stdout);
            if !out.stderr.is_empty() {
                agg_stderr.push_str(&out.stderr);
            }
            last_exit = out.exit_code;
            if self.exit_signal.is_some() {
                break;
            }
            if self.break_levels > 0 {
                self.break_levels -= 1;
                break;
            }
            if self.continue_levels > 0 {
                self.continue_levels -= 1;
                if self.continue_levels > 0 {
                    break;
                }
                continue;
            }
        }
        CommandOutput {
            stdout: agg_stdout,
            stderr: agg_stderr,
            exit_code: last_exit,
        }
    }

    /// Handle `PRODUCER | while read VAR...; do BODY; done` — the threaded
    /// pipeline path can't express a loop stage (loops need a shared stdin),
    /// so run the producer, then iterate its lines setting the read vars.
    /// Handles `while read VAR...; do BODY; done < FILE` (also `<<< WORD`) by
    /// iterating the input's lines and setting the vars each pass. Returns
    /// `None` when the shape doesn't match (falls back to the generic loop).
    fn read_file_loop(
        &mut self,
        condition: &str,
        body: &str,
        tail: &str,
        heredoc: Option<&str>,
    ) -> Option<CommandOutput> {
        let tail = tail.trim();
        if !tail.starts_with('<') && heredoc.is_none() {
            return None;
        }
        // `[IFS=x] read [-flags] VAR...`
        let cond = condition.trim();
        let cond = match cond.find(char::is_whitespace) {
            Some(sp) if cond[..sp].starts_with("IFS=") => cond[sp..].trim(),
            _ => cond,
        };
        let mut parts = cond.split_whitespace();
        if parts.next() != Some("read") {
            return None;
        }
        let vars: Vec<String> = parts
            .filter(|t| !t.starts_with('-'))
            .map(|t| t.to_string())
            .collect();
        if vars.is_empty() {
            return None;
        }

        let content = if let Some(h) = heredoc {
            h.to_string()
        } else if let Some(rest) = tail.strip_prefix("<<<") {
            format!("{}\n", rest.trim().trim_matches('"').trim_matches('\''))
        } else {
            let file = tail[1..].trim();
            match self.shell.vfs.read_to_string(file, &self.shell.cwd) {
                Ok(c) => c,
                Err(e) => {
                    return Some(CommandOutput::error(format!("while: {}: {}\n", file, e), 1))
                }
            }
        };

        let mut agg_stdout = String::new();
        let mut agg_stderr = String::new();
        let mut last_exit = 0;
        for line in content.lines() {
            if self.cancel.load(Ordering::SeqCst) {
                return Some(CommandOutput {
                    stdout: agg_stdout,
                    stderr: format!("{}cancelled\n", agg_stderr),
                    exit_code: 143,
                });
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            for (j, name) in vars.iter().enumerate() {
                let val = if j == vars.len() - 1 && fields.len() > j {
                    fields[j..].join(" ")
                } else if j < fields.len() {
                    fields[j].to_string()
                } else {
                    String::new()
                };
                self.shell.vars.insert(name.clone(), val);
            }
            let out = self.execute(body);
            agg_stdout.push_str(&out.stdout);
            if !out.stderr.is_empty() {
                agg_stderr.push_str(&out.stderr);
            }
            last_exit = out.exit_code;
            if self.exit_signal.is_some() {
                break;
            }
            if self.break_levels > 0 {
                self.break_levels -= 1;
                break;
            }
            if self.continue_levels > 0 {
                self.continue_levels -= 1;
                if self.continue_levels > 0 {
                    break;
                }
                continue;
            }
        }
        Some(CommandOutput {
            stdout: agg_stdout,
            stderr: agg_stderr,
            exit_code: last_exit,
        })
    }

    fn try_pipe_into_while(&mut self, raw: &str) -> Option<CommandOutput> {
        let pipe = find_top_level_pipe(raw)?;
        let left = raw[..pipe].trim();
        let right = raw[pipe + 1..].trim();
        if left.is_empty() {
            return None;
        }
        if !(right == "while" || right.starts_with("while ")) {
            return None;
        }
        let do_pos = find_do(right)?;
        let head = right[..do_pos].trim().trim_end_matches(';').trim();
        let head_rest = head.strip_prefix("while")?.trim_start();
        if !(head_rest == "read" || head_rest.starts_with("read ")) {
            return None;
        }
        let vars: Vec<String> = head_rest
            .trim_start_matches("read")
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        if vars.is_empty() {
            return None;
        }
        let after_do = right[do_pos + 2..].trim_start();
        let after_do = after_do.strip_prefix(';').unwrap_or(after_do).trim_start();
        let done_pos = rfind_keyword(after_do, "done")?;
        let body = after_do[..done_pos].trim();

        let produced = self.execute(left);
        if produced.exit_code != 0 && produced.stdout.is_empty() {
            return Some(produced);
        }

        let mut agg_stdout = String::new();
        let mut agg_stderr = String::new();
        let mut last_exit = 0;
        for line in produced.stdout.lines() {
            if self.cancel.load(Ordering::SeqCst) {
                return Some(CommandOutput {
                    stdout: agg_stdout,
                    stderr: format!("{}cancelled\n", agg_stderr),
                    exit_code: 143,
                });
            }
            // Emulate `read`: split on whitespace, last var gets the rest.
            let parts: Vec<&str> = line.split_whitespace().collect();
            let n = vars.len();
            for j in 0..n {
                let val = if j == n - 1 && parts.len() > j {
                    parts[j..].join(" ")
                } else if j < parts.len() {
                    parts[j].to_string()
                } else {
                    String::new()
                };
                self.shell.vars.insert(vars[j].clone(), val);
            }
            let out = self.execute(body);
            agg_stdout.push_str(&out.stdout);
            if !out.stderr.is_empty() {
                agg_stderr.push_str(&out.stderr);
            }
            last_exit = out.exit_code;
        }
        Some(CommandOutput {
            stdout: agg_stdout,
            stderr: agg_stderr,
            exit_code: last_exit,
        })
    }

    /// Handle `PRODUCER | { read VAR...; BODY; }` — the group reads one line
    /// from the producer, then runs the remaining statements once.
    fn try_pipe_into_group(&mut self, raw: &str) -> Option<CommandOutput> {
        let pipe = find_top_level_pipe(raw)?;
        let left = raw[..pipe].trim();
        let right = raw[pipe + 1..].trim();
        // Accept `{ ... }` and `( ... )` groups (not `(( ))` arithmetic).
        let is_group = (right.starts_with('{') && right.ends_with('}'))
            || (right.starts_with('(') && right.ends_with(')') && !right.starts_with("(("));
        if left.is_empty() || !is_group {
            return None;
        }
        let inner = right[1..right.len() - 1].trim();
        let (vars, body) = parse_leading_read(inner)?;
        if vars.is_empty() {
            return None;
        }
        let produced = self.execute(left);
        if produced.exit_code != 0 && produced.stdout.is_empty() {
            return Some(produced);
        }
        let line = produced.stdout.lines().next().unwrap_or("");
        let parts: Vec<&str> = line.split_whitespace().collect();
        let n = vars.len();
        for j in 0..n {
            let val = if j == n - 1 && parts.len() > j {
                parts[j..].join(" ")
            } else if j < parts.len() {
                parts[j].to_string()
            } else {
                String::new()
            };
            self.shell.vars.insert(vars[j].clone(), val);
        }
        Some(self.execute(body))
    }

    /// Handle `PRODUCER | python3 -c '...'` — feed the producer's stdout to the
    /// embedded Python as `sys.stdin` (via the `_py_stdin` file the RustPython
    /// wrapper opens), since the pipeline-stage path can't run Python.
    ///
    /// Only applies where subprocess is unavailable (mobile / RustPython). On
    /// desktop the normal pipeline path is used unchanged.
    /// `{ group; } | cmd` / `( subshell ) | cmd` — run the construct in-process
    /// (the pipeline-stage path can't handle block constructs) and pipe its
    /// stdout into the rest.
    fn try_construct_into_pipe(&mut self, raw: &str) -> Option<CommandOutput> {
        let pipe = find_top_level_pipe(raw)?;
        let left = raw[..pipe].trim();
        let right = raw[pipe + 1..].trim();
        if left.is_empty() || right.is_empty() {
            return None;
        }
        let is_group = left.starts_with('{') && left.ends_with('}');
        let is_subshell = left.starts_with('(') && left.ends_with(')') && !left.starts_with("((");
        if !is_group && !is_subshell {
            return None;
        }
        let produced = self.execute(left);
        if produced.exit_code != 0 && produced.stdout.is_empty() {
            return Some(produced);
        }
        let mut out = self.execute_pipeline(right, Some(&produced.stdout));
        if !produced.stderr.is_empty() {
            out.stderr = format!("{}{}", produced.stderr, out.stderr);
        }
        Some(out)
    }

    /// `sh`/`bash` compatibility wrapper. Supports `sh -c 'CMD' [args...]`,
    /// `sh SCRIPT [args...]`, and piping (`sh -c '...' | cat`). There is no
    /// external `sh` binary on mobile; the script runs through this shell so
    /// `#!/bin/sh` scripts and `sh -c` callers work.
    fn try_shell_script(&mut self, raw: &str) -> Option<CommandOutput> {
        let trimmed = raw.trim();
        let first = trimmed.split_whitespace().next()?;
        if first != "sh" && first != "bash" {
            return None;
        }
        let rest = trimmed[first.len()..].trim_start();
        let (head, tail) = match find_top_level_pipe(rest) {
            Some(p) => (&rest[..p], Some(rest[p + 1..].trim())),
            None => (rest, None),
        };
        let head = self.expand_line(head);
        // Strip redirects (`> file`, `2>&1`, …) so they apply to the `sh` command
        // itself instead of being mistaken for a script operand.
        let (toks_pt, spec) = self.extract_redirects(parse_command(&head));
        let toks: Vec<String> = toks_pt.into_iter().map(|t| t.value).collect();
        if toks.is_empty() {
            return Some(CommandOutput::error(
                "sh: interactive mode is not supported (use `sh -c CMD` or `sh FILE`)\n"
                    .to_string(),
                2,
            ));
        }

        // Skip leading option flags (e.g. `-e`, `-x`); `-c` takes the program.
        let mut idx = 0;
        let mut from_c = false;
        while idx < toks.len() && toks[idx].starts_with('-') && toks[idx].len() > 1 {
            if toks[idx] == "-c" {
                from_c = true;
                break;
            }
            idx += 1;
        }

        let (script, positional): (String, Vec<String>) = if from_c {
            if idx + 1 >= toks.len() {
                return Some(CommandOutput::error(
                    "sh: -c: option requires an argument\n".to_string(),
                    2,
                ));
            }
            let prog = toks[idx + 1].clone();
            // `sh -c CMD name arg1...`: name is $0, arg1 is $1 (bash semantics).
            let mut a = toks[idx + 2..].to_vec();
            if a.is_empty() {
                a.push("sh".to_string());
            }
            (prog, a)
        } else {
            if idx >= toks.len() {
                return Some(CommandOutput::error(
                    "sh: missing script operand\n".to_string(),
                    2,
                ));
            }
            let file = toks[idx].clone();
            let mut a = vec![file.clone()];
            a.extend(toks[idx + 1..].iter().cloned());
            match self.shell.vfs.read_to_string(&file, &self.shell.cwd) {
                Ok(s) => (s, a),
                Err(e) => return Some(CommandOutput::error(format!("sh: {}: {}\n", file, e), 127)),
            }
        };

        // Set positional params ($0=name, $1..=args) for the duration and run in
        // an isolated subshell so an `exit` in the program cannot kill the
        // parent script (bash runs `sh -c` in a child process).
        let saved = std::mem::replace(&mut self.shell.positional, positional);
        let mut out = self.execute_subshell(&script);
        self.shell.positional = saved;

        if let Some(t) = tail {
            if !t.is_empty() {
                let mut piped = self.execute_pipeline(t, Some(&out.stdout));
                if !out.stderr.is_empty() {
                    piped.stderr = format!("{}{}", out.stderr, piped.stderr);
                }
                out = piped;
            }
        }
        self.apply_redirects(&mut out, &spec);
        Some(out)
    }

    /// `PRODUCER | sh [-c CMD | FILE]` — run the shell stage in-process with the
    /// producer's stdout as its stdin (mobile has no external `sh` binary).
    fn try_pipe_into_shell(&mut self, raw: &str) -> Option<CommandOutput> {
        let pipe = find_top_level_pipe(raw)?;
        let left = raw[..pipe].trim();
        let right = raw[pipe + 1..].trim();
        if left.is_empty() || right.is_empty() {
            return None;
        }
        let first = right.split_whitespace().next()?;
        if first != "sh" && first != "bash" {
            return None;
        }
        let produced = self.execute(left);
        if produced.exit_code != 0 && produced.stdout.is_empty() {
            return Some(produced);
        }
        let rest = self.expand_line(right[first.len()..].trim_start());
        let (toks_pt, spec) = self.extract_redirects(parse_command(&rest));
        let toks: Vec<String> = toks_pt.into_iter().map(|t| t.value).collect();
        let has_c = toks.iter().any(|t| t == "-c");
        let has_operand = toks.iter().any(|t| !t.starts_with('-'));
        let mut out = if !has_c && !has_operand {
            // `... | sh` — the piped text is the program itself.
            self.execute(&produced.stdout)
        } else {
            let script = if has_c {
                let i = toks.iter().position(|t| t == "-c").unwrap();
                toks.get(i + 1).cloned().unwrap_or_default()
            } else {
                let f = toks
                    .iter()
                    .find(|t| !t.starts_with('-'))
                    .cloned()
                    .unwrap_or_default();
                self.shell
                    .vfs
                    .read_to_string(&f, &self.shell.cwd)
                    .unwrap_or_default()
            };
            self.execute_pipeline(&script, Some(&produced.stdout))
        };
        if !produced.stderr.is_empty() {
            out.stderr = format!("{}{}", produced.stderr, out.stderr);
        }
        self.apply_redirects(&mut out, &spec);
        Some(out)
    }

    /// `python3 ... | rest` — run the python stage in-process (so the pipe is
    /// not swallowed by `is_python_command`), then pipe its stdout into `rest`.
    fn try_python_into_pipe(&mut self, raw: &str) -> Option<CommandOutput> {
        let pipe = find_top_level_pipe(raw)?;
        let left = raw[..pipe].trim();
        let right = raw[pipe + 1..].trim();
        if left.is_empty() || right.is_empty() || !is_python_command(left) {
            return None;
        }
        let produced = self.execute(left);
        if produced.exit_code != 0 && produced.stdout.is_empty() {
            return Some(produced);
        }
        let mut out = self.execute_pipeline(right, Some(&produced.stdout));
        if !produced.stderr.is_empty() {
            out.stderr = format!("{}{}", produced.stderr, out.stderr);
        }
        Some(out)
    }

    fn try_pipe_into_python(&mut self, raw: &str) -> Option<CommandOutput> {
        let stages = split_top_level_pipes(raw);
        if stages.len() < 2 {
            return None;
        }
        // Locate the Python stage anywhere in the pipeline. Python at the head
        // is handled by `try_python_into_pipe`.
        let py_idx = stages.iter().position(|st| is_python_command(st))?;
        if py_idx == 0 {
            return None;
        }
        let prefix = stages[..py_idx].join(" | ");
        let suffix = if py_idx + 1 < stages.len() {
            Some(stages[py_idx + 1..].join(" | "))
        } else {
            None
        };
        let mut py_stage = stages[py_idx].clone();
        // `... | python3` (no `-`) — real Python reads the program from stdin,
        // so append `-` to route it through the stdin path instead of the REPL.
        {
            let toks: Vec<&str> = py_stage.split_whitespace().collect();
            let has_dash = toks.iter().any(|t| *t == "-");
            let has_script = toks
                .get(1..)
                .map(|r| r.iter().any(|t| !t.starts_with('-')))
                .unwrap_or(false);
            if !has_dash && !has_script {
                py_stage = format!("{py_stage} -");
            }
        }
        let produced = self.execute(&prefix);
        if produced.exit_code != 0 && produced.stdout.is_empty() {
            return Some(produced);
        }
        let Ok(real) = self.shell.vfs.resolve("_py_stdin", &self.shell.cwd) else {
            return None;
        };
        if std::fs::write(&real, &produced.stdout).is_err() {
            return None;
        }
        let mut out = match suffix {
            Some(s) if !s.is_empty() => {
                let py = self.execute(&py_stage);
                let mut o = self.execute_pipeline(&s, Some(&py.stdout));
                if !py.stderr.is_empty() {
                    o.stderr = format!("{}{}", py.stderr, o.stderr);
                }
                o
            }
            _ => self.execute(&py_stage),
        };
        let _ = std::fs::remove_file(&real);
        if !produced.stderr.is_empty() {
            out.stderr = format!("{}{}", produced.stderr, out.stderr);
        }
        Some(out)
    }

    fn execute_case(&mut self, input: &str) -> CommandOutput {
        let s = input.trim();
        let in_pos = match find_keyword(s, "in") {
            Some(p) => p,
            None => return CommandOutput::error("case: missing 'in'\n".to_string(), 1),
        };
        // The `case` word IS subject to expansion + quote removal
        // (`case $V in abc)` / `case "$V" in abc)` must both match `abc`).
        let word = {
            let raw = s[..in_pos].trim().trim_end_matches(';').trim();
            let expanded = self.expand_line(raw);
            strip_shell_quotes(expanded.trim())
        };
        let rest = s[in_pos + 2..].trim_start();
        let rest = rest.strip_prefix(';').unwrap_or(rest).trim_start();

        let patterns_str = match rfind_keyword(rest, "esac") {
            Some(x) => rest[..x].trim().to_string(),
            None => return CommandOutput::error("case: missing esac\n".to_string(), 1),
        };

        let mut i = 0usize;
        let chars: Vec<char> = patterns_str.chars().collect();
        let n = chars.len();
        while i < n {
            // Skip whitespace/newlines between branches.
            while i < n && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= n {
                break;
            }
            // Pattern terminator `)` — must be at top level and followed by
            // whitespace / `;` / end (not `)` inside a pattern).
            let pat_start = i;
            let mut dq = false;
            let mut sq = false;
            let mut paren_pos = None;
            while i < n {
                match chars[i] {
                    '"' => dq = !dq,
                    '\'' => sq = !sq,
                    ')' if !dq
                        && !sq
                        && (i + 1 >= n || chars[i + 1].is_whitespace() || chars[i + 1] == ';') =>
                    {
                        paren_pos = Some(i);
                        i += 1;
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
            let paren_pos = match paren_pos {
                Some(x) => x,
                None => break,
            };
            // Quote removal applies to case PATTERNS too (`*"world"*` → `*world*`),
            // and patterns undergo variable / command substitution (`*$V*`).
            let pattern = {
                let raw = patterns_str[pat_start..paren_pos].trim();
                self.expand_line(raw).replace(['"', '\''], "")
            };

            // Body runs until `;;` (a lone `;` is a command separator inside
            // the branch, not a terminator).
            let body_start = i;
            let mut dq = false;
            let mut sq = false;
            while i < n {
                match chars[i] {
                    '"' => dq = !dq,
                    '\'' => sq = !sq,
                    ';' if !dq && !sq && i + 1 < n && chars[i + 1] == ';' => {
                        i += 2;
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
            let body = patterns_str[body_start..i.min(n)]
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();

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
        self.local_saves.push(Vec::new());
        self.fn_call_depth += 1;
        let result = self.execute(body);
        self.fn_call_depth -= 1;
        // Restore any `local` declarations made in this call.
        if let Some(scope) = self.local_saves.pop() {
            for (var, old) in scope {
                match old {
                    Some(v) => {
                        self.shell.vars.insert(var, v);
                    }
                    None => {
                        self.shell.vars.remove(&var);
                    }
                }
            }
        }
        self.shell.positional = saved_positional;
        // `return N` unwinds the function; the signal is consumed here.
        if let Some(code) = self.return_code.take() {
            let stdout = result.stdout;
            let stderr = result.stderr;
            let _ = stdin;
            return CommandOutput {
                stdout,
                stderr,
                exit_code: code,
            };
        }
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
                    // Preserve the escape for the tokenizer, which marks the
                    // token literal: `\*` must not glob, `\"` stays a literal
                    // quote, `\$` must not expand. We still consume the escaped
                    // char here so it is not treated as a substitution trigger.
                    out.push('\\');
                    out.push(chars[i + 1]);
                    i += 2;
                }
                '`' => {
                    // Backtick command substitution.
                    if let Some(close) = chars[i + 1..].iter().position(|&ch| ch == '`') {
                        let inner: String = chars[i + 1..i + 1 + close].iter().collect();
                        let subst = self.run_substitution(&inner);
                        if in_double {
                            push_expansion_value(&mut out, &subst);
                        } else if i > 0 && chars[i - 1] == '=' {
                            push_assignment_value(&mut out, &subst);
                        } else {
                            let words: Vec<&str> = subst.split_whitespace().collect();
                            push_expansion_value(&mut out, &words.join(" "));
                        }
                        i += close + 2;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '$' if i + 1 < n && chars[i + 1] == '\'' && !in_double => {
                    // `$'...'` ANSI-C quoting. Inside double quotes `$'` is a
                    // literal `$` + quote (bash), so it must not be decoded.
                    let mut j = i + 2;
                    let mut s = String::new();
                    while j < n && chars[j] != '\'' {
                        if chars[j] == '\\' && j + 1 < n {
                            j += 1;
                            let e = chars[j];
                            match e {
                                'n' => s.push('\n'),
                                't' => s.push('\t'),
                                'r' => s.push('\r'),
                                '\\' => s.push('\\'),
                                '\'' => s.push('\''),
                                '"' => s.push('"'),
                                'a' => s.push('\x07'),
                                'b' => s.push('\x08'),
                                'f' => s.push('\x0c'),
                                'v' => s.push('\x0b'),
                                'e' => s.push('\x1b'),
                                'x' => {
                                    let mut hex = String::new();
                                    while j + 1 < n
                                        && hex.len() < 2
                                        && chars[j + 1].is_ascii_hexdigit()
                                    {
                                        j += 1;
                                        hex.push(chars[j]);
                                    }
                                    if let Ok(v) = u8::from_str_radix(&hex, 16) {
                                        s.push(v as char);
                                    }
                                }
                                '0'..='7' => {
                                    let mut oct = String::new();
                                    oct.push(e);
                                    while j + 1 < n
                                        && oct.len() < 3
                                        && ('0'..='7').contains(&chars[j + 1])
                                    {
                                        j += 1;
                                        oct.push(chars[j]);
                                    }
                                    if let Ok(v) = u8::from_str_radix(&oct, 8) {
                                        s.push(v as char);
                                    }
                                }
                                other => {
                                    s.push('\\');
                                    s.push(other);
                                }
                            }
                            j += 1;
                        } else {
                            s.push(chars[j]);
                            j += 1;
                        }
                    }
                    if j < n {
                        j += 1;
                    }
                    if i > 0 && chars[i - 1] == '=' {
                        push_assignment_value(&mut out, &s);
                    } else {
                        // Keep it a single word through tokenization: the
                        // decoded value may contain whitespace/control chars.
                        out.push('"');
                        for ch in s.chars() {
                            match ch {
                                '"' => out.push_str("\\\""),
                                '\\' => out.push_str("\\\\"),
                                '$' => out.push_str("\\$"),
                                '`' => out.push_str("\\`"),
                                _ => out.push(ch),
                            }
                        }
                        out.push('"');
                    }
                    i = j;
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
                        // `$((N=9))` applies the assignment (like `((N=9))`),
                        // not just evaluates it.
                        out.push_str(&self.eval_arith_command(&expanded_inner).to_string());
                        i = end + 2;
                    } else {
                        out.push(c);
                        i += 1;
                    }
                }
                '$' if i + 1 < n && chars[i + 1] == '(' => {
                    // $( ... ) with nesting support. Quote-aware: a `)` inside
                    // quotes (e.g. `$(grep -oE '\(a\)' f)`) must NOT terminate
                    // the substitution early.
                    let mut depth = 0usize;
                    let mut j = i + 1;
                    let mut end = None;
                    let mut q_single = false;
                    let mut q_double = false;
                    // `case ... esac` inside `$(...)`: the `)` that ends a case
                    // pattern must NOT terminate the substitution.
                    let mut case_depth = 0usize;
                    while j < n {
                        let cj = chars[j];
                        if q_single {
                            if cj == '\'' {
                                q_single = false;
                            }
                        } else if q_double {
                            if cj == '"' {
                                q_double = false;
                            } else if cj == '\\' && j + 1 < n {
                                j += 1;
                            }
                        } else if starts_word(&chars, j, "case") {
                            case_depth += 1;
                        } else if case_depth > 0 && starts_word(&chars, j, "esac") {
                            case_depth -= 1;
                        } else {
                            match cj {
                                '\'' => q_single = true,
                                '"' => q_double = true,
                                '\\' if j + 1 < n => j += 1,
                                '(' => depth += 1,
                                ')' => {
                                    if case_depth == 0 {
                                        depth -= 1;
                                        if depth == 0 {
                                            end = Some(j);
                                            break;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        j += 1;
                    }
                    if let Some(end) = end {
                        let inner: String = chars[i + 2..end].iter().collect();
                        let subst = self.run_substitution(&inner);
                        if in_double {
                            push_expansion_value(&mut out, &subst);
                        } else if i > 0 && chars[i - 1] == '=' {
                            push_assignment_value(&mut out, &subst);
                        } else {
                            let words: Vec<&str> = subst.split_whitespace().collect();
                            push_expansion_value(&mut out, &words.join(" "));
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
                        let val = self.expand_param(&body);
                        if i > 0 && chars[i - 1] == '=' {
                            push_assignment_value(&mut out, &val);
                        } else if !in_double {
                            let ifs = ifs_value(&self.shell.vars);
                            if ifs_is_default(&ifs) {
                                push_expansion_value(&mut out, &val);
                            } else {
                                push_ifs_words(&mut out, &val, &ifs);
                            }
                        } else {
                            push_expansion_value(&mut out, &val);
                        }
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
                '$' if i + 1 < n && chars[i + 1] == '!' => {
                    // PID of the most recent background (`&`) job; empty when
                    // none has run (like bash with no background job).
                    out.push_str(&self.shell.last_bg_pid);
                    i += 2;
                }
                '$' if i + 1 < n && chars[i + 1] == '#' => {
                    let count = self.shell.positional.len().saturating_sub(1);
                    out.push_str(&count.to_string());
                    i += 2;
                }
                '$' if i + 1 < n && (chars[i + 1] == '@' || chars[i + 1] == '*') => {
                    let all = if self.shell.positional.len() > 1 {
                        self.shell.positional[1..].join(" ")
                    } else {
                        String::new()
                    };
                    if i > 0 && chars[i - 1] == '=' {
                        push_assignment_value(&mut out, &all);
                    } else {
                        push_expansion_value(&mut out, &all);
                    }
                    i += 2;
                }
                '$' if i + 1 < n && chars[i + 1].is_ascii_digit() => {
                    let d = chars[i + 1].to_digit(10).unwrap() as usize;
                    if d < self.shell.positional.len() {
                        let val = self.shell.positional[d].clone();
                        if i > 0 && chars[i - 1] == '=' {
                            push_assignment_value(&mut out, &val);
                        } else {
                            push_expansion_value(&mut out, &val);
                        }
                    }
                    i += 2;
                }
                '$' if i + 1 < n && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_') => {
                    let mut j = i + 1;
                    while j < n && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    let name: String = chars[i + 1..j].iter().collect();
                    let val = self.lookup_var(&name);
                    if i > 0 && chars[i - 1] == '=' {
                        push_assignment_value(&mut out, &val);
                    } else if !in_double {
                        let ifs = ifs_value(&self.shell.vars);
                        if ifs_is_default(&ifs) {
                            push_expansion_value(&mut out, &val);
                        } else {
                            push_ifs_words(&mut out, &val, &ifs);
                        }
                    } else {
                        push_expansion_value(&mut out, &val);
                    }
                    i = j;
                }
                '~' if !in_double => {
                    let at_start = i == 0
                        || chars[i - 1].is_whitespace()
                        || chars[i - 1] == ':'
                        // `X=~/path` is an assignment; `=~` is the regex operator.
                        || (chars[i - 1] == '='
                            && i >= 2
                            && (chars[i - 2].is_ascii_alphanumeric() || chars[i - 2] == '_'));
                    let next_ok = i + 1 >= n || chars[i + 1] == '/' || chars[i + 1].is_whitespace();
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
    /// returns its stdout with only trailing newlines stripped. Internal
    /// newlines are preserved (bash semantics) — unquoted uses word-split at
    /// the call site, quoted uses keep the newlines.
    fn run_substitution(&mut self, inner: &str) -> String {
        if self.subst_depth >= 8 {
            return String::new();
        }
        self.subst_depth += 1;
        let result = self.execute(inner);
        self.subst_depth -= 1;
        result.stdout.trim_end_matches('\n').to_string()
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
        // `${!name}` indirect expansion: value of the variable whose name is
        // the value of `name`. `${!arr[@]}` yields the array's indices/keys.
        if let Some(inner) = body.strip_prefix('!') {
            if let Some(name) = inner
                .strip_suffix("[@]")
                .or_else(|| inner.strip_suffix("[*]"))
            {
                if let Some(m) = self.shell.assoc.get(name) {
                    let mut keys: Vec<&String> = m.keys().collect();
                    keys.sort();
                    return keys
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Some(v) = self.shell.arrays.get(name) {
                    return (0..v.len())
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                return String::new();
            }
            if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                let target = self.lookup_var(inner);
                if !target.is_empty() {
                    return self.lookup_var(&target);
                }
            }
        }
        // Arrays: `${#a[@]}`, `${a[@]}` / `${a[*]}`, `${a[N]}`, assoc `${m[k]}`.
        if let Some(rest) = body.strip_prefix('#') {
            if let Some(name) = rest
                .strip_suffix("[@]")
                .or_else(|| rest.strip_suffix("[*]"))
            {
                if let Some(m) = self.shell.assoc.get(name) {
                    return m.len().to_string();
                }
                return self
                    .shell
                    .arrays
                    .get(name)
                    .map(|v| v.len())
                    .unwrap_or(0)
                    .to_string();
            }
        }
        if let Some(name) = body
            .strip_suffix("[@]")
            .or_else(|| body.strip_suffix("[*]"))
        {
            if let Some(m) = self.shell.assoc.get(name) {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                return keys
                    .iter()
                    .map(|k| m[*k].clone())
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            return self
                .shell
                .arrays
                .get(name)
                .map(|v| v.join(" "))
                .unwrap_or_default();
        }
        // Array slice: `${a[@]:OFF[:LEN]}` / `${a[*]:OFF[:LEN]}`.
        for marker in ["[@]:", "[*]:"] {
            if let Some(pos) = body.find(marker) {
                let name = &body[..pos];
                let spec = &body[pos + marker.len()..];
                let (o, l) = match spec.split_once(':') {
                    Some((o, l)) => (o.trim().parse::<i64>().ok(), l.trim().parse::<usize>().ok()),
                    None => (spec.trim().parse::<i64>().ok(), None),
                };
                if let Some(off) = o {
                    if let Some(v) = self.shell.arrays.get(name) {
                        let n = v.len() as i64;
                        let start = if off >= 0 {
                            (off as usize).min(v.len())
                        } else {
                            n.saturating_sub((-off) as i64) as usize
                        };
                        let slice: Vec<String> = v[start..].iter().cloned().collect();
                        let out: Vec<String> = match l {
                            Some(len) => slice.into_iter().take(len).collect(),
                            None => slice,
                        };
                        return out.join(" ");
                    }
                }
            }
        }
        if body.ends_with(']') {
            if let Some(open) = body.rfind('[') {
                let name = &body[..open];
                let idx = &body[open + 1..body.len() - 1];
                if let Some(m) = self.shell.assoc.get(name) {
                    return m.get(idx).cloned().unwrap_or_default();
                }
                if let Ok(i) = idx.parse::<usize>() {
                    return self
                        .shell
                        .arrays
                        .get(name)
                        .and_then(|v| v.get(i))
                        .cloned()
                        .unwrap_or_default();
                }
            }
        }
        for &op in &[":-", ":=", ":+", ":?"] {
            if let Some(pos) = body.find(op) {
                let name = &body[..pos];
                let word = &body[pos + 2..];
                let val = self.lookup_var(name);
                let is_set = self.shell.vars.contains_key(name);
                let is_empty = val.is_empty();
                match op {
                    ":-" => {
                        return if is_set && !is_empty {
                            val
                        } else {
                            self.expand_word(word)
                        }
                    }
                    ":=" => {
                        if !is_set || is_empty {
                            let w = self.expand_word(word);
                            self.shell.vars.insert(name.to_string(), w.clone());
                            return w;
                        }
                        return val;
                    }
                    ":+" => {
                        return if is_set && !is_empty {
                            self.expand_word(word)
                        } else {
                            String::new()
                        }
                    }
                    ":?" => {
                        if !is_set || is_empty {
                            let w = self.expand_word(word);
                            let msg = if w.is_empty() {
                                "parameter null or not set"
                            } else {
                                &w
                            };
                            return format!("{}: {}", name, msg);
                        }
                        return val;
                    }
                    _ => {}
                }
            }
        }
        // Non-colon default/assign/alternate/error operators: `${v-w}`,
        // `${v=w}`, `${v+w}`, `${v?w}` — trigger only when the parameter is
        // unset (empty-but-set values are kept, unlike the `:` forms).
        for &op in &["-", "=", "+", "?"] {
            if let Some(pos) = body.find(op) {
                let name = &body[..pos];
                let word = &body[pos + 1..];
                if !is_var_name(name) {
                    continue;
                }
                let is_set = self.shell.vars.contains_key(name);
                match op {
                    "-" => {
                        return if is_set {
                            self.lookup_var(name)
                        } else {
                            self.expand_word(word)
                        };
                    }
                    "=" => {
                        if !is_set {
                            let w = self.expand_word(word);
                            self.shell.vars.insert(name.to_string(), w.clone());
                            return w;
                        }
                        return self.lookup_var(name);
                    }
                    "+" => {
                        return if is_set {
                            self.expand_word(word)
                        } else {
                            String::new()
                        };
                    }
                    "?" => {
                        if !is_set {
                            let w = self.expand_word(word);
                            let msg = if w.is_empty() {
                                "parameter null or not set"
                            } else {
                                &w
                            };
                            return format!("{}: {}", name, msg);
                        }
                        return self.lookup_var(name);
                    }
                    _ => {}
                }
            }
        }
        // `${v^^}` / `${v,,}` / `${v^}` / `${v,}` — case conversion.
        if let Some(name) = body.strip_suffix("^^") {
            return self.lookup_var(name).to_uppercase();
        }
        if let Some(name) = body.strip_suffix(",,") {
            return self.lookup_var(name).to_lowercase();
        }
        if let Some(name) = body.strip_suffix('^') {
            if !name.is_empty() {
                let v = self.lookup_var(name);
                return match v.chars().next() {
                    Some(c) => c.to_uppercase().collect::<String>() + &v[c.len_utf8()..],
                    None => v,
                };
            }
        }
        if let Some(name) = body.strip_suffix(',') {
            if !name.is_empty() {
                let v = self.lookup_var(name);
                return match v.chars().next() {
                    Some(c) => c.to_lowercase().collect::<String>() + &v[c.len_utf8()..],
                    None => v,
                };
            }
        }
        if body.starts_with('#') && body.len() > 1 {
            let name = &body[1..];
            let val = self.lookup_var(name);
            return val.chars().count().to_string();
        }
        if let Some(pos) = body.find("##") {
            if !body[..pos].contains('/') {
                let name = &body[..pos];
                let pat = &body[pos + 2..];
                let val = self.lookup_var(name);
                if let Some(r) = glob_remove_prefix(&val, pat, true) {
                    return r;
                }
                return val;
            }
        }
        if let Some(pos) = body.find('#') {
            if pos > 0 && !body.starts_with('#') && !body[..pos].contains('/') {
                let name = &body[..pos];
                let pat = &body[pos + 1..];
                let val = self.lookup_var(name);
                if let Some(r) = glob_remove_prefix(&val, pat, false) {
                    return r;
                }
                return val;
            }
        }
        if let Some(pos) = body.rfind("%%") {
            if !body[..pos].contains('/') {
                let name = &body[..pos];
                let pat = &body[pos + 2..];
                let val = self.lookup_var(name);
                if let Some(r) = glob_remove_suffix(&val, pat, true) {
                    return r;
                }
                return val;
            }
        }
        if let Some(pos) = body.rfind('%') {
            if pos > 0 && !body[..pos].contains('/') {
                let name = &body[..pos];
                let pat = &body[pos + 1..];
                let val = self.lookup_var(name);
                if let Some(r) = glob_remove_suffix(&val, pat, false) {
                    return r;
                }
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
                // `${v/#pat/rep}` (anchor at start) / `${v/%pat/rep}` (at end).
                let (rest, anchor) = if let Some(r) = rest.strip_prefix('#') {
                    (r, Some(true))
                } else if let Some(r) = rest.strip_prefix('%') {
                    (r, Some(false))
                } else {
                    (rest, None)
                };
                let (pat, rep) = split_unescaped_slash(rest);
                let pat = unescape_param_chars(&pat);
                let rep = unescape_param_chars(&rep);
                let val = self.lookup_var(name);
                match anchor {
                    Some(true) => {
                        if let Some(remainder) = glob_remove_prefix(&val, &pat, false) {
                            return format!("{}{}", rep, remainder);
                        }
                        return val;
                    }
                    Some(false) => {
                        if let Some(remainder) = glob_remove_suffix(&val, &pat, false) {
                            return format!("{}{}", remainder, rep);
                        }
                        return val;
                    }
                    None => {
                        if global {
                            return val.replace(&pat, &rep);
                        } else {
                            return val.replacen(&pat, &rep, 1);
                        }
                    }
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
                    if let (Ok(offset), Ok(len)) =
                        (o_str.trim().parse::<i64>(), l_str.trim().parse::<usize>())
                    {
                        let val = self.lookup_var(name);
                        return substr(&val, offset, Some(len));
                    }
                }
                if let Ok(offset) = rest.trim().parse::<i64>() {
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

    /// xorshift PRNG step (for `$RANDOM`).
    fn next_random(&self) -> u64 {
        let mut x = self.rand_state.get();
        if x == 0 {
            x = 0x9E37_79B9_7F4A_7C15;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rand_state.set(x);
        x
    }

    /// Follow `declare -n` nameref chains (bounded) to the real variable.
    fn resolve_nameref(&self, name: &str) -> String {
        let mut cur = name.to_string();
        let mut depth = 0;
        while let Some(target) = self.shell.namerefs.get(&cur) {
            if depth > 16 {
                break;
            }
            cur = target.clone();
            depth += 1;
        }
        cur
    }

    fn lookup_var(&self, name: &str) -> String {
        let resolved = self.resolve_nameref(name);
        let name: &str = &resolved;
        if let Some(v) = self.shell.vars.get(name) {
            return v.clone();
        }
        // A bare array name expands to its first element (`$a` → `a[0]`).
        if let Some(first) = self.shell.arrays.get(name).and_then(|v| v.first()) {
            return first.clone();
        }
        match name {
            "PWD" => self.shell.cwd.clone(),
            "HOME" => self.home_dir(),
            "?" => self.last_exit.to_string(),
            // bash-compatible identifiers so scripts probing these work.
            "BASH_VERSION" => "5.2.15(1)-release".to_string(),
            "RANDOM" => (self.next_random() % 32768).to_string(),
            "SECONDS" => self.start_time.elapsed().as_secs().to_string(),
            "BASH" => "/bin/bash".to_string(),
            "SHELL" => "/bin/bash".to_string(),
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
        self.shell
            .vars
            .get("HOME")
            .cloned()
            .unwrap_or_else(|| "/".to_string())
    }
}

// ── Arithmetic expansion: $(( expr )) ─────────────────────────────────

/// Byte offset of a standalone shell keyword (e.g. `do`, `done`) in `s`.
/// A keyword is standalone when the preceding char is a separator
/// (whitespace / `;`) or the start, and the following char is a separator or end.
fn find_keyword(s: &str, kw: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut from = 0;
    while let Some(p) = s[from..].find(kw) {
        let pos = from + p;
        let before_ok = pos == 0 || matches!(bytes[pos - 1], b' ' | b'\t' | b'\n' | b'\r' | b';');
        let end = pos + kw.len();
        let after_ok = end >= s.len() || matches!(bytes[end], b' ' | b'\t' | b'\n' | b'\r' | b';');
        if before_ok && after_ok {
            return Some(pos);
        }
        from = pos + kw.len();
    }
    None
}

/// Last standalone occurrence of `kw`.
fn rfind_keyword(s: &str, kw: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut best = None;
    let mut from = 0;
    while let Some(p) = s[from..].find(kw) {
        let pos = from + p;
        let before_ok = pos == 0 || matches!(bytes[pos - 1], b' ' | b'\t' | b'\n' | b'\r' | b';');
        let end = pos + kw.len();
        let after_ok = end >= s.len() || matches!(bytes[end], b' ' | b'\t' | b'\n' | b'\r' | b';');
        if before_ok && after_ok {
            best = Some(pos);
        }
        from = pos + kw.len();
    }
    best
}

/// Offset of the `do` keyword terminating a `for`/`while` header.
/// Prefers the unambiguous `; do` form so a word-list item literally named `do`
/// can't confuse it; accepts any whitespace/newline/`;`/end after `do` (fixes
/// `...; do\n ... \ndone`, which the old `"; do "` search rejected).
fn find_do(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut from = 0;
    while let Some(p) = s[from..].find(';') {
        let semi = from + p;
        let after = s[semi + 1..].trim_start();
        if after.starts_with("do") {
            let at = semi + 1 + (s[semi + 1..].len() - after.len());
            let end = at + 2;
            if end >= s.len() || matches!(bytes[end], b' ' | b'\t' | b'\n' | b'\r' | b';') {
                return Some(at);
            }
        }
        from = semi + 1;
    }
    find_keyword(s, "do")
}

/// If `s` starts with `kw` as a standalone word, return the remainder (trimmed).
fn strip_leading_word<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let s = s.trim_start();
    let rest = s.strip_prefix(kw)?;
    let boundary =
        rest.is_empty() || matches!(rest.as_bytes()[0], b' ' | b'\t' | b'\n' | b'\r' | b';');
    boundary.then(|| rest.trim_start())
}

/// Earliest standalone occurrence among `kws`, with which keyword matched.
/// A general shell-parsing helper (kept for reuse even if a given path doesn't
/// call it today).
#[allow(dead_code)]
fn find_first_keyword<'a>(s: &str, kws: &[&'a str]) -> Option<(usize, &'a str)> {
    let mut best: Option<(usize, &'a str)> = None;
    for kw in kws {
        if let Some(p) = find_keyword(s, kw) {
            if best.map_or(true, |(bp, _)| p < bp) {
                best = Some((p, kw));
            }
        }
    }
    best
}

/// Find the next top-level `elif` / `else` / `fi` in `s`, skipping over nested
/// `if ... fi` blocks so an `if` inside a branch body is not mistaken for the
/// branch terminator.
fn find_if_terminator(s: &str) -> Option<(usize, &'static str)> {
    let mut depth = 0usize;
    let mut i = 0;
    while i < s.len() {
        let mut best: Option<(usize, &'static str)> = None;
        for kw in ["if", "fi", "elif", "else"] {
            if let Some(p) = find_keyword(&s[i..], kw) {
                let abs = i + p;
                if best.map_or(true, |(bp, _)| abs < bp) {
                    best = Some((abs, kw));
                }
            }
        }
        let Some((pos, kw)) = best else { break };
        match kw {
            "if" => {
                depth += 1;
                i = pos + 2;
            }
            "fi" => {
                if depth == 0 {
                    return Some((pos, "fi"));
                }
                depth -= 1;
                i = pos + 2;
            }
            "elif" | "else" => {
                if depth == 0 {
                    return Some((pos, kw));
                }
                i = pos + kw.len();
            }
            _ => i = pos + 1,
        }
    }
    None
}

/// Byte offset of the last top-level `|` (not `||`, not inside quotes).
fn find_top_level_pipe(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut last = None;
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' if !in_single && !in_double => depth += 1,
            b')' if !in_single && !in_double => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b'|' if !in_single && !in_double && depth == 0 => {
                if i + 1 < b.len() && b[i + 1] == b'|' {
                    i += 2;
                    continue;
                }
                last = Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    last
}

/// Split on top-level `|` (quote-aware; `||` is not a pipe).
fn split_top_level_pipes(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut stages = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'(' if !in_single && !in_double => depth += 1,
            b')' if !in_single && !in_double => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b'|' if !in_single && !in_double && depth == 0 => {
                if i + 1 < b.len() && b[i + 1] == b'|' {
                    i += 2;
                    continue;
                }
                stages.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    stages.push(s[start..].trim().to_string());
    stages
}

/// True if the segment's last pipe stage begins a `while` loop
/// (e.g. `cat f | while read x`).
fn ends_with_pipe_while(text: &str) -> bool {
    match find_top_level_pipe(text) {
        Some(p) => {
            let after = text[p + 1..].trim_start();
            after == "while" || after.starts_with("while ")
        }
        None => false,
    }
}

/// Emit a substitution result as a single assignment value. bash does not
/// word-split expansions in assignment context (`X=$(cmd)` keeps spaces), so we
/// wrap the result in double quotes (escaping) to preserve it as one token.
/// Insert an expansion result so the tokenizer treats it as literal text:
/// quotes/backslashes/dollars inside a variable's value are data, not syntax.
/// (Word-splitting on whitespace and globbing still apply where unquoted.)
/// The effective `IFS` (defaults to space/tab/newline when unset).
fn ifs_value(vars: &std::collections::HashMap<String, String>) -> String {
    vars.get("IFS")
        .cloned()
        .unwrap_or_else(|| " \t\n".to_string())
}

fn ifs_is_default(ifs: &str) -> bool {
    ifs == " \t\n" || ifs == " \t\n\r" || ifs == " \t"
}

/// Push an unquoted variable value as IFS-separated words: split on the current
/// `IFS`, drop empty fields, and re-emit each field double-quoted (so spaces
/// survive tokenization) separated by spaces. Used only when IFS is non-default.
fn push_ifs_words(out: &mut String, val: &str, ifs: &str) {
    let seps: Vec<char> = ifs.chars().collect();
    let mut first = true;
    if seps.is_empty() {
        out.push('"');
        push_expansion_value(out, val);
        out.push('"');
        return;
    }
    for f in val.split(|c| seps.contains(&c)) {
        if f.is_empty() {
            continue;
        }
        if !first {
            out.push(' ');
        }
        first = false;
        out.push('"');
        push_expansion_value(out, f);
        out.push('"');
    }
}

fn push_expansion_value(out: &mut String, val: &str) {
    for ch in val.chars() {
        match ch {
            '\\' | '"' | '\'' | '`' | '$' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
}

fn push_assignment_value(out: &mut String, subst: &str) {
    out.push('"');
    for ch in subst.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

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
            b'(' | b')' | b'?' | b':' | b'~' | b'+' | b'-' | b'/' | b'%' | b'^' | b'&' | b'|' => {
                tokens.push(expr[i..i + 1].to_string());
                i += 1;
            }
            b'*' => {
                if i + 1 < n && bytes[i + 1] == b'*' {
                    tokens.push("**".to_string());
                    i += 2;
                } else {
                    tokens.push("*".to_string());
                    i += 1;
                }
            }
            b'!' => {
                if i + 1 < n && bytes[i + 1] == b'=' {
                    tokens.push(expr[i..i + 2].to_string());
                    i += 2;
                } else {
                    tokens.push(expr[i..i + 1].to_string());
                    i += 1;
                }
            }
            b'=' => {
                if i + 1 < n && bytes[i + 1] == b'=' {
                    tokens.push(expr[i..i + 2].to_string());
                    i += 2;
                } else {
                    tokens.push(expr[i..i + 1].to_string());
                    i += 1;
                }
            }
            b'<' => {
                if i + 1 < n && bytes[i + 1] == b'<' {
                    tokens.push(expr[i..i + 2].to_string());
                    i += 2;
                } else if i + 1 < n && bytes[i + 1] == b'=' {
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
                } else if i + 1 < n && bytes[i + 1] == b'=' {
                    tokens.push(expr[i..i + 2].to_string());
                    i += 2;
                } else {
                    tokens.push(expr[i..i + 1].to_string());
                    i += 1;
                }
            }
            b'0'..=b'9' => {
                let start = i;
                if bytes[i] == b'0' && i + 1 < n && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                    i += 2;
                    while i < n && bytes[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                } else {
                    while i < n && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    // `base#number` literals (e.g. `16#ff`, `2#101`).
                    if i < n && bytes[i] == b'#' {
                        i += 1;
                        while i < n && bytes[i].is_ascii_alphanumeric() {
                            i += 1;
                        }
                    }
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
            if cond != 0 {
                true_val
            } else {
                false_val
            }
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
        let mut left = self.parse_equality();
        while self.peek() == Some("&") {
            self.advance();
            left &= self.parse_equality();
        }
        left
    }

    fn parse_equality(&mut self) -> i64 {
        let mut left = self.parse_relational();
        loop {
            match self.peek() {
                Some("==") => {
                    self.advance();
                    left = (left == self.parse_relational()) as i64;
                }
                Some("!=") => {
                    self.advance();
                    left = (left != self.parse_relational()) as i64;
                }
                _ => break,
            }
        }
        left
    }

    fn parse_relational(&mut self) -> i64 {
        let mut left = self.parse_shift();
        loop {
            match self.peek() {
                Some("<") => {
                    self.advance();
                    left = (left < self.parse_shift()) as i64;
                }
                Some("<=") => {
                    self.advance();
                    left = (left <= self.parse_shift()) as i64;
                }
                Some(">") => {
                    self.advance();
                    left = (left > self.parse_shift()) as i64;
                }
                Some(">=") => {
                    self.advance();
                    left = (left >= self.parse_shift()) as i64;
                }
                _ => break,
            }
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
        let mut left = self.parse_pow();
        loop {
            match self.peek() {
                Some("*") => {
                    self.advance();
                    left = left.wrapping_mul(self.parse_pow());
                }
                Some("/") => {
                    self.advance();
                    let rhs = self.parse_pow();
                    left = if rhs == 0 { 0 } else { left / rhs };
                }
                Some("%") => {
                    self.advance();
                    let rhs = self.parse_pow();
                    left = if rhs == 0 { 0 } else { left % rhs };
                }
                _ => break,
            }
        }
        left
    }

    /// `**` exponentiation: binds tighter than unary minus (bash: `-2**2` == 4)
    /// and is right-associative (`2**3**2` == 512).
    fn parse_pow(&mut self) -> i64 {
        let base = self.parse_unary();
        if self.peek() == Some("**") {
            self.advance();
            let exp = self.parse_pow();
            if exp < 0 {
                return 0;
            }
            let mut result: i64 = 1;
            let mut b = base;
            let mut e = exp as u64;
            while e > 0 {
                if e & 1 == 1 {
                    result = result.wrapping_mul(b);
                }
                e >>= 1;
                if e > 0 {
                    b = b.wrapping_mul(b);
                }
            }
            return result;
        }
        base
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
                if self.parse_unary() != 0 {
                    0
                } else {
                    1
                }
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
        if let Some(n) = parse_int_literal(name) {
            return n;
        }
        let val = self.rt.lookup_var(name);
        parse_int_literal(&val).unwrap_or(0)
    }
}

/// Parse an integer literal the way bash arithmetic does: decimal, `0x`/`0X`
/// hex, leading-`0` octal, and `base#number` (base 2..=36).
fn parse_int_literal(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        let n = i64::from_str_radix(hex, 16).ok()?;
        return Some(if neg { -n } else { n });
    }
    if let Some(hash) = s.find('#') {
        let base: u32 = s[..hash].trim().parse().ok()?;
        if !(2..=36).contains(&base) {
            return None;
        }
        let n = i64::from_str_radix(s[hash + 1..].trim(), base).ok()?;
        return Some(if neg { -n } else { n });
    }
    if s.len() > 1 && s.starts_with('0') && s.chars().all(|c| ('0'..='7').contains(&c)) {
        let n = i64::from_str_radix(&s[1..], 8).ok()?;
        return Some(if neg { -n } else { n });
    }
    let n = s.parse::<i64>().ok()?;
    Some(if neg { -n } else { n })
}

impl Runtime {
    fn eval_arithmetic(&self, expr: &str) -> i64 {
        let tokens = tokenize_arithmetic(expr);
        let mut parser = ArithParser {
            tokens: &tokens,
            pos: 0,
            rt: self,
        };
        parser.parse_expr()
    }

    /// `(( expr ))` command: applies side effects (`i++`, `i+=3`, `i=5`) then
    /// returns the resulting value (bash uses it as the exit status: 0 when
    /// non-zero).
    fn eval_arith_command(&mut self, expr: &str) -> i64 {
        let e = expr.trim();
        for (suffix, delta) in [("++", 1i64), ("--", -1)] {
            if let Some(name) = e.strip_suffix(suffix) {
                let name = name.trim();
                if is_var_name(name) {
                    let v = self.lookup_var(name).parse::<i64>().unwrap_or(0);
                    self.shell
                        .vars
                        .insert(name.to_string(), (v + delta).to_string());
                    return v;
                }
            }
        }
        for (prefix, delta) in [("++", 1i64), ("--", -1)] {
            if let Some(name) = e.strip_prefix(prefix) {
                let name = name.trim();
                if is_var_name(name) {
                    let v = self.lookup_var(name).parse::<i64>().unwrap_or(0) + delta;
                    self.shell.vars.insert(name.to_string(), v.to_string());
                    return v;
                }
            }
        }
        let bytes = e.as_bytes();
        for i in 0..bytes.len() {
            if bytes[i] != b'=' {
                continue;
            }
            let prev = if i > 0 { bytes[i - 1] } else { 0 };
            if matches!(prev, b'=' | b'!' | b'<' | b'>') {
                continue;
            }
            let (name, op) = if matches!(prev, b'+' | b'-' | b'*' | b'/' | b'%') {
                (&e[..i - 1], &e[i - 1..i + 1])
            } else {
                (&e[..i], "=")
            };
            let name = name.trim();
            if !is_var_name(name) {
                break;
            }
            let rhs = e.get(i + 1..).unwrap_or("").trim();
            let old = self.lookup_var(name).parse::<i64>().unwrap_or(0);
            let rv = self.eval_arithmetic(rhs);
            let newv = match op {
                "+=" => old + rv,
                "-=" => old - rv,
                "*=" => old * rv,
                "/=" => {
                    if rv != 0 {
                        old / rv
                    } else {
                        0
                    }
                }
                "%=" => {
                    if rv != 0 {
                        old % rv
                    } else {
                        0
                    }
                }
                _ => rv,
            };
            self.shell.vars.insert(name.to_string(), newv.to_string());
            return newv;
        }
        self.eval_arithmetic(e)
    }

    /// `[[ ... ]]` conditional expression evaluator.
    fn eval_dbracket(&mut self, expr: &str) -> bool {
        let toks = tokenize_dbracket(expr);
        let mut p = DbracketParser {
            rt: self,
            toks: &toks,
            pos: 0,
        };
        p.parse_or()
    }
}

fn is_var_name(s: &str) -> bool {
    !s.is_empty()
        && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Matches a for-loop word list that is exactly `"${a[@]}"` / `${a[*]}` etc.
fn array_words_name(w: &str) -> Option<&str> {
    let w = w.trim();
    let w = w.strip_prefix('"').unwrap_or(w);
    let w = w.strip_suffix('"').unwrap_or(w);
    let inner = w.strip_prefix("${")?.strip_suffix('}')?;
    inner
        .strip_suffix("[@]")
        .or_else(|| inner.strip_suffix("[*]"))
}

/// Tokenizes a `[[ ]]` expression, honoring quotes.
fn tokenize_dbracket(expr: &str) -> Vec<String> {
    let chars: Vec<char> = expr.chars().collect();
    let n = chars.len();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            let q = c;
            i += 1;
            let mut s = String::new();
            while i < n && chars[i] != q {
                s.push(chars[i]);
                i += 1;
            }
            i += 1;
            toks.push(s);
            continue;
        }
        if c == '(' || c == ')' {
            toks.push(c.to_string());
            i += 1;
            continue;
        }
        // Multi-char operators.
        let rest: String = chars[i..].iter().take(3).collect();
        let mut matched = None;
        for op in [
            "&&", "||", "==", "!=", "=~", "-eq", "-ne", "-lt", "-le", "-gt", "-ge",
        ] {
            if rest.starts_with(op) {
                matched = Some(op);
                break;
            }
        }
        if let Some(op) = matched {
            toks.push(op.to_string());
            i += op.chars().count();
            continue;
        }
        if c == '!' || c == '<' || c == '>' || c == '=' {
            toks.push(c.to_string());
            i += 1;
            continue;
        }
        // A word: consume until whitespace or an unquoted operator, folding
        // adjacent quoted segments into the same word (`*" "*` → `* *`).
        let mut word = String::new();
        while i < n {
            let ch = chars[i];
            if ch.is_whitespace() {
                break;
            }
            if ch == '\'' || ch == '"' {
                let q = ch;
                i += 1;
                while i < n && chars[i] != q {
                    word.push(chars[i]);
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
                continue;
            }
            if ch == '(' || ch == ')' || ch == '<' || ch == '>' || ch == '=' {
                break;
            }
            let rest2: String = chars[i..].iter().take(3).collect();
            if ["&&", "||", "==", "!=", "=~"]
                .iter()
                .any(|o| rest2.starts_with(*o))
            {
                break;
            }
            word.push(ch);
            i += 1;
        }
        toks.push(word);
    }
    toks
}

struct DbracketParser<'a> {
    rt: &'a mut Runtime,
    toks: &'a [String],
    pos: usize,
}

impl<'a> DbracketParser<'a> {
    fn peek(&self) -> Option<&str> {
        self.toks.get(self.pos).map(|s| s.as_str())
    }
    fn parse_or(&mut self) -> bool {
        let mut v = self.parse_and();
        while self.peek() == Some("||") {
            self.pos += 1;
            let r = self.parse_and();
            v = v || r;
        }
        v
    }
    fn parse_and(&mut self) -> bool {
        let mut v = self.parse_not();
        while self.peek() == Some("&&") {
            self.pos += 1;
            let r = self.parse_not();
            v = v && r;
        }
        v
    }
    fn parse_not(&mut self) -> bool {
        if self.peek() == Some("!") {
            self.pos += 1;
            return !self.parse_not();
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> bool {
        if self.peek() == Some("(") {
            self.pos += 1;
            let v = self.parse_or();
            if self.peek() == Some(")") {
                self.pos += 1;
            }
            return v;
        }
        // Unary operators.
        if let Some(t) = self.peek() {
            let unary = matches!(
                t,
                "-f" | "-d"
                    | "-e"
                    | "-s"
                    | "-z"
                    | "-n"
                    | "-r"
                    | "-w"
                    | "-x"
                    | "-L"
                    | "-h"
                    | "-b"
                    | "-c"
                    | "-p"
                    | "-S"
            );
            if unary {
                let op = t.to_string();
                self.pos += 1;
                let operand = self.toks.get(self.pos).cloned().unwrap_or_default();
                self.pos += 1;
                return self.unary(&op, &operand);
            }
        }
        // Binary: LHS OP RHS.
        let lhs = self.toks.get(self.pos).cloned();
        if let Some(lhs) = lhs {
            if let Some(op) = self.toks.get(self.pos + 1).cloned() {
                if matches!(
                    op.as_str(),
                    "==" | "="
                        | "!="
                        | "=~"
                        | "<"
                        | ">"
                        | "-eq"
                        | "-ne"
                        | "-lt"
                        | "-le"
                        | "-gt"
                        | "-ge"
                ) {
                    let rhs = self.toks.get(self.pos + 2).cloned().unwrap_or_default();
                    self.pos += 3;
                    return self.binary(&lhs, &op, &rhs);
                }
            }
            self.pos += 1;
            return !lhs.is_empty();
        }
        false
    }
    fn unary(&self, op: &str, operand: &str) -> bool {
        match op {
            "-z" => operand.is_empty(),
            "-n" => !operand.is_empty(),
            "-e" => self
                .rt
                .shell
                .vfs
                .resolve(operand, &self.rt.shell.cwd)
                .map(|p| p.exists())
                .unwrap_or(false),
            "-f" => self
                .rt
                .shell
                .vfs
                .resolve(operand, &self.rt.shell.cwd)
                .map(|p| p.is_file())
                .unwrap_or(false),
            "-d" => self
                .rt
                .shell
                .vfs
                .resolve(operand, &self.rt.shell.cwd)
                .map(|p| p.is_dir())
                .unwrap_or(false),
            "-s" => self
                .rt
                .shell
                .vfs
                .resolve(operand, &self.rt.shell.cwd)
                .map(|p| p.metadata().map(|m| m.len() > 0).unwrap_or(false))
                .unwrap_or(false),
            "-L" | "-h" => self
                .rt
                .shell
                .vfs
                .resolve(operand, &self.rt.shell.cwd)
                .map(|p| {
                    p.symlink_metadata()
                        .map(|m| m.is_symlink())
                        .unwrap_or(false)
                })
                .unwrap_or(false),
            "-r" | "-w" => true,
            "-x" => false,
            _ => false,
        }
    }
    fn binary(&self, lhs: &str, op: &str, rhs: &str) -> bool {
        match op {
            "==" | "=" => simple_glob_match(lhs, rhs),
            "!=" => !simple_glob_match(lhs, rhs),
            "=~" => regex::Regex::new(rhs)
                .map(|re| re.is_match(lhs))
                .unwrap_or(false),
            "<" => lhs < rhs,
            ">" => lhs > rhs,
            "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                let a = lhs.parse::<i64>().unwrap_or(0);
                let b = rhs.parse::<i64>().unwrap_or(0);
                match op {
                    "-eq" => a == b,
                    "-ne" => a != b,
                    "-lt" => a < b,
                    "-le" => a <= b,
                    "-gt" => a > b,
                    "-ge" => a >= b,
                    _ => false,
                }
            }
            _ => false,
        }
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
            if longest {
                best = Some(result);
            } else {
                return Some(result);
            }
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
            if longest {
                best = Some(result);
            } else {
                return Some(result);
            }
        }
    }
    best
}

fn simple_glob_match(s: &str, pat: &str) -> bool {
    if !pat.contains('*') && !pat.contains('?') {
        return s == pat;
    }
    let sc: Vec<char> = s.chars().collect();
    let pc: Vec<char> = pat.chars().collect();
    let (n, m) = (sc.len(), pc.len());
    let mut dp = vec![false; m + 1];
    dp[0] = true;
    for j in 0..m {
        if pc[j] == '*' {
            dp[j + 1] = dp[j];
        } else {
            break;
        }
    }
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
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '/' {
            let pat: String = chars[..i].iter().collect();
            let rep: String = chars[i + 1..].iter().collect();
            return (pat, rep);
        }
        i += 1;
    }
    (s.to_string(), String::new())
}

/// Unescape backslash escapes inside `${v/pat/rep}` pattern/replacement text
/// (e.g. `\/` -> `/`), matching how bash lets you embed the delimiter.
fn unescape_param_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Runtime {
    /// Extracts redirect operators from parsed tokens and returns the
    /// (cleaned_args, redirect_spec). Handles: > >> < 2> 2>> 1> 1>> 2>&1 >& &>
    fn extract_redirects(&self, parts: Vec<ParsedToken>) -> (Vec<ParsedToken>, RedirectSpec) {
        if parts.is_empty() {
            return (parts, RedirectSpec::default());
        }
        // Preserve the quoted flag so a QUOTED `>`/`<` is a literal argument,
        // not a redirect operator.
        let pairs: Vec<(String, bool)> = parts.into_iter().map(|t| (t.value, t.quoted)).collect();
        let (clean, spec) = parse_redirects(&pairs);
        (
            clean
                .into_iter()
                .map(|(v, q)| ParsedToken::new(v, q))
                .collect(),
            spec,
        )
    }

    /// Applies redirect_spec to a CommandOutput: writes stdout/stderr to files.
    /// When stdout and stderr target the same file (>& / &>), they are merged
    /// before writing to avoid the second write truncating the first.
    fn apply_redirects(&self, result: &mut CommandOutput, spec: &RedirectSpec) {
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

        // Binary stdout (gzip -c / tar -c …): consume it now so a stale buffer
        // never leaks into a later command. Written verbatim below.
        let binary = self.shell.take_binary_out();

        // When both stdout and stderr target the same file (>& / &>), write
        // them together to avoid truncation.
        let same_file = match (&spec.stdout_file, &spec.stderr_file) {
            (Some((a, _)), Some((b, _))) => a == b,
            _ => false,
        };

        if same_file {
            if let Some((ref path, append)) = &spec.stdout_file {
                if path == "/dev/null" {
                    result.stdout = String::new();
                    result.stderr = String::new();
                    return;
                }
                if let Some(bytes) = binary {
                    let mut buf = if *append {
                        self.shell
                            .vfs
                            .read(path, &self.shell.cwd)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    buf.extend_from_slice(&bytes);
                    if !result.stderr.is_empty() {
                        buf.extend_from_slice(result.stderr.as_bytes());
                    }
                    if let Err(e) = self.shell.vfs.write_bytes(path, &self.shell.cwd, &buf) {
                        result.stderr = format!("redirect: {}: {}\n", path, e);
                        result.exit_code = 1;
                    }
                    result.stdout = String::new();
                    result.stderr = String::new();
                    return;
                }
                let mut content = if *append {
                    self.shell
                        .vfs
                        .read_to_string(path, &self.shell.cwd)
                        .unwrap_or_default()
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
            if path != "/dev/null" {
                if let Some(bytes) = binary {
                    let buf = if *append {
                        let mut existing = self
                            .shell
                            .vfs
                            .read(path, &self.shell.cwd)
                            .unwrap_or_default();
                        existing.extend_from_slice(&bytes);
                        existing
                    } else {
                        bytes
                    };
                    if let Err(e) = self.shell.vfs.write_bytes(path, &self.shell.cwd, &buf) {
                        result.stderr = format!("redirect: {}: {}\n", path, e);
                        result.exit_code = 1;
                    }
                    result.stdout = String::new();
                    return;
                }
                let final_stdout = if *append {
                    let existing = self
                        .shell
                        .vfs
                        .read_to_string(path, &self.shell.cwd)
                        .unwrap_or_default();
                    existing + &result.stdout
                } else {
                    result.stdout.clone()
                };
                if let Err(e) = self.shell.vfs.write(path, &self.shell.cwd, &final_stdout) {
                    result.stderr = format!("redirect: {}: {}\n", path, e);
                    result.exit_code = 1;
                }
            }
            result.stdout = String::new();
        }

        // Write stderr to file
        if let Some((ref path, append)) = &spec.stderr_file {
            if path != "/dev/null" {
                let final_stderr = if *append {
                    let existing = self
                        .shell
                        .vfs
                        .read_to_string(path, &self.shell.cwd)
                        .unwrap_or_default();
                    existing + &result.stderr
                } else {
                    result.stderr.clone()
                };
                if let Err(e) = self.shell.vfs.write(path, &self.shell.cwd, &final_stderr) {
                    if result.stderr.is_empty() {
                        result.stderr = format!("redirect: {}: {}\n", path, e);
                    } else {
                        result
                            .stderr
                            .push_str(&format!("redirect: {}: {}\n", path, e));
                    }
                    result.exit_code = 1;
                }
            }
            result.stderr = String::new();
        }
    }

    fn expand_globs(&self, tokens: Vec<ParsedToken>) -> Vec<ParsedToken> {
        let mut expanded = Vec::new();
        for (i, token) in tokens.into_iter().enumerate() {
            let quoted = token.quoted;
            let value = token.value;
            // Brace expansion first (per word), then globbing. Done per-token so
            // `echo {a,b}c` → `ac bc` (not a whole-line rewrite that also
            // duplicates the command name).
            let words: Vec<String> = if !quoted && value.contains('{') && value.contains('}') {
                brace_expand_inner(&value)
            } else {
                vec![value]
            };
            for (j, w) in words.into_iter().enumerate() {
                let is_cmd = i == 0 && j == 0;
                if is_cmd || quoted || !has_glob_chars(&w) {
                    expanded.push(ParsedToken::new(w, quoted));
                    continue;
                }
                if *self.shell.shopt.get("noglob").unwrap_or(&false) {
                    expanded.push(ParsedToken::new(w, quoted));
                    continue;
                }
                let matches = self.try_glob(&w);
                if matches.is_empty() {
                    if *self.shell.shopt.get("failglob").unwrap_or(&false) {
                        // `shopt -s failglob`: a non-matching glob is an error.
                        self.glob_error.set(Some(format!("{}: no match\n", w)));
                        expanded.push(ParsedToken::new(w, quoted));
                    } else if !*self.shell.shopt.get("nullglob").unwrap_or(&false) {
                        expanded.push(ParsedToken::new(w, quoted));
                    }
                } else {
                    expanded.extend(matches.into_iter().map(|m| ParsedToken::new(m, quoted)));
                }
            }
        }
        expanded
    }

    /// Glob-expand every unquoted token (no positional exception). Used for
    /// shell word lists such as `for f in *.jpg`.
    fn expand_glob_words(&self, tokens: Vec<ParsedToken>) -> Vec<String> {
        let mut expanded = Vec::new();
        for token in tokens {
            let quoted = token.quoted;
            let value = token.value;
            let words: Vec<String> = if !quoted && value.contains('{') && value.contains('}') {
                brace_expand_inner(&value)
            } else {
                vec![value]
            };
            for w in words {
                if quoted || !has_glob_chars(&w) {
                    expanded.push(w);
                    continue;
                }
                if *self.shell.shopt.get("noglob").unwrap_or(&false) {
                    expanded.push(w);
                    continue;
                }
                let matches = self.try_glob(&w);
                if matches.is_empty() {
                    if *self.shell.shopt.get("nullglob").unwrap_or(&false) {
                        // `shopt -s nullglob`: no match → the word disappears.
                    } else {
                        expanded.push(w);
                    }
                } else {
                    expanded.extend(matches);
                }
            }
        }
        expanded
    }

    fn execute_pipeline(&mut self, input: &str, init_stdin: Option<&str>) -> CommandOutput {
        // (c) 2025 xiefujin <490021684@qq.com>
        // A binary stdout buffer is only meaningful for a redirect; drop any
        // stale value so it cannot leak across a pipe.
        let _ = self.shell.take_binary_out();
        let stages = parse_pipeline(input);
        if stages.is_empty() {
            return CommandOutput::success(String::new());
        }

        let mut expanded_stages: Vec<Vec<ParsedToken>> = Vec::new();
        for mut stage in stages {
            let expanded = self.expand_globs(std::mem::take(&mut stage));
            expanded_stages.push(expanded);
        }

        // Extract redirects from ALL stages (not just the last).
        // Per-stage stderr redirects and 2>&1 are applied in each thread.
        // Last-stage stdout redirects are applied after all threads join.
        let mut stage_redirects: Vec<RedirectSpec> = Vec::new();
        let mut clean_stages: Vec<Vec<ParsedToken>> = Vec::new();
        for stage in &expanded_stages {
            let (clean, spec) = self.extract_redirects(stage.clone());
            clean_stages.push(clean);
            stage_redirects.push(spec);
        }

        let last_idx = expanded_stages.len() - 1;
        let last_spec = stage_redirects.last().cloned().unwrap_or_default();

        if expanded_stages.len() == 1 {
            let clean = &clean_stages[0];
            if clean.is_empty() {
                let mut result = CommandOutput::success(String::new());
                self.apply_redirects(&mut result, &last_spec);
                return result;
            }
            let cmd = &clean[0].value;
            let args: Vec<&str> = clean[1..].iter().map(|t| t.value.as_str()).collect();
            let stdin_from_file = if init_stdin.is_none() {
                last_spec
                    .stdin_file
                    .as_ref()
                    .and_then(|path| self.shell.vfs.read_to_string(path, &self.shell.cwd).ok())
            } else {
                None
            };
            let stdin = init_stdin.or(stdin_from_file.as_deref());
            let mut result = self.shell.execute(cmd, &args, stdin);
            self.apply_redirects(&mut result, &last_spec);
            return result;
        }

        let saved_cwd = self.shell.cwd.clone();

        // stdin for the first stage: heredoc body, or `< file` redirect.
        let first_stdin: Option<String> = match init_stdin {
            Some(s) => Some(s.to_string()),
            None => stage_redirects
                .first()
                .and_then(|s| s.stdin_file.as_ref())
                .and_then(|path| self.shell.vfs.read_to_string(path, &self.shell.cwd).ok()),
        };

        let mut threads: Vec<
            std::thread::JoinHandle<std::thread::Result<(i32, String, String, Vec<u8>)>>,
        > = Vec::new();
        let mut prev_rx: Option<mpsc::Receiver<Vec<u8>>> = None;

        for (i, stage) in clean_stages.into_iter().enumerate() {
            if stage.is_empty() {
                continue;
            }
            if self.cancel.load(Ordering::SeqCst) {
                return CommandOutput {
                    stdout: String::new(),
                    stderr: "cancelled\n".to_string(),
                    exit_code: 143,
                };
            }
            let cmd = stage[0].value.clone();
            let args: Vec<String> = stage[1..].iter().map(|t| t.value.clone()).collect();
            let mut shell = self.shell.clone();
            let rx = prev_rx.take();
            let is_last = i == last_idx;
            let init = if i == 0 { first_stdin.clone() } else { None };

            // Per-stage redirects for non-last stages: 2>&1, 2> file.
            // stdout file redirects (>, >>) are only applied for the last stage.
            let stage_spec = stage_redirects.get(i).cloned().unwrap_or_default();

            let (tx, next_rx) = if !is_last {
                let (t, r) = mpsc::channel::<Vec<u8>>();
                (Some(t), Some(r))
            } else {
                (None, None)
            };

            let handle = std::thread::spawn(
                move || -> std::thread::Result<(i32, String, String, Vec<u8>)> {
                    // Pipeline stdin is received as Vec<u8> chunks.
                    // Shell commands currently accept stdin as &str (text-oriented design).
                    // Non-UTF-8 bytes are replaced with U+FFFD at the pipe boundary.
                    // Binary pipelines (e.g. `gzip -c | base64`) are not yet fully supported;
                    // use file redirection for binary workflows instead.
                    let mut stdin_buf = String::new();
                    let mut stdin_bytes: Vec<u8> = Vec::new();
                    let mut got_stdin = false;
                    if let Some(init) = init {
                        got_stdin = true;
                        stdin_bytes.extend_from_slice(init.as_bytes());
                        stdin_buf.push_str(&init);
                    }
                    if let Some(rx) = rx {
                        loop {
                            if shell.cancel.load(Ordering::SeqCst) {
                                let _ = tx.map(|t| t.send(Vec::new()));
                                return Ok((
                                    143,
                                    String::new(),
                                    "cancelled\n".to_string(),
                                    Vec::new(),
                                ));
                            }
                            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                                Ok(chunk) => {
                                    got_stdin = true;
                                    stdin_bytes.extend_from_slice(&chunk);
                                    stdin_buf.push_str(&String::from_utf8_lossy(&chunk));
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                            }
                        }
                    }
                    // Hand the byte-accurate stdin to binary-aware commands.
                    if got_stdin && !stdin_bytes.is_empty() {
                        shell.set_binary_in(stdin_bytes);
                    }
                    let stdin = if got_stdin { Some(stdin_buf) } else { None };
                    let stdin_ref = stdin.as_deref();
                    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

                    // Alias resolution (pipeline thread has cloned Shell with aliases)
                    let (resolved_cmd, resolved_args) =
                        if let Some((new_cmd, new_args)) = shell.resolve_alias(&cmd, &args_refs) {
                            (new_cmd, new_args)
                        } else {
                            (
                                cmd.clone(),
                                args_refs.iter().map(|s| s.to_string()).collect(),
                            )
                        };
                    let resolved_refs: Vec<&str> =
                        resolved_args.iter().map(|s| s.as_str()).collect();

                    if shell.cancel.load(Ordering::SeqCst) {
                        let _ = tx.map(|t| t.send(Vec::new()));
                        return Ok((143, String::new(), "cancelled\n".to_string(), Vec::new()));
                    }

                    let mut result = shell.execute(&resolved_cmd, &resolved_refs, stdin_ref);

                    // Per-stage redirects: 2>&1 merges stderr into the pipe.
                    if stage_spec.merge_stderr_to_stdout && !result.stderr.is_empty() {
                        if !result.stdout.is_empty() && !result.stdout.ends_with('\n') {
                            result.stdout.push('\n');
                        }
                        result.stdout.push_str(&result.stderr);
                        result.stderr.clear();
                    }
                    // Per-stage stderr file redirect.
                    if let Some((ref path, append)) = &stage_spec.stderr_file {
                        if path != "/dev/null" {
                            let mut content = if *append {
                                shell
                                    .vfs
                                    .read_to_string(path, &shell.cwd)
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            };
                            content.push_str(&result.stderr);
                            let _ = shell.vfs.write(path, &shell.cwd, &content);
                        }
                        result.stderr.clear();
                    }

                    let out_code = result.exit_code;
                    let out_stdout = result.stdout;
                    let out_stderr = result.stderr;
                    // Prefer the byte-accurate stdout (binary producers set it).
                    let out_bytes = shell
                        .take_binary_out()
                        .unwrap_or_else(|| out_stdout.as_bytes().to_vec());

                    if let Some(tx) = tx {
                        let _ = tx.send(out_bytes.clone());
                    }

                    Ok((out_code, out_stdout, out_stderr, out_bytes))
                },
            );

            threads.push(handle);
            prev_rx = next_rx;
        }

        let mut all_stderr = String::new();
        let mut final_exit_code = 0;
        let mut final_stdout = String::new();
        let mut panicked = false;
        let mut first_panicked: Option<usize> = None;
        let total_stages = threads.len();

        let pipefail = self.shell.pipefail;

        for (i, handle) in threads.into_iter().enumerate() {
            // Poll join with timeout so we can detect cancellation.
            loop {
                if self.cancel.load(Ordering::SeqCst) {
                    // Cancel flag set — abandon remaining threads.
                    // Already-handled stderr from earlier stages is preserved.
                    return CommandOutput {
                        stdout: String::new(),
                        stderr: format!("{}cancelled\n", all_stderr),
                        exit_code: 143,
                    };
                }
                if handle.is_finished() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            match handle.join() {
                Ok(Ok((code, out, err, bytes))) => {
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
                        if !bytes.is_empty() {
                            self.shell.set_binary_out(bytes);
                        }
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    if first_panicked.is_none() {
                        first_panicked = Some(i);
                    }
                    panicked = true;
                }
            }
        }

        self.shell.cwd = saved_cwd;

        if panicked {
            let stage = first_panicked.map(|i| i + 1).unwrap_or(0);
            let total = total_stages;
            return CommandOutput::error(
                format!("pipeline: stage {}/{} panicked (may be empty pipe, stderr noise, or unrecognized option; use 2>/dev/null to silence stderr)\n{}",
                    stage, total, all_stderr), 1);
        }

        let mut result = CommandOutput {
            stdout: final_stdout,
            stderr: all_stderr,
            exit_code: final_exit_code,
        };
        self.apply_redirects(&mut result, &last_spec);
        result
    }

    fn try_glob(&self, pattern: &str) -> Vec<String> {
        // extglob (`?(...)`/`*(...)`/`+(...)`/`@(...)`/`!(...)`) needs the
        // custom engine — the `glob` crate doesn't understand it.
        if *self.shell.shopt.get("extglob").unwrap_or(&false) && has_extglob(pattern) {
            let r = self.glob_engine(pattern);
            if !r.is_empty() {
                return r;
            }
        }
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
        // Bash keeps a trailing `/` in the pattern (`echo */` → `sub/`); the
        // glob crate drops it, so re-append it to directory matches.
        let trailing_slash = pattern.ends_with('/');
        let opts = glob::MatchOptions {
            case_sensitive: !*self.shell.shopt.get("nocaseglob").unwrap_or(&false),
            require_literal_separator: false,
            // bash: `*` does NOT match a leading dot unless `dotglob`.
            require_literal_leading_dot: !*self.shell.shopt.get("dotglob").unwrap_or(&false),
        };
        match glob::glob_with(&glob_pattern, opts) {
            Ok(paths) => paths
                .filter_map(|entry| entry.ok())
                .filter_map(|p| {
                    p.strip_prefix(&cwd_path).ok().map(|r| {
                        let s = r.to_string_lossy().to_string();
                        if trailing_slash && !s.ends_with('/') {
                            format!("{s}/")
                        } else {
                            s
                        }
                    })
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    /// Full glob engine: per-segment matching with extglob + `**`, honoring
    /// `nocaseglob`/`dotglob`. Returns display paths (relative to cwd, or
    /// absolute when the pattern is absolute).
    fn glob_engine(&self, pattern: &str) -> Vec<String> {
        let extglob = *self.shell.shopt.get("extglob").unwrap_or(&false);
        let nocase = *self.shell.shopt.get("nocaseglob").unwrap_or(&false);
        let dotglob = *self.shell.shopt.get("dotglob").unwrap_or(&false);
        let absolute = pattern.starts_with('/');
        let segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        let start = if absolute {
            "/".to_string()
        } else if self.shell.cwd.is_empty() {
            "/".to_string()
        } else {
            self.shell.cwd.clone()
        };
        let mut bases: Vec<String> = vec![start];
        let mut results: Vec<String> = Vec::new();
        let n = segs.len();

        for (si, seg) in segs.iter().enumerate() {
            let last = si + 1 == n;
            if *seg == "**" {
                let mut all_dirs: Vec<String> = Vec::new();
                for b in &bases {
                    self.collect_all_dirs(b, &mut all_dirs, 0);
                }
                if last {
                    for d in &all_dirs {
                        if let Ok(entries) = self.shell.vfs.list_dir(d, "") {
                            for e in entries {
                                if !dotglob && e.name.starts_with('.') {
                                    continue;
                                }
                                results.push(join_vpath(d, &e.name));
                            }
                        }
                    }
                } else {
                    bases = all_dirs;
                }
                continue;
            }
            let re = compile_glob_seg(seg, extglob, nocase);
            // `!(pat|pat)` — whole-segment negative match (Rust's regex has no
            // lookahead, so exclude positively-matched alternatives here).
            let neg_alts: Option<Vec<regex::Regex>> = if extglob {
                seg.strip_prefix("!(")
                    .and_then(|r| r.strip_suffix(')'))
                    .map(|inner| {
                        split_top_alt(inner)
                            .iter()
                            .filter_map(|a| compile_glob_seg(a, extglob, nocase))
                            .collect()
                    })
            } else {
                None
            };
            let mut next: Vec<String> = Vec::new();
            for b in &bases {
                let entries = match self.shell.vfs.list_dir(b, "") {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for e in entries {
                    if !dotglob && e.name.starts_with('.') && !seg.starts_with('.') {
                        continue;
                    }
                    let ok = if let Some(alts) = &neg_alts {
                        !alts.iter().any(|a| a.is_match(&e.name))
                    } else {
                        match &re {
                            Some(re) => re.is_match(&e.name),
                            None => e.name == *seg,
                        }
                    };
                    if ok {
                        let child = join_vpath(b, &e.name);
                        if last {
                            results.push(child.clone());
                        }
                        if e.is_dir {
                            next.push(child);
                        }
                    }
                }
            }
            bases = next;
        }
        results.sort();
        results
            .into_iter()
            .map(|p| self.glob_display(p, absolute))
            .collect()
    }

    fn collect_all_dirs(&self, base: &str, out: &mut Vec<String>, depth: usize) {
        if depth > 32 {
            return;
        }
        out.push(base.to_string());
        if let Ok(entries) = self.shell.vfs.list_dir(base, "") {
            for e in entries {
                if e.is_dir && !e.name.starts_with('.') {
                    self.collect_all_dirs(&join_vpath(base, &e.name), out, depth + 1);
                }
            }
        }
    }

    fn glob_display(&self, path: String, absolute: bool) -> String {
        if absolute {
            return path;
        }
        let cwd = &self.shell.cwd;
        if cwd == "/" {
            path.trim_start_matches('/').to_string()
        } else if let Some(rest) = path.strip_prefix(&format!("{}/", cwd)) {
            rest.to_string()
        } else {
            path.trim_start_matches('/').to_string()
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

    /// Python entry that preserves argument boundaries. Handles
    /// `python3 -c CODE [args…]` (CODE kept as one unit, extras exposed as
    /// `sys.argv[1..]`), falling back to the string classifier otherwise.
    fn execute_python_argv(&mut self, toks: &[String]) -> CommandOutput {
        let mut i = 1; // toks[0] is python/python3
        while i < toks.len() {
            let t = toks[i].as_str();
            if t == "-c" {
                if i + 1 < toks.len() {
                    let code = toks[i + 1].clone();
                    let args = toks[i + 2..].to_vec();
                    let argv = std::iter::once("-c".to_string())
                        .chain(args.iter().cloned())
                        .map(|a| format!("{a:?}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let wrapped = format!("import sys\nsys.argv = [{argv}]\n{code}");
                    return self.execute_python_code(&wrapped);
                }
                break;
            }
            // `python3 -m pip …` → route to the native pip implementation
            // (the embedded engine has no `pip` module).
            if t == "-m" && i + 1 < toks.len() && toks[i + 1] == "pip" {
                let rest: Vec<&str> = toks[i + 2..].iter().map(|s| s.as_str()).collect();
                return self.shell.cmd_pip(&rest);
            }
            if !t.starts_with('-') {
                break;
            }
            i += 1;
        }
        self.execute_python_inner(&toks.join(" "))
    }

    fn execute_python_inner(&mut self, input: &str) -> CommandOutput {
        // Export the sandbox root for EVERY python entry point (incl. `python3 -c`),
        // otherwise the embedded RustPython engine disables its file sandbox and
        // `python3 -c "open('/etc/passwd')"` could read host files.
        self.export_sandbox_env();
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
            PyInvocation::Stdin(args) => {
                // Program comes from stdin: the executor stages it in
                // `_py_stdin` for `PRODUCER | python3 -` (mobile path).
                let code = self
                    .shell
                    .vfs
                    .resolve("_py_stdin", &self.shell.cwd)
                    .ok()
                    .and_then(|p| std::fs::read_to_string(p).ok());
                let Some(code) = code else {
                    return CommandOutput::error(
                        "python3 -: no program on stdin (pipe a script or use a heredoc)\n"
                            .to_string(),
                        1,
                    );
                };
                let full = if args.is_empty() {
                    code
                } else {
                    let argv = std::iter::once("python3".to_string())
                        .chain(args.iter().cloned())
                        .map(|a| format!("{a:?}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("import sys\nsys.argv = [{argv}]\n{code}")
                };
                self.execute_python_code(&full)
            }
            PyInvocation::Repl => CommandOutput::error(
                "interactive python REPL is not supported; use `python -c` or a script".to_string(),
                1,
            ),
            PyInvocation::Version => {
                self.execute_python_code("import sys; print('Python ' + sys.version)")
            }
            PyInvocation::Help => CommandOutput::success(
                "usage: python3 [-c cmd | -m mod | file | -] [arg]...\n\
                 -c cmd : program passed in as string\n\
                 -m mod : run library module as a script\n\
                 --version : print the Python version\n"
                    .to_string(),
            ),
        }
    }

    /// Export the sandbox root + logical cwd for the Python sandbox wrapper
    /// (`FASTSHELL_ROOT` / `FASTSHELL_CWD`). Both the embedded RustPython engine
    /// (in-process) and the subprocess engine read these, so `/projects/...`
    /// resolves the same way in Python as in the shell.
    fn export_sandbox_env(&self) {
        crate::python::set_python_sandbox(
            &self.shell.vfs.root().to_string_lossy(),
            &self.shell.cwd,
        );
    }

    pub fn execute_python_code(&mut self, code: &str) -> CommandOutput {
        let cwd = self.vfs_cwd();
        // Expose the sandbox root + logical cwd so the Python sandbox wrapper can
        // resolve the shell's virtual paths (`/projects/...`) identically.
        self.export_sandbox_env();
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
        self.export_sandbox_env();
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
    // Bare pytest is rewritten to `python -m pytest`.
    if trimmed == "pytest" || trimmed.starts_with("pytest ") {
        return true;
    }
    let first = trimmed.split_whitespace().next().unwrap_or("");
    is_python_prog(first)
}

/// True for `python`, `python3`, `python3.11`, `python2.7`, …
fn is_python_prog(name: &str) -> bool {
    if name == "py" {
        return true;
    }
    match name.strip_prefix("python") {
        Some(rest) => {
            rest.is_empty()
                || rest
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
        }
        None => false,
    }
}

/// The kind of python invocation extracted from a command line.
#[derive(Debug, PartialEq)]
enum PyInvocation {
    /// `python -c "code"`
    Code(String),
    /// `python - [args...]` — program read from stdin (heredoc / pipe).
    Stdin(Vec<String>),
    /// `python script.py [args...]`
    Script(String, Vec<String>),
    /// `python -m module [args...]`  (also bare `pytest ...`)
    Module(String, Vec<String>),
    /// bare `python` / `python3` REPL
    Repl,
    /// `python --version` / `-V`
    Version,
    /// `python --help` / `-h`
    Help,
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

    // -c inline code (preserve original quote-stripping behavior). Note: the
    // dispatch strips quotes before we get here, so extra args after the code
    // can't be split reliably — the whole remainder is treated as the code.
    for name in ["python3", "python2", "python2.7", "python", "py"] {
        if trimmed.starts_with(&format!("{name} -c")) {
            return PyInvocation::Code(extract_python_code(trimmed));
        }
    }

    let tokens = tokenize(trimmed);
    // tokens[0] is python/python3
    if tokens.len() <= 1 {
        return PyInvocation::Repl;
    }
    let rest = &tokens[1..];

    // `python3 --version` / `-V` and `--help` / `-h` (no script follows).
    if matches!(rest[0].as_str(), "--version" | "-V") {
        return PyInvocation::Version;
    }
    if matches!(rest[0].as_str(), "--help" | "-h") {
        return PyInvocation::Help;
    }

    // -c inline code (generic — also for `python3.11 -c ...`).
    if rest[0] == "-c" {
        if rest.len() >= 2 {
            return PyInvocation::Code(rest[1].clone());
        }
        return PyInvocation::Repl;
    }

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
    // first non-flag token as the script path. A bare `-` means "program from
    // stdin" (heredoc or pipe).
    let mut idx = 0;
    while idx < rest.len() && rest[idx].starts_with('-') {
        if rest[idx] == "-" {
            return PyInvocation::Stdin(rest[idx + 1..].to_vec());
        }
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

    for name in ["python3", "python2", "python2.7", "python", "py"] {
        if let Some(code) = trimmed.strip_prefix(&format!("{name} -c ")) {
            return strip_quotes(code);
        }
        if let Some(code) = trimmed.strip_prefix(&format!("{name} -c")) {
            return strip_quotes(code.trim());
        }
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
/// Parses a leading `read VAR...` statement out of a brace-group body.
/// Returns (vars, remaining_body). Flags like `-r` are ignored.
fn parse_leading_read(inner: &str) -> Option<(Vec<String>, &str)> {
    let inner = inner.trim_start();
    let end = inner.find(|c| c == ';' || c == '\n').unwrap_or(inner.len());
    let first = inner[..end].trim();
    let rest = if end < inner.len() {
        inner[end + 1..].trim()
    } else {
        ""
    };
    let after = first.strip_prefix("read")?;
    if !(after.is_empty() || after.starts_with(char::is_whitespace)) {
        return None;
    }
    let vars: Vec<String> = after
        .split_whitespace()
        .filter(|t| !t.starts_with('-'))
        .map(|t| t.to_string())
        .collect();
    Some((vars, rest))
}

/// True when `s` contains a `|` that is an actual pipeline separator (not
/// inside single/double quotes and not escaped). Prevents a quoted pipe such as
/// `grep 'a|b'` or `f "x|y"` from being routed into the pipeline executor,
/// which does not resolve shell functions.
fn has_unquoted_pipe(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut in_single = false;
    let mut in_double = false;
    let mut depth = 0i32;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if in_single {
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match c {
            '\\' if i + 1 < n => {
                i += 2;
                continue;
            }
            '\'' => in_single = true,
            '"' => in_double = true,
            '(' => depth += 1,
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            '|' if depth == 0 => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// True if the standalone keyword `kw` starts at byte/char index `i`.
fn starts_word(chars: &[char], i: usize, kw: &str) -> bool {
    let n = chars.len();
    let k: Vec<char> = kw.chars().collect();
    if i + k.len() > n {
        return false;
    }
    for (off, &kc) in k.iter().enumerate() {
        if chars[i + off] != kc {
            return false;
        }
    }
    let before_ok = i == 0
        || chars[i - 1].is_whitespace()
        || matches!(chars[i - 1], ';' | '&' | '|' | '(' | ')');
    let after = i + k.len();
    let after_ok = after >= n
        || chars[after].is_whitespace()
        || matches!(chars[after], ';' | '&' | '|' | '(' | ')');
    before_ok && after_ok
}

/// Number of standalone occurrences of `w` in `s`.
fn count_word(s: &str, w: &str) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut n = 0;
    let mut i = 0;
    while i < chars.len() {
        if starts_word(&chars, i, w) {
            n += 1;
            i += w.chars().count();
        } else {
            i += 1;
        }
    }
    n
}

/// True if `s` contains the standalone word `w`.
fn contains_word(s: &str, w: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    (0..chars.len()).any(|i| starts_word(&chars, i, w))
}

/// Split a leading `( … )` subshell or `{ … }` group from the trailing text
/// (usually redirects like `> f 2>&1`). Nesting- and quote-aware.
/// Returns `None` when `raw` doesn't start with a group or its matching close
/// can't be found — so callers fall back to normal command parsing.
fn split_leading_group(raw: &str) -> Option<(String, String)> {
    let chars: Vec<char> = raw.chars().collect();
    let n = chars.len();
    let open = chars.first().copied()?;
    let close = match open {
        '(' => ')',
        '{' => '}',
        _ => return None,
    };
    // `(( ... ))` is arithmetic, not a subshell.
    if open == '(' && n >= 2 && chars[1] == '(' {
        return None;
    }
    let mut depth = 0i32; // depth of the opening/closing bracket pair
    let mut paren = 0i32; // nested `$( ... )`
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if in_single {
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if c == '$' && i + 1 < n && chars[i + 1] == '(' {
                paren += 1;
                i += 2;
                continue;
            }
            if paren > 0 {
                if c == '(' {
                    paren += 1;
                } else if c == ')' {
                    paren -= 1;
                }
            } else if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' => in_single = true,
            '"' => in_double = true,
            '\\' if i + 1 < n => {
                i += 2;
                continue;
            }
            '$' if i + 1 < n && chars[i + 1] == '(' => {
                paren += 1;
                i += 2;
                continue;
            }
            '(' if paren > 0 => paren += 1,
            ')' if paren > 0 => paren -= 1,
            c2 if c2 == open => depth += 1,
            c2 if c2 == close => {
                depth -= 1;
                if depth == 0 {
                    let construct: String = chars[..=i].iter().collect();
                    let trailing: String = chars[i + 1..].iter().collect();
                    return Some((construct, trailing));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether a leading construct from [`split_leading_group`] is a real subshell
/// (`( … )`) or command group (`{ …; }`) — not e.g. a brace expansion `{a,b}`.
fn is_group_construct(construct: &str) -> bool {
    if construct.starts_with('(') && construct.ends_with(')') && !construct.starts_with("((") {
        return true;
    }
    if construct.starts_with('{') && construct.ends_with('}') {
        let inner = construct[1..construct.len() - 1].trim();
        return !inner.is_empty()
            && (inner.contains(';')
                || inner.contains("&&")
                || inner.contains("||")
                || inner.contains('\n'));
    }
    false
}

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
    // `$(...)` nesting inside double quotes: a `"` there is a NESTED quote and
    // must not terminate the outer double-quoted string.
    let mut subst_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut case_depth = 0usize;
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
            if c == '\\' && i + 1 < n {
                current.push(chars[i + 1]);
                i += 1;
                i += 1;
                continue;
            }
            if c == '$' && i + 1 < n && chars[i + 1] == '(' {
                subst_depth += 1;
                current.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if subst_depth > 0 {
                // Inside `$(...)`: track paren nesting; `"`/`'` here are nested
                // quotes and must not end the outer double-quoted string.
                if c == '(' {
                    subst_depth += 1;
                } else if c == ')' {
                    subst_depth -= 1;
                }
            } else if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        // `case ... esac`: `;;` and `&&`/`||` inside a branch must not split.
        if case_depth == 0 && starts_word(&chars, i, "case") {
            case_depth += 1;
        } else if case_depth > 0 && starts_word(&chars, i, "esac") {
            case_depth -= 1;
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
            '[' if i + 1 < n && chars[i + 1] == '[' && brace_depth == 0 && paren_depth == 0 => {
                // `[[ ... ]]` is one conditional expression: copy it verbatim
                // so `&&` / `||` inside it are not treated as segment connectors.
                let mut j = i + 2;
                let mut found = false;
                while j + 1 < n {
                    if chars[j] == ']' && chars[j + 1] == ']' {
                        j += 2;
                        found = true;
                        break;
                    }
                    j += 1;
                }
                if found {
                    for ch in &chars[i..j] {
                        current.push(*ch);
                    }
                    i = j;
                } else {
                    current.push(c);
                    i += 1;
                }
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
                    if brace_depth == 0 && paren_depth == 0 && case_depth == 0 {
                        flush!();
                    } else {
                        current.push(c);
                    }
                    i += 1;
                }
            }
            '#' if brace_depth == 0
                && paren_depth == 0
                && (i == 0 || matches!(chars[i - 1], ' ' | '\t' | '\n' | ';' | '&' | '|')) =>
            {
                // Comment: skip to end of line. Quotes inside a comment must NOT
                // start a quoted region (an apostrophe like "let's" otherwise
                // swallows the following lines and breaks splitting).
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            ';' => {
                if brace_depth == 0 && paren_depth == 0 && case_depth == 0 {
                    flush!();
                } else {
                    current.push(c);
                }
                i += 1;
            }
            '&' => {
                if i + 1 < n
                    && chars[i + 1] == '&'
                    && brace_depth == 0
                    && paren_depth == 0
                    && case_depth == 0
                {
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
                } else if brace_depth == 0 && paren_depth == 0 && case_depth == 0 {
                    // Background `&` — keep it on the segment; the executor runs
                    // it synchronously and records a job (so `$!`/`jobs`/`wait`
                    // behave sensibly).
                    current.push('&');
                    flush!();
                    i += 1;
                } else {
                    // Inside a subshell/brace group — keep `&`/`&&` literally.
                    current.push(c);
                    i += 1;
                }
            }
            '|' => {
                if i + 1 < n
                    && chars[i + 1] == '|'
                    && brace_depth == 0
                    && paren_depth == 0
                    && case_depth == 0
                {
                    next_connector = Connector::OrIf;
                    flush!();
                    i += 2;
                } else {
                    current.push(c);
                    i += 1;
                }
            }
            '<' if i + 1 < n
                && chars[i + 1] == '<'
                && i + 2 < n
                && chars[i + 2] == '<'
                && brace_depth == 0
                && paren_depth == 0 =>
            {
                // Here-string: <<< WORD. The value may be quoted (spaces OK) or a
                // single unquoted word — it must NOT swallow the rest of the
                // line (`read x <<< word; echo done` used to eat `; echo done`).
                let mut j = i + 3;
                while j < n && (chars[j] == ' ' || chars[j] == '\t') {
                    j += 1;
                }
                let mut text = String::new();
                let mut expand = true;
                if j + 1 < n && chars[j] == '$' && chars[j + 1] == '\'' {
                    // `$'...'` ANSI-C quoted here-string: decode escapes now.
                    let mut k = j + 2;
                    let mut inner = String::new();
                    while k < n && chars[k] != '\'' {
                        if chars[k] == '\\' && k + 1 < n {
                            inner.push(chars[k]);
                            inner.push(chars[k + 1]);
                            k += 2;
                            continue;
                        }
                        inner.push(chars[k]);
                        k += 1;
                    }
                    text = decode_ansi_c_escapes(&inner);
                    j = (k + 1).min(n);
                    expand = false;
                } else if j < n && (chars[j] == '"' || chars[j] == '\'') {
                    let q = chars[j];
                    let mut k = j + 1;
                    while k < n && chars[k] != q {
                        text.push(chars[k]);
                        k += 1;
                    }
                    j = (k + 1).min(n);
                    // Single-quoted here-strings are literal; double-quoted expand.
                    expand = q == '"';
                } else {
                    // Unquoted word: stop at any unquoted shell metacharacter,
                    // not just whitespace — otherwise `<<< x; cmd` would swallow
                    // the `;` and the following command into the value.
                    while j < n
                        && !chars[j].is_whitespace()
                        && !matches!(chars[j], ';' | '&' | '|' | '<' | '>' | '(' | ')')
                    {
                        text.push(chars[j]);
                        j += 1;
                    }
                }
                // bash appends a trailing newline to a here-string's input.
                heredoc = Some(format!("{}\n", text));
                heredoc_expand_flag = expand;
                i = j;
            }
            '<' if i + 1 < n && chars[i + 1] == '<' && brace_depth == 0 && paren_depth == 0 => {
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
            '$' if i + 2 < n && chars[i + 1] == '(' && chars[i + 2] == '(' => {
                // `$(( arithmetic ))` — copy verbatim; its parens must NOT affect
                // the subshell depth (the inner `(` was otherwise miscounted).
                let mut j = i + 3;
                let mut depth = 0i32;
                while j < n {
                    if chars[j] == '(' {
                        depth += 1;
                    } else if chars[j] == ')' {
                        if depth == 0 {
                            j += if j + 1 < n && chars[j + 1] == ')' {
                                2
                            } else {
                                1
                            };
                            break;
                        }
                        depth -= 1;
                    }
                    j += 1;
                }
                for ch in &chars[i..j.min(n)] {
                    current.push(*ch);
                }
                i = j;
            }
            '(' => {
                // Count every `(` (including `$(`, `<(` and `>(`) toward the
                // paren depth so `;` / `&&` / `||` *inside* a command/process
                // substitution are not treated as top-level segment connectors.
                // `$(( … ))` arithmetic is copied verbatim above and never
                // reaches here.
                paren_depth += 1;
                current.push(c);
                i += 1;
            }
            ')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
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
/// Parses `name=(a b c)` array assignment. Returns (name, items, rest).
fn take_array_assignment(input: &str) -> Option<(String, Vec<String>, &str, bool)> {
    let s = input.trim_start();
    // `name=(...)` or `name+=(...)`.
    let (name_end, eq, append) = if let Some(p) = s.find("+=(") {
        (p, p + 1, true)
    } else {
        let e = s.find('=')?;
        (e, e, false)
    };
    let name = &s[..name_end];
    if !is_var_name(name) {
        return None;
    }
    let after = &s[eq + 1..];
    if !after.starts_with('(') {
        return None;
    }
    let chars: Vec<char> = after.chars().collect();
    let mut depth = 0i32;
    let mut close: Option<usize> = None;
    let mut q: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(qc) = q {
            if c == qc {
                q = None;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' | '"' => q = Some(c),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let close = close?;
    let inner: String = chars[1..close].iter().collect();
    let items: Vec<String> = shell_words_parse(&inner)
        .unwrap_or_else(|_| inner.split_whitespace().map(|s| s.to_string()).collect());
    let consumed: String = chars[..=close].iter().collect();
    let rest = &s[eq + 1 + consumed.len()..];
    Some((name.to_string(), items, rest, append))
}

fn take_assignment(input: &str) -> Option<(String, String, &str, bool)> {
    let s = input.trim_start();
    let mut name_end = 0;
    let mut append = false;
    let mut eq_pos: Option<usize> = None;
    let bytes = s.as_bytes();
    for (idx, c) in s.char_indices() {
        if idx == 0 {
            if !(c.is_ascii_alphabetic() || c == '_') {
                return None;
            }
            name_end = c.len_utf8();
            continue;
        }
        if c == '=' {
            eq_pos = Some(idx);
            break;
        }
        // `NAME+=value` — append assignment.
        if c == '+' && bytes.get(idx + 1) == Some(&b'=') {
            append = true;
            eq_pos = Some(idx + 1);
            name_end = idx;
            break;
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        name_end = idx + c.len_utf8();
    }
    let eq = eq_pos?;
    let name = s[..name_end].to_string();
    let after_eq = &s[eq + 1..];

    // Value: a single shell word — a concatenation of quoted and unquoted
    // segments (`x"y"z`, `"a b"c`, `"p"$(cmd)post`). Quotes are stripped; the
    // text is already expanded by `expand_line`.
    let (value, rest_idx) = scan_assignment_value(after_eq);
    Some((name, value, &after_eq[rest_idx..], append))
}

/// Scans a single shell-word value for an assignment. Reads until the first
/// *unnested* whitespace, tracking `( … )` (incl. `$( … )`) so
/// `VAR=${x}$(cmd with spaces)` keeps the whole value. Returns the value and
/// the byte offset in `after_eq` where the value ends.
fn scan_assignment_value(after_eq: &str) -> (String, usize) {
    let mut value = String::new();
    let mut rest_idx = after_eq.len();
    let mut depth = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;
    let mut escaped = false;
    let mut chars = after_eq.char_indices();
    while let Some((idx, c)) = chars.next() {
        if in_sq {
            if c == '\'' {
                in_sq = false;
            } else {
                value.push(c);
            }
            continue;
        }
        if in_dq {
            if escaped {
                // Only `\"` and `\\` are unescaped; other `\x` keeps the `\`.
                if c == '"' || c == '\\' {
                    value.push(c);
                } else {
                    value.push('\\');
                    value.push(c);
                }
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '"' {
                in_dq = false;
                continue;
            }
            value.push(c);
            continue;
        }
        match c {
            '\\' => {
                // Unquoted escape: the escaped char is taken literally.
                if let Some((_, next)) = chars.next() {
                    value.push(next);
                } else {
                    value.push('\\');
                }
            }
            '\'' => in_sq = true,
            '"' => in_dq = true,
            '(' => {
                depth += 1;
                value.push(c);
            }
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
                value.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                rest_idx = idx;
                break;
            }
            _ => value.push(c),
        }
    }
    (value, rest_idx)
}

/// Parses `NAME[subscript]=value` / `NAME[subscript]+=value`.
/// Returns (name, subscript, value, rest, append).
fn take_element_assignment(input: &str) -> Option<(String, String, String, &str, bool)> {
    let s = input.trim_start();
    let open = s.find('[')?;
    let name = &s[..open];
    if !is_var_name(name) {
        return None;
    }
    let close = s[open + 1..].find(']')? + open + 1;
    let key = s[open + 1..close].to_string();
    let rest = &s[close + 1..];
    let (append, rest) = if let Some(r) = rest.strip_prefix("+=") {
        (true, r)
    } else if let Some(r) = rest.strip_prefix('=') {
        (false, r)
    } else {
        return None;
    };
    let (value, rest_idx) = scan_assignment_value(rest);
    Some((name.to_string(), key, value, &rest[rest_idx..], append))
}

/// Decode ANSI-C (`$'...'`) escape sequences into their real characters.
fn decode_ansi_c_escapes(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let c = chars[i + 1];
        match c {
            'n' => {
                out.push('\n');
                i += 2;
            }
            't' => {
                out.push('\t');
                i += 2;
            }
            'r' => {
                out.push('\r');
                i += 2;
            }
            'a' => {
                out.push('\u{07}');
                i += 2;
            }
            'b' => {
                out.push('\u{08}');
                i += 2;
            }
            'f' => {
                out.push('\u{0C}');
                i += 2;
            }
            'v' => {
                out.push('\u{0B}');
                i += 2;
            }
            'e' | 'E' => {
                out.push('\u{1B}');
                i += 2;
            }
            '\\' => {
                out.push('\\');
                i += 2;
            }
            '\'' => {
                out.push('\'');
                i += 2;
            }
            '"' => {
                out.push('"');
                i += 2;
            }
            'x' => {
                let mut k = 0;
                let mut val: u32 = 0;
                while k < 2 && i + 2 + k < chars.len() {
                    if let Some(d) = chars[i + 2 + k].to_digit(16) {
                        val = val * 16 + d;
                        k += 1;
                    } else {
                        break;
                    }
                }
                if k == 0 {
                    out.push('\\');
                    out.push('x');
                    i += 2;
                } else {
                    out.push(char::from_u32(val).unwrap_or('\u{FFFD}'));
                    i += 2 + k;
                }
            }
            '0'..='7' => {
                let mut k = 0;
                let mut val: u32 = 0;
                while k < 3 && i + 1 + k < chars.len() {
                    if let Some(d) = chars[i + 1 + k].to_digit(8) {
                        val = val * 8 + d;
                        k += 1;
                    } else {
                        break;
                    }
                }
                out.push(char::from_u32(val).unwrap_or('\0'));
                i += 1 + k;
            }
            _ => {
                out.push('\\');
                out.push(c);
                i += 2;
            }
        }
    }
    out
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
    merge_stderr_to_stdout: bool, // 2>&1
    merge_stdout_to_stderr: bool, // 1>&2
}

/// Parses redirect operators from `args` and returns (clean_args, spec).
/// Scans right-to-left so the rightmost redirect for a given fd wins.
fn parse_redirects(args: &[(String, bool)]) -> (Vec<(String, bool)>, RedirectSpec) {
    let mut spec = RedirectSpec::default();
    let n = args.len();
    // Only UNQUOTED tokens can be redirect operators.
    let is_op = |t: &str| {
        matches!(
            t,
            ">" | "1>"
                | ">>"
                | "1>>"
                | "2>"
                | "2>>"
                | "<"
                | "<>"
                | "&>>"
                | ">&"
                | "&>"
                | "2>&1"
                | "1>&2"
                | ">&2"
        )
    };
    let consumes_target = |t: &str| {
        matches!(
            t,
            ">" | "1>" | ">>" | "1>>" | "2>" | "2>>" | "<" | "<>" | "&>>" | ">&" | "&>"
        )
    };

    // Scan from the end so the right-most operator of each kind wins.
    let mut i: isize = n as isize - 1;
    while i >= 0 {
        let idx = i as usize;
        if args[idx].1 || !is_op(&args[idx].0) {
            i -= 1;
            continue;
        }
        let has_next = (i + 1) < n as isize;
        let next = (i + 1) as usize;
        match args[idx].0.as_str() {
            ">" | "1>" if has_next && spec.stdout_file.is_none() => {
                spec.stdout_file = Some((args[next].0.clone(), false))
            }
            ">>" | "1>>" if has_next && spec.stdout_file.is_none() => {
                spec.stdout_file = Some((args[next].0.clone(), true))
            }
            "2>" if has_next && spec.stderr_file.is_none() => {
                spec.stderr_file = Some((args[next].0.clone(), false))
            }
            "2>>" if has_next && spec.stderr_file.is_none() => {
                spec.stderr_file = Some((args[next].0.clone(), true))
            }
            "<" | "<>" if has_next && spec.stdin_file.is_none() => {
                spec.stdin_file = Some(args[next].0.clone())
            }
            "2>&1" => spec.merge_stderr_to_stdout = true,
            "1>&2" | ">&2" => spec.merge_stdout_to_stderr = true,
            ">&" | "&>" if has_next && spec.stdout_file.is_none() && spec.stderr_file.is_none() => {
                let p = args[next].0.clone();
                spec.stdout_file = Some((p.clone(), false));
                spec.stderr_file = Some((p, false));
            }
            "&>>" if has_next && spec.stdout_file.is_none() && spec.stderr_file.is_none() => {
                let p = args[next].0.clone();
                spec.stdout_file = Some((p.clone(), true));
                spec.stderr_file = Some((p, true));
            }
            _ => {}
        }
        i -= 1;
    }

    // Build clean: skip unquoted operators and the target token they consume.
    let mut clean = Vec::new();
    let mut k = 0;
    while k < n {
        let t = args[k].0.as_str();
        let quoted = args[k].1;
        if !quoted && is_op(t) {
            k += 1;
            if consumes_target(t) && k < n {
                k += 1; // skip the filename target
            }
        } else {
            clean.push(args[k].clone());
            k += 1;
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
            } else if ch == '\\'
                && chars.peek().map_or(false, |&n| {
                    n == '"' || n == '\\' || n == '$' || n == '`' || n == '!'
                })
            {
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
        } else if ch == '\\' {
            // Backslash escape outside quotes: `\*` → literal `*`, `\ ` → a
            // literal space inside the word. Mark the token as quoted so glob
            // / brace expansion leave it alone.
            match chars.next() {
                Some(next) => {
                    current.push(next);
                    quoted = true;
                }
                None => current.push(ch),
            }
        } else if ch == ' ' || ch == '\t' {
            // An explicitly quoted empty token (`printf ''`) is a real argument
            // and must survive; only truly absent words are skipped.
            if !current.is_empty() || quoted {
                parts.push(ParsedToken::new(current.clone(), quoted));
                current.clear();
                quoted = false;
            }
        } else if ch == '>' || ch == '<' {
            // Redirect operator. Fold a numeric fd prefix (or `&`) into the
            // operator token so `2>`, `2>>`, `2>&1`, `&>` are one token —
            // matching how bash lexes `2>/dev/null` (fd and `>` are adjacent).
            // A non-fd word before the operator is pushed as a normal arg
            // first (e.g. `echo hi>f` → `echo hi > f`).
            let is_fd_prefix = !current.is_empty()
                && (current.chars().all(|c| c.is_ascii_digit()) || current == "&");
            if !is_fd_prefix && (!current.is_empty() || quoted) {
                parts.push(ParsedToken::new(current.clone(), quoted));
                current.clear();
            }
            let mut op = String::new();
            if is_fd_prefix {
                op.push_str(&current);
                current.clear();
            }
            op.push(ch);
            // Combine `>>`, `<<`, `<>`, `>&`.
            if let Some(&next) = chars.peek() {
                if next == '>' || next == '<' {
                    op.push(chars.next().unwrap());
                } else if ch == '>' && next == '&' {
                    op.push(chars.next().unwrap());
                }
            }
            // `2>&1` / `1>&2`: fold the trailing fd digit into the operator.
            if op.ends_with(">&") {
                if let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() {
                        op.push(chars.next().unwrap());
                    }
                }
            }
            parts.push(ParsedToken::new(op, false));
            quoted = false;
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() || quoted {
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
            } else if ch == '\\'
                && chars.peek().map_or(false, |&n| {
                    n == '"' || n == '\\' || n == '$' || n == '`' || n == '!'
                })
            {
                current.push(chars.next().unwrap());
            }
        } else if ch == '\'' {
            current.push(ch);
            in_single_quote = true;
        } else if ch == '"' {
            current.push(ch);
            in_double_quote = true;
        } else if ch == '\\' {
            // Backslash escape outside quotes. Consume the next char so `\'`
            // does not toggle quote state — this keeps the `'\''` idiom (a
            // literal quote inside a single-quoted string) in sync with bash,
            // so a following `|` still splits the pipeline.
            current.push(ch);
            if let Some(next) = chars.next() {
                current.push(next);
            }
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
/// (Applied per-token in `expand_globs` / `expand_glob_words`.)
fn brace_expand_inner(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut open: Option<usize> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '{' if !in_single && !in_double => {
                if depth == 0 {
                    open = Some(i);
                }
                depth += 1;
            }
            '}' if !in_single && !in_double => {
                if depth == 0 {
                    return vec![input.to_string()];
                }
                depth -= 1;
                if depth == 0 {
                    let o = open.unwrap();
                    let body: String = chars[o + 1..i].iter().collect();
                    let parts = split_brace_parts(&body);
                    // Only `{a,b,...}` or a range `{1..3}` expand. Bare `{}` or
                    // `{a}` stay literal (matches bash), which matters for
                    // tokens like xargs' `-I{}`.
                    let expandable = parts.len() > 1 || is_range(&body);
                    if expandable {
                        let prefix: String = chars[..o].iter().collect();
                        let suffix: String = chars[i + 1..].iter().collect();
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
                    // Not expandable: keep it literal and keep scanning for a
                    // later expandable group.
                    open = None;
                }
            }
            _ => {}
        }
        i += 1;
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
            '{' => {
                depth += 1;
                current.push(c);
            }
            '}' => {
                if depth > 0 {
                    depth -= 1;
                }
                current.push(c);
            }
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
        // Support `{1..10..2}` step form.
        let (end_str, step_str) = match s[pos + 2..].find("..") {
            Some(spos) => (&s[pos + 2..pos + 2 + spos], Some(&s[pos + 2 + spos + 2..])),
            None => (&s[pos + 2..], None),
        };
        let explicit_step: Option<i64> = step_str.and_then(|v| v.parse().ok());

        // Numeric range (with optional leading-zero padding).
        if let (Ok(start), Ok(end)) = (start_str.parse::<i64>(), end_str.parse::<i64>()) {
            // Implicit step: descending when start > end (bash `{5..1}`).
            let step = explicit_step.unwrap_or(if start > end { -1 } else { 1 });
            if step == 0 {
                return vec![s.to_string()];
            }
            let mut out = Vec::new();
            let mut n = start;
            if step > 0 {
                while n <= end {
                    out.push(format_num(n, start_str, end_str));
                    n += step;
                }
            } else {
                while n >= end {
                    out.push(format_num(n, start_str, end_str));
                    n += step;
                }
            }
            return out;
        }

        // Alphabetic range `{a..z}` / `{z..a}`.
        if start_str.len() == 1 && end_str.len() == 1 {
            if let (Some(sc), Some(ec)) = (start_str.chars().next(), end_str.chars().next()) {
                if sc.is_ascii_alphabetic() && ec.is_ascii_alphabetic() {
                    let step = explicit_step.unwrap_or(if sc > ec { -1 } else { 1 });
                    if step == 0 {
                        return vec![s.to_string()];
                    }
                    let mut out = Vec::new();
                    let mut c = sc as u8 as i64;
                    let target = ec as u8 as i64;
                    if step > 0 {
                        while c <= target {
                            out.push((c as u8 as char).to_string());
                            c += step;
                        }
                    } else {
                        while c >= target {
                            out.push((c as u8 as char).to_string());
                            c += step;
                        }
                    }
                    return out;
                }
            }
        }
    }
    vec![s.to_string()]
}

/// Format a numeric range element, preserving leading-zero padding only when
/// a range endpoint itself is zero-padded (e.g. `{01..10}` → 01, 02, ...;
/// `{1..10}` → 1, 2, ...).
fn format_num(n: i64, start_str: &str, end_str: &str) -> String {
    let pad_zero = start_str.starts_with('0') || end_str.starts_with('0');
    if pad_zero {
        let width = start_str.len().max(end_str.len());
        let s = n.to_string();
        let digits = s.trim_start_matches('-').len();
        if digits < width {
            let pad = width - digits;
            let sign = if n < 0 { "-" } else { "" };
            return format!("{}{}{}", sign, "0".repeat(pad), s.trim_start_matches('-'));
        }
    }
    n.to_string()
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

/// Remove one layer of matching surrounding single/double quotes.
fn strip_shell_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let (a, b) = (bytes[0], bytes[bytes.len() - 1]);
        if (a == b'\'' && b == b'\'') || (a == b'"' && b == b'"') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
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
            if c == '\'' {
                in_sq = false;
                i += 1;
                continue;
            }
            current.push(c);
        } else if in_dq {
            if c == '"' {
                in_dq = false;
                i += 1;
                continue;
            }
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

fn connector_delim(c: Connector) -> &'static str {
    match c {
        Connector::AndIf => " && ",
        Connector::OrIf => " || ",
        Connector::Always => "; ",
    }
}

fn combine_loop_segments(segments: Vec<Segment>) -> Vec<Segment> {
    let mut result = Vec::new();
    let segments_vec = segments;
    let mut i = 0;
    while i < segments_vec.len() {
        let text = segments_vec[i].text.trim();
        if (text == "for"
            || text.starts_with("for ")
            || text == "while"
            || text.starts_with("while ")
            || text == "until"
            || text.starts_with("until ")
            || ends_with_pipe_while(text))
            && !contains_word(text, "done")
        {
            let mut combined = segments_vec[i].text.clone();
            let connector = segments_vec[i].connector;
            // A here-string / heredoc may sit on the last `done` segment
            // (`while read x; do …; done <<< word`) — carry the latest one.
            let mut heredoc = segments_vec[i].heredoc.clone();
            let mut heredoc_expand = segments_vec[i].heredoc_expand;
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
                // Count keywords anywhere in the segment: a nested `for`/`if`
                // often follows `then`/`do` in the same segment.
                let opens = count_word(txt, "if")
                    + count_word(txt, "for")
                    + count_word(txt, "while")
                    + count_word(txt, "until");
                let closes = count_word(txt, "fi") + count_word(txt, "done");
                combined.push_str(connector_delim(segments_vec[j].connector));
                combined.push_str(&segments_vec[j].text);
                if segments_vec[j].heredoc.is_some() {
                    heredoc = segments_vec[j].heredoc.clone();
                    heredoc_expand = segments_vec[j].heredoc_expand;
                }
                j += 1;
                depth += opens;
                if closes >= depth {
                    break;
                }
                depth -= closes;
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
        if (text == "case" || text.starts_with("case ")) && !contains_word(text, "esac") {
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
                        break;
                    }
                }
                combined.push_str(connector_delim(segments_vec[j].connector));
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
    s.contains('*') || s.contains('?') || s.contains('[') || has_extglob(s)
}

/// True when the pattern uses an extglob operator (`?(`, `*(`, `+(`, `@(`, `!(`).
fn has_extglob(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i + 1 < c.len() {
        if matches!(c[i], '?' | '*' | '+' | '@' | '!') && c[i + 1] == '(' {
            return true;
        }
        i += 1;
    }
    false
}

fn join_vpath(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", base, name)
    }
}

/// Index just past the `)` matching the `(` at `open`.
fn match_paren(c: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < c.len() {
        match c[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_top_alt(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            '|' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(ch),
        }
    }
    out.push(cur);
    out
}

/// Translate one path-segment glob (with extglob) into regex source (unanchored).
fn glob_seg_source(seg: &str, extglob: bool) -> String {
    let c: Vec<char> = seg.chars().collect();
    let mut r = String::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if extglob && matches!(ch, '?' | '*' | '+' | '@' | '!') && c.get(i + 1) == Some(&'(') {
            if let Some(end) = match_paren(&c, i + 1) {
                let inner: String = c[i + 2..end - 1].iter().collect();
                let alts: Vec<String> = split_top_alt(&inner)
                    .iter()
                    .map(|a| glob_seg_source(a, extglob))
                    .collect();
                let alt = format!("(?:{})", alts.join("|"));
                match ch {
                    '?' => r.push_str(&format!("{alt}?")),
                    '*' => r.push_str(&format!("{alt}*")),
                    '+' => r.push_str(&format!("{alt}+")),
                    '@' => r.push_str(&alt),
                    '!' => r.push_str(&format!("(?:(?!^(?:{})$).*)", alts.join("|"))),
                    _ => {}
                }
                i = end;
                continue;
            }
        }
        match ch {
            '*' => r.push_str(".*"),
            '?' => r.push('.'),
            '[' => {
                r.push('[');
                i += 1;
                if i < c.len() && (c[i] == '!' || c[i] == '^') {
                    r.push('^');
                    i += 1;
                }
                while i < c.len() && c[i] != ']' {
                    r.push(c[i]);
                    i += 1;
                }
                if i < c.len() {
                    r.push(']');
                }
            }
            ch if ".+()^${}|\\".contains(ch) => {
                r.push('\\');
                r.push(ch);
            }
            ch => r.push(ch),
        }
        i += 1;
    }
    r
}

fn compile_glob_seg(seg: &str, extglob: bool, nocase: bool) -> Option<regex::Regex> {
    let src = format!("^{}$", glob_seg_source(seg, extglob));
    regex::RegexBuilder::new(&src)
        .case_insensitive(nocase)
        .build()
        .ok()
}

/// Parse `if` branches from the clause after `if` and before `fi`.
///
/// Keywords (`then`/`elif`/`else`/`fi`) are matched as standalone words, so the
/// standard multi-line form works:
/// ```text
/// if cond
/// then
///   body
/// elif cond2
/// then
///   body2
/// else
///   body3
/// fi
/// ```
/// A branch with an empty condition is the `else` body.
fn parse_if_branches(input: &str) -> Vec<(String, String)> {
    let mut branches: Vec<(String, String)> = Vec::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        // A leading `elif` / `else` starts a new branch.
        if let Some(r) = strip_leading_word(rest, "elif") {
            rest = r;
        } else if let Some(r) = strip_leading_word(rest, "else") {
            branches.push((String::new(), r.trim().to_string()));
            break;
        }
        // Condition runs up to the next `then`.
        let Some(then_pos) = find_keyword(rest, "then") else {
            // No `then`: treat the remainder as an else body.
            branches.push((String::new(), rest.trim().to_string()));
            break;
        };
        let cond = rest[..then_pos]
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();
        let after = rest[then_pos + 4..].trim_start();
        // Body runs up to the next top-level `elif` / `else` / `fi`
        // (skipping nested `if ... fi`).
        match find_if_terminator(after) {
            Some((p, _)) => {
                branches.push((cond, after[..p].trim().to_string()));
                rest = after[p..].trim_start();
            }
            None => {
                branches.push((cond, after.trim().to_string()));
                break;
            }
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
        if (text == "if" || text.starts_with("if ")) && !contains_word(text, "fi") {
            let mut combined = segments_vec[i].text.clone();
            let connector = segments_vec[i].connector;
            let heredoc = segments_vec[i].heredoc.clone();
            let heredoc_expand = segments_vec[i].heredoc_expand;
            let mut depth = 1usize;
            let mut j = i + 1;
            while j < segments_vec.len() {
                let txt = segments_vec[j].text.trim();
                // Count standalone `if`/`fi` anywhere in the segment (a nested
                // `if` often follows `then` in the same segment).
                let ifs = count_word(txt, "if");
                let fis = count_word(txt, "fi");
                combined.push_str(connector_delim(segments_vec[j].connector));
                combined.push_str(&segments_vec[j].text);
                j += 1;
                depth = depth + ifs;
                if fis >= depth {
                    depth = 0;
                    break;
                }
                depth -= fis;
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
    let (name, name_end_idx) = if let Some(rest) = s.strip_prefix("function ") {
        let offset = s.len() - rest.len();
        let trimmed = rest.trim_start();
        let lead = rest.len() - trimmed.len();
        let name_end = trimmed
            .find(|c: char| c.is_whitespace() || c == '{' || c == '(')
            .unwrap_or(trimmed.len());
        (&trimmed[..name_end], offset + lead + name_end)
    } else {
        let paren_pos = s.find("()")?;
        (s[..paren_pos].trim(), paren_pos + 2)
    };
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return None;
    }
    let brace_start = s[name_end_idx..].find('{')?;
    let body_start = name_end_idx + brace_start + 1;
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
        let dir =
            std::env::temp_dir().join(format!("fastshell_rt_test_{}_{}", std::process::id(), n));
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
            PyInvocation::Script(
                "script.py".to_string(),
                vec!["a".to_string(), "b".to_string()]
            )
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
            PyInvocation::Module(
                "pytest".to_string(),
                vec!["-q".to_string(), "test_x.py".to_string()]
            )
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
    fn test_parse_command_redirect_attached() {
        // bash treats `2>/dev/null` as fd 2 redirect to /dev/null (no space).
        let parts = parse_command("ls 2>/dev/null");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["ls", "2>", "/dev/null"]);
    }

    #[test]
    fn test_parse_command_redirect_merge_attached() {
        let parts = parse_command("cmd 2>&1");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["cmd", "2>&1"]);
    }

    #[test]
    fn test_parse_command_redirect_append_attached() {
        let parts = parse_command("echo hi 2>>err.log");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["echo", "hi", "2>>", "err.log"]);
    }

    #[test]
    fn test_parse_command_redirect_stdout_attached() {
        let parts = parse_command("echo hi >out.txt");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["echo", "hi", ">", "out.txt"]);
    }

    #[test]
    fn test_parse_command_redirect_both_attached() {
        let parts = parse_command("cat x &>out.txt");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["cat", "x", "&>", "out.txt"]);
    }

    #[test]
    fn test_parse_command_redirect_after_arg() {
        // `echo hi>f` → hi is a normal arg, `>` redirects stdout.
        let parts = parse_command("echo hi>f");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["echo", "hi", ">", "f"]);
    }

    #[test]
    fn test_parse_command_redirect_in_quotes_not_split() {
        // `>` inside quotes is literal, not a redirect.
        let parts = parse_command("echo \"a > b\"");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["echo", "a > b"]);
    }

    #[test]
    fn test_parse_command_redirect_with_space_still_works() {
        // `2> /dev/null` (space) must still tokenize the same way.
        let parts = parse_command("ls 2> /dev/null");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["ls", "2>", "/dev/null"]);
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
        let content = rt
            .shell
            .vfs
            .read_to_string("/out.txt", &rt.shell.cwd)
            .unwrap();
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_redirect_stdout_append() {
        let mut rt = setup_runtime();
        rt.execute("echo first > /append.txt");
        let r = rt.execute("echo second >> /append.txt");
        assert_eq!(r.exit_code, 0);
        let content = rt
            .shell
            .vfs
            .read_to_string("/append.txt", &rt.shell.cwd)
            .unwrap();
        assert!(content.contains("first"));
        assert!(content.contains("second"));
    }

    #[test]
    fn test_redirect_stdin() {
        let mut rt = setup_runtime();
        rt.shell
            .vfs
            .write("/input.txt", &rt.shell.cwd, "hello stdin")
            .unwrap();
        let r = rt.execute("cat < /input.txt");
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.contains("hello stdin"));
    }

    #[test]
    fn test_redirect_stderr_to_file() {
        let mut rt = setup_runtime();
        // Use a command that definitely produces stderr
        let r = rt.execute("cat /nonexistent_path_xyz 2> /err.txt");
        assert!(
            r.stderr.is_empty(),
            "stderr should be empty after redirect, got: {:?}",
            r.stderr
        );
        let content = rt
            .shell
            .vfs
            .read_to_string("/err.txt", &rt.shell.cwd)
            .unwrap_or_else(|_| String::new());
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
        let content = rt
            .shell
            .vfs
            .read_to_string("/both.txt", &rt.shell.cwd)
            .unwrap();
        assert_eq!(content.trim(), "merged");
    }

    #[test]
    fn test_redirect_pipeline_with_redirect() {
        let mut rt = setup_runtime();
        let r = rt.execute("echo hello | grep hello > /pipe_out.txt");
        assert_eq!(r.exit_code, 0);
        let content = rt
            .shell
            .vfs
            .read_to_string("/pipe_out.txt", &rt.shell.cwd)
            .unwrap();
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_pipeline_stderr_redirect_to_dev_null() {
        let mut rt = setup_runtime();
        // Pipeline where first stage has stderr redirected — simulate model's pattern
        let r = rt.execute("cat /nonexistent_file 2> /dev/null | head -20");
        assert_eq!(
            r.exit_code, 0,
            "exit code should be 0 even with 2>/dev/null in pipe"
        );
    }

    #[test]
    fn test_compound_with_pipe_and_stderr_redirect() {
        let mut rt = setup_runtime();
        // Exact pattern from the session: ; chain with pipeline containing 2>
        let r = rt.execute("echo before; cat /nonexistent 2> /dev/null; echo after");
        assert_eq!(
            r.exit_code, 0,
            "compound command with 2>/dev/null should not crash"
        );
        assert!(r.stdout.contains("before"));
        assert!(r.stdout.contains("after"));
    }

    #[test]
    fn test_compound_pipe_head_with_stderr_redirect() {
        let mut rt = setup_runtime();
        // Pattern from session: ls piped to head, with stderr redirect elsewhere
        let r = rt.execute("ls /tmp 2>/dev/null | head -5");
        assert_eq!(r.exit_code, 0);
        assert!(
            r.stderr.is_empty(),
            "stderr should be redirected, got: {}",
            r.stderr
        );
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
        let out = rt
            .shell
            .vfs
            .read_to_string("/stdout.txt", &rt.shell.cwd)
            .unwrap();
        let err = rt
            .shell
            .vfs
            .read_to_string("/stderr.txt", &rt.shell.cwd)
            .unwrap();
        assert!(out.contains("stdout_msg"));
        assert!(!err.is_empty(), "stderr file should contain error message");
    }

    fn rargs(v: &[&str]) -> Vec<(String, bool)> {
        v.iter().map(|s| (s.to_string(), false)).collect()
    }
    fn rvals(clean: &[(String, bool)]) -> Vec<String> {
        clean.iter().map(|t| t.0.clone()).collect()
    }

    #[test]
    fn test_parse_redirects_stdout() {
        let args = rargs(&["echo", "hello", ">", "file.txt"]);
        let (clean, spec) = parse_redirects(&args);
        assert_eq!(rvals(&clean), vec!["echo", "hello"]);
        assert_eq!(spec.stdout_file, Some(("file.txt".to_string(), false)));
    }

    #[test]
    fn test_parse_redirects_append() {
        let args = rargs(&["echo", "hello", ">>", "file.txt"]);
        let (_clean, spec) = parse_redirects(&args);
        assert_eq!(spec.stdout_file, Some(("file.txt".to_string(), true)));
    }

    #[test]
    fn test_parse_redirects_rightmost_wins() {
        let args = rargs(&["cmd", ">", "first.txt", ">", "second.txt"]);
        let (clean, spec) = parse_redirects(&args);
        assert_eq!(rvals(&clean), vec!["cmd"]);
        assert_eq!(spec.stdout_file, Some(("second.txt".to_string(), false)));
    }

    #[test]
    fn test_parse_redirects_no_redirects() {
        let args = rargs(&["ls", "-la"]);
        let (clean, spec) = parse_redirects(&args);
        assert_eq!(rvals(&clean), vec!["ls", "-la"]);
        assert!(spec.stdout_file.is_none());
        assert!(spec.stderr_file.is_none());
        assert!(spec.stdin_file.is_none());
        assert!(!spec.merge_stderr_to_stdout);
        assert!(!spec.merge_stdout_to_stderr);
    }

    #[test]
    fn test_parse_redirects_quoted_gt_is_literal() {
        // A quoted `>` must stay an argument, not become a redirect.
        let args = vec![("echo".to_string(), false), (">".to_string(), true)];
        let (clean, spec) = parse_redirects(&args);
        assert_eq!(rvals(&clean), vec!["echo", ">"]);
        assert!(spec.stdout_file.is_none());
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
        let out = rt.execute("cat > f.txt << 'EOF'\ndata\nEOF\ncat f.txt && echo done");
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
        let (n, v, rest, app) = take_assignment("X=5").unwrap();
        assert_eq!((n.as_str(), v.as_str(), rest, app), ("X", "5", "", false));
        let (n, v, rest, _) = take_assignment("NAME='a b' cmd").unwrap();
        assert_eq!(
            (n.as_str(), v.as_str(), rest.trim()),
            ("NAME", "a b", "cmd")
        );
        // Concatenated shell-word value (quoted + bare segments).
        let (n, v, rest, _) = take_assignment("s=\"x\"y").unwrap();
        assert_eq!((n.as_str(), v.as_str(), rest), ("s", "xy", ""));
        let (n, v, rest, _) = take_assignment("s=x\"x\"").unwrap();
        assert_eq!((n.as_str(), v.as_str(), rest), ("s", "xx", ""));
        // `+=` append assignment.
        let (n, v, rest, app) = take_assignment("s+=y").unwrap();
        assert_eq!((n.as_str(), v.as_str(), rest, app), ("s", "y", "", true));
        let (n, v, rest, app) = take_assignment("s+=\"a b\" c").unwrap();
        assert_eq!(
            (n.as_str(), v.as_str(), rest.trim(), app),
            ("s", "a b", "c", true)
        );
        assert!(take_assignment("ls -la").is_none());
        assert!(take_assignment("dd if=/dev/zero").is_none());
        assert!(take_assignment("3X=5").is_none());
        assert!(take_assignment("s+ y").is_none());
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
        assert!(
            out_trimmed.is_empty() || out_trimmed == "src/**/*.py",
            "expected empty or literal, got: {}",
            out_trimmed
        );
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
        let content = rt
            .shell
            .vfs
            .read_to_string("/out.txt", &rt.shell.cwd)
            .unwrap();
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
    fn test_redirect_dev_null_does_not_create_file() {
        let mut rt = setup_runtime();
        let _ = rt.execute("echo hello 2> /dev/null");
        assert!(
            !rt.shell.vfs.exists("/dev/null", &rt.shell.cwd),
            "/dev/null should not be created as a regular file"
        );
    }

    #[test]
    fn test_redirect_dev_null_stdout_does_not_create_file() {
        let mut rt = setup_runtime();
        let _ = rt.execute("echo hello > /dev/null");
        assert!(!rt.shell.vfs.exists("/dev/null", &rt.shell.cwd));
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
        eprintln!(
            "FOR DEBUG: stdout='{}' stderr='{}' exit={}",
            out.stdout, out.stderr, out.exit_code
        );
        assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
        // At minimum the loop should execute and produce output
        assert!(
            !out.stdout.is_empty(),
            "stdout is empty: stderr='{}'",
            out.stderr
        );
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
    fn test_for_loop_expands_glob() {
        let mut rt = mk_rt();
        rt.execute("mkdir -p gtest && touch gtest/a.jpg gtest/b.jpg gtest/c.txt");
        let out = rt.execute("cd gtest && for f in *.jpg; do echo $f; done");
        assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
        assert!(out.stdout.contains("a.jpg"), "stdout: {}", out.stdout);
        assert!(out.stdout.contains("b.jpg"), "stdout: {}", out.stdout);
        assert!(!out.stdout.contains("c.txt"), "stdout: {}", out.stdout);
        assert!(
            !out.stdout.contains("*.jpg"),
            "glob not expanded: {}",
            out.stdout
        );
    }

    #[test]
    fn test_for_loop_quoted_glob_not_expanded() {
        let mut rt = mk_rt();
        rt.execute("mkdir -p gtest2 && touch gtest2/a.jpg");
        // Quoted word list → the glob stays literal in `f`; quoting `"$f"` in the
        // body prevents re-globbing on expansion (same as bash).
        let out = rt.execute("cd gtest2 && for f in \"*.jpg\"; do echo \"$f\"; done");
        assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
        assert!(out.stdout.contains("*.jpg"), "stdout: {}", out.stdout);
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
        let out =
            rt.execute("case banana in apple) echo a;; banana) echo b;; *) echo other;; esac");
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
    fn test_if_multiline_then_own_line() {
        let mut rt = mk_rt();
        let out = rt.execute("if true\nthen\n  echo yes\nfi");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout.trim(), "yes");
    }

    #[test]
    fn test_if_else_multiline() {
        let mut rt = mk_rt();
        let out = rt.execute("if false\nthen\n  echo a\nelse\n  echo c\nfi");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout.trim(), "c");
    }

    #[test]
    fn test_if_elif_else_multiline() {
        let mut rt = mk_rt();
        let out =
            rt.execute("if false\nthen\n  echo a\nelif true\nthen\n  echo b\nelse\n  echo c\nfi");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout.trim(), "b");
    }

    #[test]
    fn test_case_multiline() {
        let mut rt = mk_rt();
        let out = rt.execute(
            "case banana in\n  apple) echo a;;\n  banana) echo b;;\n  *) echo other;;\nesac",
        );
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout.trim(), "b");
    }

    #[test]
    fn test_for_multiline_do_newline() {
        let mut rt = mk_rt();
        let out = rt.execute("for u in a b c; do\n  echo \"v=$u\"\ndone");
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("v=a"), "stdout={}", out.stdout);
        assert!(out.stdout.contains("v=c"), "stdout={}", out.stdout);
    }

    #[test]
    fn test_for_with_if_inside() {
        let mut rt = mk_rt();
        let out = rt
            .execute("for f in a b; do if test $f = b; then echo found; else echo skip; fi; done");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("skip"));
        assert!(out.stdout.contains("found"));
    }

    #[test]
    fn test_while_with_for_inside() {
        let mut rt = mk_rt();
        let out =
            rt.execute("i=0; while test $i -lt 1; do for x in hello; do echo $x; done; i=1; done");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("hello"));
    }

    #[test]
    fn test_combine_loop_segments_passthrough() {
        let segs = vec![Segment {
            connector: Connector::Always,
            text: "echo hi".into(),
            heredoc: None,
            heredoc_expand: false,
        }];
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
        assert!(
            out.exit_code == 0 || out.exit_code == 1,
            "unexpected exit {}: stderr={}",
            out.exit_code,
            out.stderr
        );
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
        assert_ne!(
            out.exit_code, 0,
            "shell functions are not yet supported in pipelines"
        );
    }

    // ── Pipeline connector preservation tests ───────────────────

    #[test]
    fn test_and_if_preserved_in_if_body() {
        let mut rt = mk_rt();
        // In if body, cmd1 && cmd2: cmd1 fails, cmd2 should NOT execute.
        rt.execute("echo first > /out.txt");
        let _ = rt.execute(
            "if true; then grep nosuch /out.txt && echo 'should not appear' > /out2.txt; fi",
        );
        // grep fails (returncode 1), && skips echo. /out2.txt should not exist.
        assert!(
            !rt.shell.vfs.exists("/out2.txt", &rt.shell.cwd),
            "&& should skip second command when first fails"
        );
    }

    #[test]
    fn test_or_if_preserved_in_while_body() {
        let mut rt = mk_rt();
        // cmd1 || cmd2: cmd1 fails, cmd2 should execute.
        rt.execute("echo ok > /test.txt");
        let _ = rt.execute("grep nosuch /test.txt || echo 'fallback' > /fallback.txt");
        assert!(
            rt.shell.vfs.exists("/fallback.txt", &rt.shell.cwd),
            "|| should execute second command when first fails"
        );
    }

    #[test]
    fn test_and_if_preserved_in_for_body() {
        let mut rt = mk_rt();
        rt.execute("echo pass > /one.txt");
        // for body: cat /one.txt succeeds → && executes echo.
        let _ = rt.execute("for f in one; do cat /$f.txt && echo 'ok' > /log.txt; done");
        let content = rt
            .shell
            .vfs
            .read_to_string("/log.txt", &rt.shell.cwd)
            .unwrap_or_default();
        assert!(
            content.contains("ok"),
            "&& should execute second command when first succeeds"
        );
    }

    #[test]
    fn test_and_if_skips_after_failure() {
        let mut rt = mk_rt();
        // cat fails → && prevents the second command.
        let _ = rt.execute("cat /nonexist && echo 'nope' > /shouldnotexist.txt");
        assert!(
            !rt.shell.vfs.exists("/shouldnotexist.txt", &rt.shell.cwd),
            "&& must skip second command when first fails"
        );
    }

    // ── Pipeline redirect tests ─────────────────────────────────

    #[test]
    fn test_pipeline_merge_stderr_on_first_stage() {
        let mut rt = mk_rt();
        // cmd1 2>&1 | cmd2 — stderr from cmd1 should merge into the pipe.
        let out = rt.execute("cat /noexist 2>&1 | wc -l");
        assert_eq!(out.exit_code, 0);
        // The stderr from cat becomes stdout, passes through pipe, wc counts >= 1 line.
        let lines: i32 = out.stdout.trim().parse().unwrap_or(0);
        assert!(lines >= 1, "2>&1 should merge stderr into pipe");
    }

    #[test]
    fn test_pipeline_stderr_redirect_on_middle_stage() {
        let mut rt = mk_rt();
        // cmd1 | cmd2 2> /err.txt | cmd3 — stderr from cmd2 goes to file.
        let out = rt.execute("echo hello | cat /noexist 2> /err.txt | wc -c");
        // cat fails (stderr → /err.txt), wc -c counts stdin from pipe (0 bytes).
        let content = rt
            .shell
            .vfs
            .read_to_string("/err.txt", &rt.shell.cwd)
            .unwrap_or_default();
        assert!(
            !content.is_empty(),
            "stderr redirect should capture error to file"
        );
    }

    #[test]
    fn test_pipeline_stdout_redirect_on_last_stage() {
        let mut rt = mk_rt();
        let out = rt.execute("echo hello | tr a-z A-Z > /upper.txt");
        assert_eq!(out.exit_code, 0);
        let content = rt
            .shell
            .vfs
            .read_to_string("/upper.txt", &rt.shell.cwd)
            .unwrap_or_default();
        assert_eq!(content.trim(), "HELLO");
    }

    // ── Pipeline quote-awareness tests ──────────────────────────

    #[test]
    fn test_pipeline_quoted_pipe_not_split() {
        let mut rt = mk_rt();
        // | inside single quotes: should NOT be treated as a pipe.
        let out = rt.execute("echo 'a|b'");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "a|b", "quoted | must not split pipeline");
    }

    #[test]
    fn test_pipeline_double_quoted_pipe_not_split() {
        let mut rt = mk_rt();
        let out = rt.execute("echo \"x|y\"");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "x|y");
    }

    // ── Pipeline cancellation tests ─────────────────────────────

    #[test]
    fn test_pipeline_receives_cancel_flag() {
        let mut rt = mk_rt();
        // Set cancel before executing pipeline — should exit quickly with 143.
        rt.cancel.store(true, Ordering::SeqCst);
        let out = rt.execute("echo hello | cat");
        assert_eq!(out.exit_code, 143, "pre-set cancel should abort pipeline");
    }

    // ── Pipeline basic correctness tests ────────────────────────

    #[test]
    fn test_pipeline_three_stage() {
        let mut rt = mk_rt();
        let out = rt.execute("echo \"a\nb\nc\" | grep a | wc -l");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "1");
    }

    #[test]
    fn test_pipeline_with_redirects_and_pipe() {
        let mut rt = mk_rt();
        // cat < file | grep x | wc -l > out.txt
        rt.execute("echo \"apple\nbanana\napple\" > /data.txt");
        let out = rt.execute("cat < /data.txt | grep apple | wc -l > /count.txt");
        let content = rt
            .shell
            .vfs
            .read_to_string("/count.txt", &rt.shell.cwd)
            .unwrap_or_default();
        assert_eq!(content.trim(), "2");
    }

    // ─────── /dev/null 专项测试 ───────

    #[test]
    fn test_dev_null_stderr_on_error() {
        let mut rt = mk_rt();
        // Error-producing command with stderr redirected to /dev/null
        let r = rt.execute("cat /no_such_file 2> /dev/null");
        assert_eq!(
            r.exit_code, 1,
            "cat non-existent file should return exit code 1"
        );
        assert!(
            r.stderr.is_empty(),
            "stderr should be discarded, got: {}",
            r.stderr
        );
        assert!(
            !rt.shell.vfs.exists("/dev/null", &rt.shell.cwd),
            "/dev/null should not be created"
        );
    }

    #[test]
    fn test_dev_null_both_redirect() {
        let mut rt = mk_rt();
        // &> redirect to /dev/null
        let r = rt.execute("cat /no_such_file &> /dev/null");
        assert_eq!(r.exit_code, 1);
        assert!(r.stdout.is_empty());
        assert!(r.stderr.is_empty());
        assert!(!rt.shell.vfs.exists("/dev/null", &rt.shell.cwd));
    }

    #[test]
    fn test_dev_null_does_not_accumulate() {
        let mut rt = mk_rt();
        // Repeated 2>/dev/null should not create a growing file
        for _ in 0..5 {
            let _ = rt.execute("cat /no_such_file 2> /dev/null");
        }
        assert!(
            !rt.shell.vfs.exists("/dev/null", &rt.shell.cwd),
            "repeated 2>/dev/null should not create file"
        );
    }

    #[test]
    fn test_dev_null_in_pipeline_with_empty_input() {
        let mut rt = mk_rt();
        // When previous stage produces no stdout, pipe to next is empty
        let r = rt.execute("cat /no_such_file 2> /dev/null | grep x");
        assert_eq!(r.exit_code, 1, "grep with no match should return 1");
        assert!(
            r.stderr.is_empty(),
            "pipeline should not panic with 2>/dev/null"
        );
    }

    #[test]
    fn test_dev_null_in_compound_chain() {
        let mut rt = mk_rt();
        // Multiple commands with 2>/dev/null in semicolon chain
        let r = rt
            .execute("echo a; cat /noexist 2> /dev/null; echo b; ls /noexist 2> /dev/null; echo c");
        assert_eq!(
            r.exit_code, 0,
            "compound chain should complete normally, got stderr: {}",
            r.stderr
        );
        assert!(r.stdout.contains("a"), "missing a");
        assert!(r.stdout.contains("b"), "missing b");
        assert!(r.stdout.contains("c"), "missing c");
        assert!(
            r.stderr.is_empty(),
            "stderr should be gone, got: {}",
            r.stderr
        );
        assert!(!rt.shell.vfs.exists("/dev/null", &rt.shell.cwd));
    }

    #[test]
    fn test_dev_null_stdout_to_file_stderr_to_null() {
        let mut rt = mk_rt();
        // stdout to real file, stderr to /dev/null
        let r = rt.execute("echo ok > /dest.txt 2> /dev/null");
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.is_empty());
        assert!(r.stderr.is_empty());
        assert!(
            rt.shell.vfs.exists("/dest.txt", &rt.shell.cwd),
            "/dest.txt should exist"
        );
        let content = rt
            .shell
            .vfs
            .read_to_string("/dest.txt", &rt.shell.cwd)
            .unwrap();
        assert_eq!(content.trim(), "ok");
        assert!(!rt.shell.vfs.exists("/dev/null", &rt.shell.cwd));
    }

    #[test]
    fn test_dev_null_in_three_stage_pipeline() {
        let mut rt = mk_rt();
        // Three-stage pipeline with stderr suppression on stage 1
        rt.execute("echo \"apple\nbanana\napricot\" > /fruits.txt");
        let r = rt.execute("cat /fruits.txt 2> /dev/null | grep ap | wc -l");
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout.trim(), "2", "should count apple and apricot");
    }

    #[test]
    fn test_pipeline_with_stderr_redirect_to_real_file() {
        let mut rt = mk_rt();
        // Pipeline stage with stderr to a real file (not /dev/null)
        let r = rt.execute("cat /noexist 2> /errors.txt | head -5");
        assert!(
            rt.shell.vfs.exists("/errors.txt", &rt.shell.cwd),
            "stderr should be written to real file"
        );
        let err = rt
            .shell
            .vfs
            .read_to_string("/errors.txt", &rt.shell.cwd)
            .unwrap();
        assert!(!err.is_empty(), "error file should contain content");
    }

    // ─────── 连写重定向（对齐 bash） + python 重定向剥离 ───────

    #[test]
    fn test_redirect_attached_dev_null_stderr() {
        let mut rt = mk_rt();
        // `2>/dev/null` (no space) — the LLM's most common spelling.
        let r = rt.execute("cat /no_such_file 2>/dev/null");
        assert_eq!(r.exit_code, 1);
        assert!(
            r.stderr.is_empty(),
            "stderr should be discarded, got: {}",
            r.stderr
        );
        assert!(!rt.shell.vfs.exists("/dev/null", &rt.shell.cwd));
    }

    #[test]
    fn test_redirect_attached_stdout_to_file() {
        let mut rt = mk_rt();
        let r = rt.execute("echo hello >/out.txt");
        assert_eq!(r.exit_code, 0);
        assert!(
            r.stdout.is_empty(),
            "stdout should go to file, got: {}",
            r.stdout
        );
        let content = rt
            .shell
            .vfs
            .read_to_string("/out.txt", &rt.shell.cwd)
            .unwrap();
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_which_redirect_attached() {
        let mut rt = mk_rt();
        // Reproduces the session's `which pip 2>/dev/null` → "2>/dev/null not found".
        let r = rt.execute("which pip 2>/dev/null");
        assert!(
            !r.stderr.contains("2>/dev/null"),
            "redirect must be parsed, got: {}",
            r.stderr
        );
    }

    #[test]
    fn test_python_c_redirect_merge_attached() {
        let mut rt = setup_runtime();
        // `python3 -c "..." 2>&1` must not leak `2>&1` into the code.
        let r = rt.execute("python3 -c \"print('ok')\" 2>&1");
        assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
        assert!(r.stdout.contains("ok"), "stdout={}", r.stdout);
        assert!(
            !r.stderr.contains("SyntaxError"),
            "stderr should not have SyntaxError: {}",
            r.stderr
        );
    }

    #[test]
    fn test_python_c_redirect_stderr_null_attached() {
        let mut rt = setup_runtime();
        // Erroring python + stderr → /dev/null (no space).
        let r = rt.execute("python3 -c \"import nonexistent_mod_xyz\" 2>/dev/null");
        assert_eq!(r.exit_code, 1);
        assert!(
            r.stderr.is_empty(),
            "stderr should be discarded, got: {}",
            r.stderr
        );
    }

    // ─────── 子 shell / 命令组 / env / 花括号 ───────

    #[test]
    fn test_subshell_isolates_cwd() {
        let mut rt = mk_rt();
        rt.execute("mkdir -p /subdir");
        // ( cd /subdir && pwd ) then pwd again should still be /.
        let r = rt.execute("(cd /subdir && pwd); pwd");
        assert!(
            r.stdout.contains("/subdir"),
            "subshell pwd should be /subdir: {}",
            r.stdout
        );
        // After the subshell, cwd is restored to /.
        let pwd = rt.execute("pwd");
        assert_eq!(
            pwd.stdout.trim(),
            "/",
            "cwd must be restored after subshell, got: {}",
            pwd.stdout
        );
    }

    #[test]
    fn test_command_group_propagates_cwd() {
        let mut rt = mk_rt();
        rt.execute("mkdir -p /subdir");
        // { cd /subdir; } propagates the cd to the current shell.
        rt.execute("{ cd /subdir; }");
        let pwd = rt.execute("pwd");
        assert_eq!(
            pwd.stdout.trim(),
            "/subdir",
            "command group cd should propagate, got: {}",
            pwd.stdout
        );
    }

    #[test]
    fn test_group_with_redirect() {
        let mut rt = mk_rt();
        // Subshell + trailing redirects (previously degraded to `(echo` → command not found).
        let r = rt.execute("(echo out; echo err 1>&2) > o.txt 2> e.txt");
        assert_eq!(r.exit_code, 0, "stderr: {}", r.stderr);
        let o = rt.execute("cat o.txt");
        assert!(o.stdout.contains("out"), "o.txt: {:?}", o.stdout);
        let e = rt.execute("cat e.txt");
        assert!(e.stdout.contains("err"), "e.txt: {:?}", e.stdout);
        // Command group + redirect.
        let r = rt.execute("{ echo a; echo b; } > f.txt");
        assert_eq!(r.exit_code, 0, "stderr: {}", r.stderr);
        let f = rt.execute("cat f.txt");
        assert!(
            f.stdout.contains('a') && f.stdout.contains('b'),
            "f.txt: {:?}",
            f.stdout
        );
    }

    #[test]
    fn test_source_defines_functions() {
        let mut rt = mk_rt();
        rt.execute(
            "printf 'greet() { echo HI-$1; }\\nrc_is() { [ \"$1\" = \"$2\" ]; }\\n' > lib.sh",
        );
        let s = rt.execute(". ./lib.sh");
        assert_eq!(s.exit_code, 0, "source stderr: {:?}", s.stderr);
        let r = rt.execute("greet world");
        assert!(
            r.stdout.contains("HI-world"),
            "sourced function not callable; stdout={:?} stderr={:?}",
            r.stdout,
            r.stderr
        );
        let r = rt.execute("rc_is a a; echo rc=$?");
        assert!(r.stdout.contains("rc=0"), "rc_is stdout={:?}", r.stdout);
    }

    #[test]
    fn test_sh_script_carries_sourced_functions() {
        let mut rt = mk_rt();
        rt.execute(
            "printf 'greet() { echo HI-$1; }\\nrc_is() { [ \"$1\" = \"$2\" ]; }\\n' > lib.sh",
        );
        rt.execute("printf '. ./lib.sh\\ngreet world\\nrc_is a a; echo rc=$?\\n' > t.sh");
        let r = rt.execute("sh t.sh");
        assert!(
            r.stdout.contains("HI-world"),
            "sh-script sourced fn; stdout={:?} stderr={:?}",
            r.stdout,
            r.stderr
        );
        assert!(
            r.stdout.contains("rc=0"),
            "rc_is in sh-script stdout={:?}",
            r.stdout
        );
    }

    #[test]
    fn test_cmdsubst_and_concat_assign() {
        let mut rt = mk_rt();
        // `$( … && … )` inside an assignment.
        let r = rt.execute("x=$(echo a && echo b); echo [$x]");
        assert!(
            r.stdout.contains('a') && r.stdout.contains('b'),
            "x=({:?}) stderr={:?}",
            r.stdout,
            r.stderr
        );
        // `$( … | … )`.
        let r = rt.execute("y=$(echo abc | cut -c 2); echo [$y]");
        assert!(
            r.stdout.contains("[b]"),
            "y=({:?}) stderr={:?}",
            r.stdout,
            r.stderr
        );
    }

    #[test]
    fn test_symlink_ls_cat() {
        let mut rt = mk_rt();
        rt.execute("printf 'hello\\n' > target.txt");
        rt.execute("ln -s target.txt mylink");
        let r = rt.execute("readlink mylink");
        assert!(r.stdout.contains("target.txt"), "readlink={:?}", r.stdout);
        let r = rt.execute("cat mylink");
        assert!(
            r.stdout.contains("hello"),
            "cat symlink stdout={:?} stderr={:?}",
            r.stdout,
            r.stderr
        );
        let r = rt.execute("ls -l mylink");
        assert_eq!(r.exit_code, 0, "ls -l symlink stderr={:?}", r.stderr);
        // Absolute target (the session's case): symlink → /target.txt.
        rt.execute("mkdir -p sub && cp target.txt sub/g.txt");
        rt.execute("ln -s /sub/g.txt abslink");
        let r = rt.execute("cat abslink");
        assert!(
            r.stdout.contains("hello"),
            "cat abs-target symlink stdout={:?} stderr={:?}",
            r.stdout,
            r.stderr
        );
    }

    #[test]
    fn test_redirect_created_file_visible() {
        let mut rt = mk_rt();
        rt.execute("printf 'L1\\n' > z.txt");
        let r = rt.execute("gzip -c z.txt > z.gz");
        assert_eq!(r.exit_code, 0, "gzip stderr: {:?}", r.stderr);
        let r = rt.execute("ls z.gz");
        assert_eq!(r.exit_code, 0, "ls z.gz stderr: {:?}", r.stderr);
        let r = rt.execute("cp z.gz z2.gz; ls z2.gz");
        assert_eq!(r.exit_code, 0, "cp redirect file stderr: {:?}", r.stderr);
    }

    #[test]
    fn test_split_prefix_arg() {
        let mut rt = mk_rt();
        rt.execute("printf 'a\\nb\\nc\\nd\\n' > in.txt");
        let r = rt.execute("split -l 2 in.txt bb_");
        assert_eq!(r.exit_code, 0, "split stderr: {:?}", r.stderr);
        let r = rt.execute("ls bb_aa");
        assert_eq!(r.exit_code, 0, "split part missing: stderr={:?}", r.stderr);
    }

    #[test]
    fn test_sh_script_uses_current_cwd() {
        let mut rt = mk_rt();
        rt.execute("mkdir -p p1 p2");
        rt.execute("printf 'echo FROM_P1\\n' > p1/t.sh");
        rt.execute("cd p2"); // leave cwd elsewhere
        let r = rt.execute("cd /p1 && sh t.sh");
        assert!(
            r.stdout.contains("FROM_P1"),
            "sh script cwd; stdout={:?} stderr={:?}",
            r.stdout,
            r.stderr
        );
    }

    #[test]
    fn test_env_prefix_assignment() {
        let mut rt = mk_rt();
        // `env VAR=x` (assignment-only) is equivalent to `VAR=x`: it sets the
        // shell variable for subsequent commands.
        rt.execute("env MYVAR=hello");
        let r = rt.execute("echo $MYVAR");
        assert!(
            r.stdout.contains("hello"),
            "env VAR=x should set the var, got: {}",
            r.stdout
        );
    }

    #[test]
    fn test_env_unset() {
        let mut rt = mk_rt();
        rt.execute("MYVAR=hello");
        assert!(rt.execute("echo $MYVAR").stdout.contains("hello"));
        rt.execute("env -u MYVAR");
        let r = rt.execute("echo $MYVAR");
        assert!(
            !r.stdout.contains("hello"),
            "env -u should unset the var, got: {}",
            r.stdout
        );
    }

    #[test]
    fn test_env_ignore_environment() {
        let mut rt = mk_rt();
        rt.execute("MYVAR=hello");
        assert!(rt.execute("echo $MYVAR").stdout.contains("hello"));
        rt.execute("env -i");
        let r = rt.execute("echo $MYVAR");
        assert!(
            !r.stdout.contains("hello"),
            "env -i should clear vars, got: {}",
            r.stdout
        );
    }

    #[test]
    fn test_brace_range_descending() {
        let r = brace_expand_inner("{5..1}");
        assert_eq!(r, vec!["5", "4", "3", "2", "1"]);
    }

    #[test]
    fn test_brace_range_alphabetic() {
        let r = brace_expand_inner("{a..e}");
        assert_eq!(r, vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn test_brace_range_step() {
        let r = brace_expand_inner("{1..10..3}");
        assert_eq!(r, vec!["1", "4", "7", "10"]);
    }

    #[test]
    fn test_brace_range_zero_padding() {
        let r = brace_expand_inner("{01..03}");
        assert_eq!(r, vec!["01", "02", "03"]);
    }
}
