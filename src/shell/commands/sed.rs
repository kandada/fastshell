// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const SED_HELP_TEXT: &str = "\
sed: stream editor
Usage: sed [OPTIONS] 'script' [FILE...]
Options:
  -e SCRIPT   Add script to commands
  -i          Edit files in-place
  -n          Suppress automatic printing
  -ni/-in     Combine -n and -i
  -i.bak      In-place with backup suffix
  -h, --help  Show this help message
";

impl Shell {
    pub fn cmd_sed(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(SED_HELP_TEXT.to_string());
        }
        let mut expressions: Vec<String> = Vec::new();
        let mut files = Vec::new();
        let mut in_place = false;
        let mut backup_suffix: Option<String> = None;
        let mut quiet = false; // -n: suppress automatic printing
        let mut null_data = false; // -z: NUL-separated records

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-e" => {
                    if i + 1 < args.len() {
                        expressions.push(args[i + 1].to_string());
                        i += 1;
                    }
                }
                // `-f FILE` / `--file=FILE`: read the script from a file (one
                // command per non-empty, non-comment line).
                "-f" | "--file" => {
                    if i + 1 < args.len() {
                        i += 1;
                        match self.read_text_lossy(args[i]) {
                            Ok(c) => expressions.extend(
                                c.lines()
                                    .map(str::trim)
                                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                    .map(|l| l.to_string()),
                            ),
                            Err(e) => {
                                return CommandOutput::error(
                                    format!("sed: {}: {}\n", args[i], e),
                                    1,
                                )
                            }
                        }
                    }
                }
                arg if arg.starts_with("--file=") => {
                    let f = &arg["--file=".len()..];
                    match self.read_text_lossy(f) {
                        Ok(c) => expressions.extend(
                            c.lines()
                                .map(str::trim)
                                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                .map(|l| l.to_string()),
                        ),
                        Err(e) => return CommandOutput::error(format!("sed: {}: {}\n", f, e), 1),
                    }
                }
                arg if arg.starts_with("-f") && arg.len() > 2 => {
                    let f = &arg[2..];
                    match self.read_text_lossy(f) {
                        Ok(c) => expressions.extend(
                            c.lines()
                                .map(str::trim)
                                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                .map(|l| l.to_string()),
                        ),
                        Err(e) => return CommandOutput::error(format!("sed: {}: {}\n", f, e), 1),
                    }
                }
                "-i" => in_place = true,
                "-z" | "--null-data" => null_data = true,
                "-n" => quiet = true,
                // -E / -r: extended regex is the default engine; accept silently.
                "-E" | "-r" | "--regexp-extended" => {}
                "-ni" | "-in" => {
                    quiet = true;
                    in_place = true;
                }
                arg if arg.starts_with("-i") && arg.len() > 2 => {
                    // `-i.bak` style: in-place with a backup suffix.
                    in_place = true;
                    backup_suffix = Some(arg[2..].to_string());
                }
                arg if arg.starts_with("-e") && arg.len() > 2 => {
                    expressions.push(arg[2..].to_string());
                }
                arg if !arg.starts_with('-') && expressions.is_empty() => {
                    expressions.push(arg.to_string());
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => crate::warn!("sed: warning: unsupported option '{}'", args[i]),
            }
            i += 1;
        }

        if expressions.is_empty() {
            return CommandOutput::error("sed: missing expression\n".to_string(), 1);
        }

        let sep = if null_data { '\0' } else { '\n' };

        // Control-flow scripts (n/N/D/P/{}/b/t/:) go through the full
        // interpreter; everything else keeps the fast linear path.
        let program = expressions.join("\n");
        let is_script = split_sed_statements(&program)
            .iter()
            .any(|s| sed_stmt_is_control(s));
        let parsed: Vec<SedCommand> = if is_script {
            parse_sed_program(&program)
        } else {
            let mut p: Vec<SedCommand> = Vec::new();
            for expr in &expressions {
                p.extend(parse_sed_command(expr));
            }
            p
        };
        // `r FILE` needs VFS access: resolve it now into an `Append` of the file
        // contents (multi-line preserved), so the pure apply functions stay
        // I/O-free.
        let parsed: Vec<SedCommand> = parsed
            .into_iter()
            .map(|c| match c {
                SedCommand::ReadFile(addr, path) => {
                    let content = self.read_text_lossy(&path).unwrap_or_default();
                    SedCommand::Append(addr, content.trim_end_matches('\n').to_string())
                }
                other => other,
            })
            .collect();
        let run = |content: &str| -> String {
            if is_script {
                apply_sed_script(content, &parsed, quiet, sep)
            } else {
                apply_sed_commands(content, &parsed, quiet, sep)
            }
        };

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    let processed = run(input);
                    return CommandOutput::success(processed);
                }
                None => return CommandOutput::error("sed: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        for file in &files {
            let content = match self.read_text_lossy(file) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("sed: {}: {}\n", file, e), 1),
            };

            let processed = run(&content);

            if in_place {
                if let Some(suf) = &backup_suffix {
                    let bak = format!("{file}{suf}");
                    if let Err(e) = self.vfs.write(&bak, &self.cwd, &content) {
                        return CommandOutput::error(format!("sed: {}: {}\n", bak, e), 1);
                    }
                }
                if let Err(e) = self.vfs.write(file, &self.cwd, &processed) {
                    return CommandOutput::error(format!("sed: {}: {}\n", file, e), 1);
                }
            } else {
                output.push_str(&processed);
            }
        }

        CommandOutput::success(output)
    }
}

