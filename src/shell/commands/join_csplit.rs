// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! `join` and `csplit` — common text/field utilities.

use crate::shell::{CommandOutput, Shell};

fn split_fields(line: &str, sep: Option<&str>) -> Vec<String> {
    match sep {
        Some(s) if !s.is_empty() => line.split(s).map(|x| x.to_string()).collect(),
        _ => line.split_whitespace().map(|x| x.to_string()).collect(),
    }
}

impl Shell {
    /// `join [-t SEP] [-1 N] [-2 N] FILE1 FILE2` — join lines on a common field.
    /// (Hash join; order follows FILE1.)
    pub fn cmd_join(&self, args: &[&str], _stdin: Option<&str>) -> CommandOutput {
        let mut sep: Option<String> = None;
        let mut f1 = 1usize;
        let mut f2 = 1usize;
        let mut out_spec: Option<Vec<String>> = None;
        let mut files: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            match a {
                "-t" => {
                    if i + 1 < args.len() {
                        i += 1;
                        sep = Some(args[i].to_string());
                    }
                }
                "-1" => {
                    if i + 1 < args.len() {
                        i += 1;
                        f1 = args[i].parse().unwrap_or(1);
                    }
                }
                "-2" => {
                    if i + 1 < args.len() {
                        i += 1;
                        f2 = args[i].parse().unwrap_or(1);
                    }
                }
                "-o" => {
                    if i + 1 < args.len() {
                        i += 1;
                        out_spec = Some(args[i].split(',').map(|s| s.trim().to_string()).collect());
                    }
                }
                arg if arg.starts_with("-o") && arg.len() > 2 => {
                    out_spec = Some(arg[2..].split(',').map(|s| s.trim().to_string()).collect())
                }
                arg if arg.starts_with("-t") && arg.len() > 2 => sep = Some(arg[2..].to_string()),
                arg if arg.starts_with("-1") && arg.len() > 2 => f1 = arg[2..].parse().unwrap_or(1),
                arg if arg.starts_with("-2") && arg.len() > 2 => f2 = arg[2..].parse().unwrap_or(1),
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }
        if files.len() < 2 {
            return CommandOutput::error("join: missing operand\n".to_string(), 1);
        }
        let read = |f: &str| -> Result<Vec<Vec<String>>, String> {
            let s = self.read_text_lossy(f)?;
            Ok(s.lines().map(|l| split_fields(l, sep.as_deref())).collect())
        };
        let a = match read(&files[0]) {
            Ok(v) => v,
            Err(e) => return CommandOutput::error(format!("join: {}: {}\n", files[0], e), 1),
        };
        let b = match read(&files[1]) {
            Ok(v) => v,
            Err(e) => return CommandOutput::error(format!("join: {}: {}\n", files[1], e), 1),
        };
        let field = |row: &[String], n: usize| -> String {
            if n >= 1 && n <= row.len() {
                row[n - 1].clone()
            } else {
                String::new()
            }
        };
        let joiner = sep.clone().unwrap_or_else(|| " ".to_string());
        let mut out = String::new();
        for ra in &a {
            let key = field(ra, f1);
            for rb in &b {
                if field(rb, f2) == key {
                    let parts: Vec<String> = match &out_spec {
                        Some(spec) => spec
                            .iter()
                            .filter_map(|f| {
                                if f == "0" {
                                    Some(key.clone())
                                } else if let Some((file, num)) = f.split_once('.') {
                                    let n: usize = num.parse().unwrap_or(0);
                                    Some(if file == "1" {
                                        field(ra, n)
                                    } else {
                                        field(rb, n)
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect(),
                        None => {
                            let mut parts: Vec<String> = vec![key.clone()];
                            parts.extend(
                                ra.iter()
                                    .enumerate()
                                    .filter(|(i, _)| *i != f1 - 1)
                                    .map(|(_, s)| s.clone()),
                            );
                            parts.extend(
                                rb.iter()
                                    .enumerate()
                                    .filter(|(i, _)| *i != f2 - 1)
                                    .map(|(_, s)| s.clone()),
                            );
                            parts
                        }
                    };
                    out.push_str(&parts.join(&joiner));
                    out.push('\n');
                }
            }
        }
        CommandOutput::success(out)
    }

    /// `csplit [-s] [-f PREFIX] FILE PATTERN...` — split a file by line numbers
    /// (`N`) or `/regex/` (split before the first matching line). Supports a
    /// `{N}`/`{*}` repeat suffix on a `/regex/` pattern.
    pub fn cmd_csplit(&self, args: &[&str], _stdin: Option<&str>) -> CommandOutput {
        let mut quiet = false;
        let mut prefix = "xx".to_string();
        let mut positional: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            match a {
                "-s" | "--quiet" | "--silent" => quiet = true,
                "-f" | "--prefix" => {
                    if i + 1 < args.len() {
                        i += 1;
                        prefix = args[i].to_string();
                    }
                }
                arg if arg.starts_with("-f") && arg.len() > 2 => prefix = arg[2..].to_string(),
                arg if !arg.starts_with('-') => positional.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }
        if positional.is_empty() {
            return CommandOutput::error("csplit: missing operand\n".to_string(), 1);
        }
        let file = positional[0].clone();
        let patterns = &positional[1..];
        if patterns.is_empty() {
            return CommandOutput::error("csplit: missing pattern\n".to_string(), 1);
        }
        let content = match self.read_text_lossy(&file) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("csplit: {}: {}\n", file, e), 1),
        };
        let lines: Vec<&str> = content.lines().collect();

        // Build the cut points (line indices where a new piece starts).
        let mut cuts: Vec<usize> = vec![0];
        let mut cursor = 0usize;
        let mut pi = 0;
        while pi < patterns.len() {
            let pat = patterns[pi].as_str();
            let (base, repeat) = split_repeat(pat);
            if let Ok(n) = base.parse::<usize>() {
                // `N` = split before line N (1-based), absolute.
                let at = n.saturating_sub(1).min(lines.len());
                cuts.push(at);
                cursor = at;
            } else if let Some(re) = base.strip_prefix('/').and_then(|s| s.strip_suffix('/')) {
                // `/regex/` — split before the next line containing `re`.
                let mut count = 0usize;
                loop {
                    let start = if count == 0 { cursor } else { cursor + 1 };
                    match (start..lines.len()).find(|&k| lines[k].contains(re)) {
                        Some(k) => {
                            cuts.push(k);
                            cursor = k;
                            count += 1;
                            match repeat {
                                Repeat::Once => break,
                                Repeat::Times(n) if count >= n => break,
                                Repeat::Star => {
                                    if k + 1 >= lines.len() {
                                        break;
                                    }
                                }
                                Repeat::Times(_) => {}
                            }
                        }
                        None => break,
                    }
                }
            }
            pi += 1;
        }
        cuts.push(lines.len());
        cuts.dedup();
        if cuts.first() != Some(&0) {
            cuts.insert(0, 0);
        }

        let mut out = String::new();
        let mut total = 0usize;
        let mut idx = 0;
        for w in cuts.windows(2) {
            let (s, e) = (w[0], w[1]);
            if s >= e {
                continue;
            }
            let mut piece = String::new();
            for l in &lines[s..e] {
                piece.push_str(l);
                piece.push('\n');
            }
            let name = format!("{}{:02}", prefix, idx);
            idx += 1;
            total += piece.len();
            if let Err(err) = self.vfs.write(&name, &self.cwd, &piece) {
                return CommandOutput::error(format!("csplit: {}: {}\n", name, err), 1);
            }
            if !quiet {
                out.push_str(&format!("{}\n", total));
            }
        }
        CommandOutput::success(out)
    }
}

#[derive(Clone, Copy)]
enum Repeat {
    Once,
    Star,
    Times(usize),
}

fn split_repeat(pat: &str) -> (&str, Repeat) {
    if let Some(base) = pat.strip_suffix("{*}") {
        (base, Repeat::Star)
    } else if let Some(open) = pat.rfind('{') {
        if pat.ends_with('}') {
            if let Ok(n) = pat[open + 1..pat.len() - 1].parse::<usize>() {
                return (&pat[..open], Repeat::Times(n));
            }
        }
        (pat, Repeat::Once)
    } else {
        (pat, Repeat::Once)
    }
}