/// A single line address: number, `$` (last line) or /pattern/.
#[derive(Debug, Clone)]
enum LineSpec {
    Num(usize),
    Last,
    Pat(String),
    /// `first~step` — e.g. `1~2` (odd lines), `0~3` (every 3rd).
    Step(usize, usize),
}

/// Optional address (single line or inclusive range) attached to a command.
#[derive(Debug, Clone)]
struct Addr {
    start: LineSpec,
    end: Option<LineSpec>,
}

impl Addr {
    /// True when the 1-based `line_no` matches this address.
    fn matches(&self, line_no: usize, total: usize, line: &str) -> bool {
        let point = |spec: &LineSpec| -> Option<usize> {
            match spec {
                LineSpec::Num(n) => Some(*n),
                LineSpec::Last => Some(total),
                LineSpec::Pat(_) | LineSpec::Step(_, _) => None,
            }
        };
        match (&self.start, &self.end) {
            (LineSpec::Step(first, step), None) => {
                if *step == 0 {
                    return false;
                }
                let first = if *first == 0 { *step } else { *first };
                line_no >= first && (line_no - first) % step == 0
            }
            (LineSpec::Pat(p), None) => match regex::Regex::new(p) {
                Ok(re) => re.is_match(line),
                Err(_) => line.contains(p.as_str()),
            },
            (s, None) => point(s) == Some(line_no),
            (s, Some(e)) => {
                let lo = point(s).unwrap_or(1);
                let hi = point(e).unwrap_or(total);
                line_no >= lo && line_no <= hi
            }
        }
    }

    /// Stateful address match supporting ranges whose start/end are regexes
    /// (`/a/,/b/`). `active` tracks whether the range is currently open.
    fn hit(&self, line_no: usize, total: usize, line: &str, active: &mut bool) -> bool {
        let pat_match = |p: &str| {
            regex::Regex::new(p)
                .map(|re| re.is_match(line))
                .unwrap_or_else(|_| line.contains(p))
        };
        let start_match = match &self.start {
            LineSpec::Num(n) => line_no == *n,
            LineSpec::Last => line_no == total,
            LineSpec::Pat(p) => pat_match(p),
            LineSpec::Step(first, step) => {
                if *step == 0 {
                    false
                } else {
                    let f = if *first == 0 { *step } else { *first };
                    line_no >= f && (line_no - f) % step == 0
                }
            }
        };
        let end_match = |e: &LineSpec| match e {
            LineSpec::Num(n) => line_no == *n,
            LineSpec::Last => line_no == total,
            LineSpec::Pat(p) => pat_match(p),
            LineSpec::Step(_, _) => false,
        };
        if let Some(end) = &self.end {
            if *active {
                let ends = match end {
                    LineSpec::Num(n) => line_no >= *n,
                    LineSpec::Last => line_no >= total,
                    LineSpec::Pat(_) => end_match(end),
                    LineSpec::Step(_, _) => false,
                };
                if ends {
                    *active = false;
                }
                return true;
            }
            if start_match {
                *active = true;
                let ends_same = match end {
                    LineSpec::Num(n) => line_no >= *n,
                    LineSpec::Last => line_no >= total,
                    LineSpec::Pat(_) => end_match(end),
                    LineSpec::Step(_, _) => false,
                };
                if ends_same {
                    *active = false;
                }
                return true;
            }
            return false;
        }
        start_match
    }
}

enum SedCommand {
    Substitute {
        addr: Option<Addr>,
        pattern: String,
        replacement: String,
        global: bool,
        print: bool,
        regex: Option<String>,
    },
    Delete(Option<Addr>),
    Print(Option<Addr>),
    /// `q` — print the pattern space (unless `-n`) and quit.
    Quit(Option<Addr>),
    /// `a TEXT` — append TEXT after the matching line.
    Append(Option<Addr>, String),
    /// `i TEXT` — insert TEXT before the matching line.
    Insert(Option<Addr>, String),
    /// `c TEXT` — replace the matching line with TEXT.
    Change(Option<Addr>, String),
    /// `r FILE` — append the contents of FILE after the matching line.
    ReadFile(Option<Addr>, String),
    /// `y/SRC/DST/` — transliterate characters.
    Transliterate(Option<Addr>, Vec<char>, Vec<char>),
    /// `n` — auto-print the pattern space, then read the next input line.
    Next(Option<Addr>),
    /// `N` — append the next input line to the pattern space (with `\n`).
    AppendNext(Option<Addr>),
    /// `D` — delete up to the first `\n`; restart the cycle without reading.
    DeleteFirst(Option<Addr>),
    /// `P` — print the pattern space up to the first `\n`.
    PrintFirst(Option<Addr>),
    /// `h` / `H` / `g` / `G` / `x` — hold-space operations.
    Hold(Option<Addr>),
    HoldAppend(Option<Addr>),
    Get(Option<Addr>),
    GetAppend(Option<Addr>),
    Exchange(Option<Addr>),
    /// `b LABEL` — branch to a label (or to the end of the script).
    Branch(usize),
    /// `t LABEL` — branch if a substitution has been made since the last input.
    BranchIfSubst(usize),
    /// `:LABEL` — a label (no-op placeholder that keeps PC indices stable).
    Label,
    /// `ADDR {` — start of a block; `end` is the index of the matching `}`.
    BlockStart {
        addr: Option<Addr>,
        end: usize,
    },
    /// `}` — end of a block.
    BlockEnd,
}

/// Parses an optional numeric/`$`//pat/ address prefix from `part`.
/// Returns (address, rest_after_address).
fn parse_addr(part: &str) -> (Option<Addr>, &str) {
    let parse_spec = |s: &str| -> Option<(LineSpec, usize)> {
        if s.starts_with('$') {
            return Some((LineSpec::Last, 1));
        }
        if s.starts_with('/') {
            if let Some(close) = s[1..].find('/') {
                return Some((LineSpec::Pat(s[1..1 + close].to_string()), close + 2));
            }
            return None;
        }
        let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let n: usize = digits.parse().ok()?;
        if let Some(rest) = s[digits.len()..].strip_prefix('~') {
            let step_digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if step_digits.is_empty() {
                return None;
            }
            let step: usize = step_digits.parse().ok()?;
            return Some((
                LineSpec::Step(n, step),
                digits.len() + 1 + step_digits.len(),
            ));
        }
        Some((LineSpec::Num(n), digits.len()))
    };

    let (start, used) = match parse_spec(part) {
        Some(x) => x,
        None => return (None, part),
    };
    let rest = &part[used..];
    if let Some(rest2) = rest.strip_prefix(',') {
        if let Some((end, used2)) = parse_spec(rest2) {
            return (
                Some(Addr {
                    start,
                    end: Some(end),
                }),
                &rest2[used2..],
            );
        }
    }
    (Some(Addr { start, end: None }), rest)
}

fn parse_sed_command(expr: &str) -> Vec<SedCommand> {
    let mut commands = Vec::new();
    for part in expr.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let (addr, rest) = parse_addr(part);
        let rest = rest.trim();

        match rest.chars().next() {
            Some('p') if rest == "p" => {
                commands.push(SedCommand::Print(addr));
                continue;
            }
            Some('d') if rest == "d" => {
                commands.push(SedCommand::Delete(addr));
                continue;
            }
            Some('q') if rest == "q" => {
                commands.push(SedCommand::Quit(addr));
                continue;
            }
            // `a TEXT` / `i TEXT` / `c TEXT` (also the `a\` newline form, where
            // the text follows on later lines — here we take the remainder of
            // this expression).
            Some(ch @ ('a' | 'i' | 'c'))
                if rest.len() == 1
                    || rest.as_bytes()[1].is_ascii_whitespace()
                    || rest.as_bytes()[1] == b'\\' =>
            {
                let text = rest[1..].trim_start_matches(['\\', ' ', '\t']).to_string();
                commands.push(match ch {
                    'a' => SedCommand::Append(addr, text),
                    'i' => SedCommand::Insert(addr, text),
                    _ => SedCommand::Change(addr, text),
                });
                continue;
            }
            // `r FILE` — read and append a file (the path follows `r`).
            Some('r') if rest.len() > 1 => {
                let file = rest[1..].trim().to_string();
                if !file.is_empty() {
                    commands.push(SedCommand::ReadFile(addr, file));
                }
                continue;
            }
            // Transliterate: `y/SRC/DST/`.
            Some('y') => {
                let stripped = &rest[1..];
                if let Some(delim) = stripped.chars().next() {
                    let body = &stripped[delim.len_utf8()..];
                    let parts: Vec<&str> = body.splitn(3, delim).collect();
                    if parts.len() >= 2 {
                        commands.push(SedCommand::Transliterate(
                            addr,
                            parts[0].chars().collect(),
                            parts[1].chars().collect(),
                        ));
                    }
                }
                continue;
            }
            _ => {}
        }

        // Substitute: s/pat/rep/[flags]
        if let Some(stripped) = rest.strip_prefix('s') {
            let delim = match stripped.chars().next() {
                Some(d) => d,
                None => continue,
            };
            let body = &stripped[delim.len_utf8()..];
            let parts: Vec<&str> = body.splitn(3, delim).collect();
            if parts.len() >= 2 {
                let pattern = parts[0].to_string();
                let replacement = parts[1].to_string();
                let flags = parts.get(2).unwrap_or(&"");
                let global = flags.contains('g');
                let print = flags.contains('p');

                let regex = if pattern.contains('*')
                    || pattern.contains('.')
                    || pattern.contains('[')
                    || pattern.contains('(')
                    || pattern.contains('\\')
                    || pattern.contains('^')
                    || pattern.contains('$')
                    || pattern.contains('+')
                    || pattern.contains('?')
                    || pattern.contains('{')
                    || pattern.contains('|')
                {
                    Some(pattern.clone())
                } else {
                    None
                };

                commands.push(SedCommand::Substitute {
                    addr,
                    pattern,
                    replacement,
                    global,
                    print,
                    regex,
                });
            }
        }
    }
    commands
}

fn apply_sed_commands(content: &str, commands: &[SedCommand], quiet: bool, sep: char) -> String {
    let lines: Vec<&str> = if sep == '\0' {
        let mut v: Vec<&str> = content.split('\0').collect();
        if content.ends_with('\0') {
            v.pop();
        }
        v
    } else {
        content.lines().collect()
    };
    let total = lines.len();
    let mut output = String::new();
    let mut quit = false;

    for (idx, &line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let mut keep = true;
        let mut modified = line.to_string();
        let mut extra_prints = 0usize;
        let mut insert_texts: Vec<String> = Vec::new();
        let mut append_texts: Vec<String> = Vec::new();
        let mut changed: Option<String> = None;

        for cmd in commands {
            match cmd {
                SedCommand::Insert(addr, text) => {
                    if addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true)
                    {
                        insert_texts.push(text.clone());
                    }
                }
                SedCommand::Append(addr, text) => {
                    if addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true)
                    {
                        append_texts.push(text.clone());
                    }
                }
                SedCommand::Change(addr, text) => {
                    if addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true)
                    {
                        changed = Some(text.clone());
                    }
                }
                SedCommand::Transliterate(addr, from, to) => {
                    let hit = addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true);
                    if hit {
                        modified = modified
                            .chars()
                            .map(|c| {
                                from.iter()
                                    .position(|&x| x == c)
                                    .and_then(|i| to.get(i).copied())
                                    .unwrap_or(c)
                            })
                            .collect();
                    }
                }
                SedCommand::Delete(addr) => {
                    let hit = addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true);
                    if hit {
                        keep = false;
                        break;
                    }
                }
                SedCommand::Quit(addr) => {
                    let hit = addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true);
                    if hit {
                        quit = true;
                        break;
                    }
                }
                SedCommand::Print(addr) => {
                    let hit = addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true);
                    if hit {
                        extra_prints += 1;
                    }
                }
                SedCommand::Substitute {
                    addr,
                    ref pattern,
                    ref replacement,
                    global,
                    print,
                    ref regex,
                } => {
                    let hit = addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true);
                    if !hit {
                        continue;
                    }
                    let prev = modified.clone();
                    let matched = if let Some(re_str) = regex {
                        regex::Regex::new(re_str)
                            .map(|re| re.is_match(&prev))
                            .unwrap_or_else(|_| prev.contains(pattern.as_str()))
                    } else {
                        prev.contains(pattern.as_str())
                    };
                    if let Some(re) = regex {
                        match regex::Regex::new(re) {
                            Ok(re) => {
                                let rep = sed_replacement_to_regex(replacement);
                                if *global {
                                    modified = re.replace_all(&modified, rep.as_str()).to_string();
                                } else {
                                    modified = re.replace(&modified, rep.as_str()).to_string();
                                }
                            }
                            Err(_) => {
                                if *global {
                                    modified = prev.replace(pattern.as_str(), replacement.as_str());
                                } else {
                                    modified =
                                        prev.replacen(pattern.as_str(), replacement.as_str(), 1);
                                }
                            }
                        }
                    } else {
                        if *global {
                            modified = prev.replace(pattern.as_str(), replacement.as_str());
                        } else {
                            modified = prev.replacen(pattern.as_str(), replacement.as_str(), 1);
                        }
                    }
                    if *print && matched {
                        extra_prints += 1;
                    }
                }
                // The fast linear path never receives the control-flow variants
                // (choosing it is gated on `sed_stmt_is_control`).
                _ => {}
            }
        }

        // `i`/`a`/`c` text is emitted regardless of `-n` (GNU sed).
        for t in &insert_texts {
            output.push_str(t);
            output.push(sep);
        }
        if let Some(t) = &changed {
            output.push_str(t);
            output.push(sep);
        } else if keep {
            if !quiet {
                output.push_str(&modified);
                output.push(sep);
            }
            // `p` output (once per matching p command); with `-n` this is the
            // only thing printed for the line.
            for _ in 0..extra_prints {
                output.push_str(&modified);
                output.push(sep);
            }
        }
        for t in &append_texts {
            output.push_str(t);
            output.push(sep);
        }
        if quit {
            break;
        }
    }

    output
}

/// Translate a sed `s///` replacement into the `regex` crate's replacement
/// syntax: `\1`..`\9` → `${1}`..`${9}`, `\&` → `${0}` (whole match), plus sed
/// escapes `\n`/`\t`/`\\`. A literal `$` is escaped as `$$`.
fn sed_replacement_to_regex(rep: &str) -> String {
    let mut out = String::with_capacity(rep.len());
    let mut chars = rep.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek().copied() {
                Some(d) if d.is_ascii_digit() => {
                    out.push_str(&format!("${{{d}}}"));
                    chars.next();
                }
                Some('&') => {
                    out.push_str("${0}");
                    chars.next();
                }
                Some('n') => {
                    out.push('\n');
                    chars.next();
                }
                Some('t') => {
                    out.push('\t');
                    chars.next();
                }
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                    chars.next();
                }
                None => out.push('\\'),
            },
            '$' => out.push_str("$$"),
            _ => out.push(c),
        }
    }
    out
}

// ── Full sed script interpreter (n/N/D/P/{}/b/t) ──────────────────────────
// Used only when the script contains control-flow constructs; simple scripts
// keep the fast linear path (`apply_sed_commands`) unchanged.

fn sed_skip_one_addr(c: &[char], mut i: usize) -> usize {
    let n = c.len();
    while i < n && (c[i] == ' ' || c[i] == '\t') {
        i += 1;
    }
    if i < n && c[i] == '$' {
        return i + 1;
    }
    if i < n && c[i] == '/' {
        i += 1;
        while i < n {
            if c[i] == '\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if c[i] == '/' {
                return i + 1;
            }
            i += 1;
        }
        return i;
    }
    let s = i;
    while i < n && c[i].is_ascii_digit() {
        i += 1;
    }
    if i > s {
        if i < n && c[i] == '~' {
            i += 1;
            while i < n && c[i].is_ascii_digit() {
                i += 1;
            }
        }
        return i;
    }
    i
}

fn sed_skip_address(c: &[char], mut i: usize) -> usize {
    let before = i;
    i = sed_skip_one_addr(c, i);
    if i > before && i < c.len() && c[i] == ',' {
        i += 1;
        let before2 = i;
        let j = sed_skip_one_addr(c, i);
        if j > before2 {
            i = j;
        }
    }
    i
}

/// Split a sed program into statements, honoring `s`/`y` delimiters, `{}`,
/// `a`/`i`/`c` text, and `;`/newline separators.
fn split_sed_statements(expr: &str) -> Vec<String> {
    let c: Vec<char> = expr.chars().collect();
    let n = c.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        while i < n && (c[i] == ';' || c[i] == '\n' || c[i] == ' ' || c[i] == '\t') {
            i += 1;
        }
        if i >= n {
            break;
        }
        if c[i] == '{' {
            out.push("{".to_string());
            i += 1;
            continue;
        }
        if c[i] == '}' {
            out.push("}".to_string());
            i += 1;
            continue;
        }
        let start = i;
        i = sed_skip_address(&c, i);
        while i < n && (c[i] == ' ' || c[i] == '\t') {
            i += 1;
        }
        if i >= n {
            out.push(c[start..].iter().collect());
            break;
        }
        match c[i] {
            '{' => {
                i += 1;
                out.push(c[start..i].iter().collect());
            }
            's' | 'y' => {
                i += 1;
                if i < n {
                    let d = c[i];
                    i += 1;
                    for _ in 0..2 {
                        while i < n {
                            if c[i] == '\\' && i + 1 < n {
                                i += 2;
                                continue;
                            }
                            if c[i] == d {
                                i += 1;
                                break;
                            }
                            i += 1;
                        }
                    }
                    while i < n && !matches!(c[i], ';' | '}' | '\n' | '{') {
                        i += 1;
                    }
                }
                out.push(c[start..i].iter().collect());
            }
            'a' | 'i' | 'c' => {
                i += 1;
                if i < n && c[i] == '\\' {
                    i += 1;
                }
                while i < n && !matches!(c[i], ';' | '}' | '\n') {
                    i += 1;
                }
                out.push(c[start..i].iter().collect());
            }
            _ => {
                i += 1;
                while i < n && !matches!(c[i], ';' | '}' | '\n' | '{') {
                    i += 1;
                }
                out.push(c[start..i].iter().collect());
            }
        }
    }
    out
}

fn sed_stmt_is_control(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    if t == "{" || t == "}" {
        return true;
    }
    let c: Vec<char> = t.chars().collect();
    let i = sed_skip_address(&c, 0);
    let addr_part: String = c[..i].iter().collect();
    if addr_part.contains(',') && addr_part.contains('/') {
        return true; // regex range → needs the stateful interpreter
    }
    let mut j = i;
    while j < c.len() && (c[j] == ' ' || c[j] == '\t') {
        j += 1;
    }
    if j >= c.len() {
        return false;
    }
    matches!(
        c[j],
        'n' | 'N' | 'D' | 'P' | 'h' | 'H' | 'g' | 'G' | 'x' | 'b' | 't' | ':'
    )
}

fn sed_push_command(
    cmds: &mut Vec<SedCommand>,
    full: &str,
    addr: Option<Addr>,
    rest: &str,
    labels: &mut std::collections::HashMap<String, usize>,
    fixups: &mut Vec<(usize, String)>,
) {
    let c = rest.chars().next().unwrap_or(' ');
    match c {
        ':' => {
            labels.insert(rest[1..].trim().to_string(), cmds.len());
            cmds.push(SedCommand::Label);
        }
        'b' => {
            let l = rest[1..].trim().to_string();
            cmds.push(SedCommand::Branch(0));
            fixups.push((cmds.len() - 1, l));
        }
        't' => {
            let l = rest[1..].trim().to_string();
            cmds.push(SedCommand::BranchIfSubst(0));
            fixups.push((cmds.len() - 1, l));
        }
        'n' if rest == "n" => cmds.push(SedCommand::Next(addr)),
        'N' if rest == "N" => cmds.push(SedCommand::AppendNext(addr)),
        'D' if rest == "D" => cmds.push(SedCommand::DeleteFirst(addr)),
        'P' if rest == "P" => cmds.push(SedCommand::PrintFirst(addr)),
        'h' if rest == "h" => cmds.push(SedCommand::Hold(addr)),
        'H' if rest == "H" => cmds.push(SedCommand::HoldAppend(addr)),
        'g' if rest == "g" => cmds.push(SedCommand::Get(addr)),
        'G' if rest == "G" => cmds.push(SedCommand::GetAppend(addr)),
        'x' if rest == "x" => cmds.push(SedCommand::Exchange(addr)),
        _ => cmds.extend(parse_sed_command(full)),
    }
}

fn parse_sed_program(expr: &str) -> Vec<SedCommand> {
    let stmts = split_sed_statements(expr);
    let mut cmds: Vec<SedCommand> = Vec::new();
    let mut block_stack: Vec<usize> = Vec::new();
    let mut labels: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut fixups: Vec<(usize, String)> = Vec::new();
    for st in &stmts {
        let st = st.trim();
        if st.is_empty() {
            continue;
        }
        if st == "{" {
            let idx = cmds.len();
            cmds.push(SedCommand::BlockStart { addr: None, end: 0 });
            block_stack.push(idx);
            continue;
        }
        if st == "}" {
            if let Some(start) = block_stack.pop() {
                let end_idx = cmds.len();
                if let SedCommand::BlockStart { end, .. } = &mut cmds[start] {
                    *end = end_idx;
                }
                cmds.push(SedCommand::BlockEnd);
            }
            continue;
        }
        let (addr, rest) = parse_addr(st);
        let rest = rest.trim();
        if let Some(base) = rest.strip_suffix('{') {
            let idx = cmds.len();
            cmds.push(SedCommand::BlockStart {
                addr: addr.clone(),
                end: 0,
            });
            block_stack.push(idx);
            let base = base.trim();
            if !base.is_empty() {
                sed_push_command(
                    &mut cmds,
                    base,
                    addr.clone(),
                    base,
                    &mut labels,
                    &mut fixups,
                );
            }
            continue;
        }
        if rest.is_empty() {
            continue;
        }
        sed_push_command(&mut cmds, st, addr, rest, &mut labels, &mut fixups);
    }
    while let Some(start) = block_stack.pop() {
        let end_idx = cmds.len();
        if let SedCommand::BlockStart { end, .. } = &mut cmds[start] {
            *end = end_idx;
        }
        cmds.push(SedCommand::BlockEnd);
    }
    for (ci, label) in fixups {
        let target = if label.is_empty() {
            cmds.len()
        } else {
            labels.get(&label).copied().unwrap_or(cmds.len())
        };
        match &mut cmds[ci] {
            SedCommand::Branch(t) | SedCommand::BranchIfSubst(t) => *t = target,
            _ => {}
        }
    }
    cmds
}

fn sed_cmd_addr(cmd: &SedCommand) -> Option<&Addr> {
    match cmd {
        SedCommand::Substitute { addr, .. }
        | SedCommand::Delete(addr)
        | SedCommand::Print(addr)
        | SedCommand::Quit(addr)
        | SedCommand::Append(addr, _)
        | SedCommand::Insert(addr, _)
        | SedCommand::Change(addr, _)
        | SedCommand::Transliterate(addr, _, _)
        | SedCommand::Next(addr)
        | SedCommand::AppendNext(addr)
        | SedCommand::DeleteFirst(addr)
        | SedCommand::PrintFirst(addr)
        | SedCommand::Hold(addr)
        | SedCommand::HoldAppend(addr)
        | SedCommand::Get(addr)
        | SedCommand::GetAppend(addr)
        | SedCommand::Exchange(addr) => addr.as_ref(),
        SedCommand::BlockStart { addr, .. } => addr.as_ref(),
        _ => None,
    }
}

fn apply_sed_script(content: &str, commands: &[SedCommand], quiet: bool, sep: char) -> String {
    let input: Vec<String> = if sep == '\0' {
        let mut v: Vec<String> = content.split('\0').map(|s| s.to_string()).collect();
        if content.ends_with('\0') {
            v.pop();
        }
        v
    } else {
        content.lines().map(|s| s.to_string()).collect()
    };
    let total = input.len();
    let mut output = String::new();
    let mut hold = String::new();
    let mut range_active: Vec<bool> = vec![false; commands.len()];
    let mut i = 0usize;
    'outer: while i < total {
        let mut pattern = input[i].clone();
        let mut line_no = i + 1;
        let mut subst_made = false;
        let mut changed: Option<String> = None;
        let mut extra_prints = 0usize;
        let mut insert_texts: Vec<String> = Vec::new();
        let mut append_texts: Vec<String> = Vec::new();
        let mut quit = false;
        let mut no_final_print = false;
        let mut pc = 0usize;
        while pc < commands.len() {
            let hit = match sed_cmd_addr(&commands[pc]) {
                Some(a) => a.hit(line_no, total, &pattern, &mut range_active[pc]),
                None => true,
            };
            match &commands[pc] {
                SedCommand::BlockStart { end, .. } => {
                    if hit {
                        pc += 1;
                    } else {
                        pc = end + 1;
                    }
                }
                SedCommand::BlockEnd | SedCommand::Label => pc += 1,
                SedCommand::Branch(t) => pc = *t,
                SedCommand::BranchIfSubst(t) => {
                    if subst_made {
                        subst_made = false;
                        pc = *t;
                    } else {
                        pc += 1;
                    }
                }
                SedCommand::Next(_) => {
                    if hit {
                        if !quiet {
                            output.push_str(&pattern);
                            output.push(sep);
                        }
                        i += 1;
                        if i >= total {
                            quit = true;
                            no_final_print = true;
                            break;
                        }
                        pattern = input[i].clone();
                        line_no = i + 1;
                        subst_made = false;
                    }
                    pc += 1;
                }
                SedCommand::AppendNext(_) => {
                    if hit && i + 1 < total {
                        i += 1;
                        pattern.push('\n');
                        pattern.push_str(&input[i]);
                    }
                    pc += 1;
                }
                SedCommand::DeleteFirst(_) => {
                    if hit {
                        if let Some(nl) = pattern.find('\n') {
                            pattern = pattern[nl + 1..].to_string();
                            subst_made = false;
                            pc = 0;
                        } else {
                            changed = None;
                            insert_texts.clear();
                            append_texts.clear();
                            i += 1;
                            continue 'outer;
                        }
                    } else {
                        pc += 1;
                    }
                }
                SedCommand::PrintFirst(_) => {
                    if hit {
                        output.push_str(pattern.split('\n').next().unwrap_or(""));
                        output.push(sep);
                    }
                    pc += 1;
                }
                SedCommand::Hold(_) => {
                    if hit {
                        hold = pattern.clone();
                    }
                    pc += 1;
                }
                SedCommand::HoldAppend(_) => {
                    if hit {
                        hold.push('\n');
                        hold.push_str(&pattern);
                    }
                    pc += 1;
                }
                SedCommand::Get(_) => {
                    if hit {
                        pattern = hold.clone();
                    }
                    pc += 1;
                }
                SedCommand::GetAppend(_) => {
                    if hit {
                        pattern.push('\n');
                        pattern.push_str(&hold);
                    }
                    pc += 1;
                }
                SedCommand::Exchange(_) => {
                    if hit {
                        std::mem::swap(&mut pattern, &mut hold);
                    }
                    pc += 1;
                }
                SedCommand::Delete(_) => {
                    if hit {
                        i += 1;
                        continue 'outer;
                    }
                    pc += 1;
                }
                SedCommand::Quit(_) => {
                    if hit {
                        quit = true;
                        break;
                    }
                    pc += 1;
                }
                SedCommand::Print(_) => {
                    if hit {
                        extra_prints += 1;
                    }
                    pc += 1;
                }
                SedCommand::Insert(_, t) => {
                    if hit {
                        insert_texts.push(t.clone());
                    }
                    pc += 1;
                }
                SedCommand::Append(_, t) => {
                    if hit {
                        append_texts.push(t.clone());
                    }
                    pc += 1;
                }
                // Resolved into `Append` before the script runs (needs VFS).
                SedCommand::ReadFile(_, _) => {
                    pc += 1;
                }
                SedCommand::Change(_, t) => {
                    if hit {
                        changed = Some(t.clone());
                        pc += 1;
                    } else {
                        pc += 1;
                    }
                }
                SedCommand::Transliterate(_, from, to) => {
                    if hit {
                        pattern = pattern
                            .chars()
                            .map(|c| {
                                from.iter()
                                    .position(|&x| x == c)
                                    .and_then(|k| to.get(k).copied())
                                    .unwrap_or(c)
                            })
                            .collect();
                    }
                    pc += 1;
                }
                SedCommand::Substitute {
                    pattern: ref pat,
                    ref replacement,
                    global,
                    print,
                    ref regex,
                    ..
                } => {
                    if hit {
                        let prev = pattern.clone();
                        let matched = if let Some(re_str) = regex {
                            regex::Regex::new(re_str)
                                .map(|re| re.is_match(&prev))
                                .unwrap_or_else(|_| prev.contains(pat.as_str()))
                        } else {
                            prev.contains(pat.as_str())
                        };
                        if let Some(re) = regex {
                            match regex::Regex::new(re) {
                                Ok(re) => {
                                    let rep = sed_replacement_to_regex(replacement);
                                    if *global {
                                        pattern =
                                            re.replace_all(&pattern, rep.as_str()).to_string();
                                    } else {
                                        pattern = re.replace(&pattern, rep.as_str()).to_string();
                                    }
                                }
                                Err(_) => {
                                    if *global {
                                        pattern = prev.replace(pat.as_str(), replacement.as_str());
                                    } else {
                                        pattern =
                                            prev.replacen(pat.as_str(), replacement.as_str(), 1);
                                    }
                                }
                            }
                        } else if *global {
                            pattern = prev.replace(pat.as_str(), replacement.as_str());
                        } else {
                            pattern = prev.replacen(pat.as_str(), replacement.as_str(), 1);
                        }
                        if matched {
                            subst_made = true;
                        }
                        if *print && matched {
                            extra_prints += 1;
                        }
                    }
                    pc += 1;
                }
            }
        }
        if !quit {
            for t in &insert_texts {
                output.push_str(t);
                output.push(sep);
            }
            if let Some(t) = &changed {
                output.push_str(t);
                output.push(sep);
            } else {
                if !quiet {
                    output.push_str(&pattern);
                    output.push(sep);
                }
                for _ in 0..extra_prints {
                    output.push_str(&pattern);
                    output.push(sep);
                }
            }
            for t in &append_texts {
                output.push_str(t);
                output.push(sep);
            }
        } else {
            // `q`: auto-print the pattern space (unless -n) then stop.
            for t in &insert_texts {
                output.push_str(t);
                output.push(sep);
            }
            if let Some(t) = &changed {
                output.push_str(t);
                output.push(sep);
            } else {
                if !quiet && !no_final_print {
                    output.push_str(&pattern);
                    output.push(sep);
                }
                for _ in 0..extra_prints {
                    output.push_str(&pattern);
                    output.push(sep);
                }
            }
            break;
        }
        i += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(content: &str, expr: &str, quiet: bool) -> String {
        apply_sed_commands(content, &parse_sed_command(expr), quiet, '\n')
    }

    #[test]
    fn print_range_with_n() {
        let c = "l1\nl2\nl3\nl4\n";
        assert_eq!(run(c, "1,2p", true), "l1\nl2\n");
        assert_eq!(run(c, "2p", true), "l2\n");
        assert_eq!(run(c, "3,$p", true), "l3\nl4\n");
        assert_eq!(run(c, "$p", true), "l4\n");
    }

    #[test]
    fn print_pattern_with_n() {
        let c = "apple\nbanana\ncherry\n";
        assert_eq!(run(c, "/an/p", true), "banana\n");
    }

    #[test]
    fn delete_with_address() {
        let c = "l1\nl2\nl3\n";
        assert_eq!(run(c, "2d", false), "l1\nl3\n");
        assert_eq!(run(c, "1,2d", false), "l3\n");
        assert_eq!(run(c, "/l2/d", false), "l1\nl3\n");
    }

    #[test]
    fn substitute_plain_and_global() {
        assert_eq!(run("aaa\n", "s/a/b/", false), "baa\n");
        assert_eq!(run("aaa\n", "s/a/b/g", false), "bbb\n");
    }

    #[test]
    fn substitute_with_address() {
        let c = "x\nx\nx\n";
        assert_eq!(run(c, "2s/x/y/", false), "x\ny\nx\n");
        assert_eq!(run(c, "2,3s/x/y/", false), "x\ny\ny\n");
    }

    #[test]
    fn without_n_p_duplicates() {
        assert_eq!(run("a\nb\n", "1p", false), "a\na\nb\n");
    }

    fn mk_shell() -> Shell {
        use crate::vfs::Vfs;
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
    fn test_sed_help() {
        let mut shell = mk_shell();
        let out = shell.execute("sed", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_sed_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("sed", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_sed_extended_regexp_no_warning() {
        let mut shell = mk_shell();
        let out = shell.execute("sed", &["-E", "s/foo+/bar/"], Some("fooobar\n"));
        assert_eq!(out.exit_code, 0);
        assert!(
            out.stdout.contains("bar"),
            "sed -E should work: {}",
            out.stdout
        );
        assert!(
            !out.stderr.contains("unsupported"),
            "sed -E should not warn: {}",
            out.stderr
        );
    }
}
