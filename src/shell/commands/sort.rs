// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::cmp::Ordering;

const SORT_HELP_TEXT: &str = "\
sort: sort lines of text files
Usage: sort [OPTIONS] [FILE...]
Options:
  -n          Sort numerically
  -r          Reverse result
  -u          Output only unique lines
  -h          Compare human-readable numbers
  -f          Fold lower case to upper case
  -s          Stabilize sort (disable last-resort)
  -k SPEC     Sort via a key (e.g. 1,2)
  -t CHAR     Use CHAR as field separator
  --help      Show this help message
";

fn parse_human_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_part, suffix) = if let Some(_stripped) =
        s.strip_suffix(|c: char| c == 'K' || c == 'M' || c == 'G' || c == 'T')
    {
        let idx = s.len() - 1;
        (&s[..idx], &s[idx..])
    } else if let Some(stripped) = s.strip_suffix("B") {
        let stripped = stripped.trim();
        if let Some(_inner) =
            stripped.strip_suffix(|c: char| c == 'K' || c == 'M' || c == 'G' || c == 'T')
        {
            (
                &stripped[..stripped.len() - 1],
                &stripped[stripped.len() - 1..],
            )
        } else {
            (s, "")
        }
    } else {
        (s, "")
    };

    let base = num_part.parse::<f64>().ok()?;
    let multiplier = match suffix {
        "K" => 1024u64,
        "M" => 1024 * 1024,
        "G" => 1024 * 1024 * 1024,
        "T" => 1024 * 1024 * 1024 * 1024,
        _ => 1,
    };
    Some((base * multiplier as f64) as u64)
}

enum SortKeyValue {
    Num(f64),
    HumanNum(u64),
    Version(String),
    Month(u8),
    Str(String),
}

/// Parse a GNU `-k` spec `F[.C][OPTS][,F[.C][OPTS]]` into
/// `(start_field, end_field, modifiers)`. Only the field numbers and the
/// modifier letters are used; `.C` character positions are accepted but
/// ignored (we sort on whole fields).
fn parse_key_spec(spec: &str) -> (usize, Option<usize>, String) {
    let mut start = 0usize;
    let mut end: Option<usize> = None;
    let mut mods = String::new();
    for (idx, part) in spec.splitn(2, ',').enumerate() {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        let field = digits.parse::<usize>().unwrap_or(0);
        let after = &part[digits.len()..];
        // Drop an optional `.C` character position.
        let after = if let Some(rest) = after.strip_prefix('.') {
            let _c: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            &rest[_c.len()..]
        } else {
            after
        };
        mods.push_str(after);
        if idx == 0 {
            start = field;
        } else {
            end = Some(field);
        }
    }
    (start, end, mods)
}

/// Apply per-key modifier letters to the global sort flags (GNU semantics for
/// the common single-key case, which is what the pragmatism here supports).
fn apply_key_modifiers(
    mods: &str,
    numeric: &mut bool,
    reverse: &mut bool,
    human: &mut bool,
    version: &mut bool,
    month: &mut bool,
    fold_case: &mut bool,
) {
    for c in mods.chars() {
        match c {
            'n' | 'g' => *numeric = true,
            'r' => *reverse = true,
            'h' => *human = true,
            'V' => *version = true,
            'M' => *month = true,
            'f' => *fold_case = true,
            _ => {}
        }
    }
}

fn extract_sort_key(
    line: &str,
    key_start: usize,
    key_end: Option<usize>,
    delimiter: Option<char>,
    fold_case: bool,
    numeric: bool,
    human: bool,
    version: bool,
    month: bool,
    orig_line: &str,
) -> SortKeyValue {
    let fields: Vec<&str> = match delimiter {
        Some(d) => line.split(d).collect(),
        None => line.split_whitespace().collect(),
    };

    let start_idx = key_start.saturating_sub(1);
    if start_idx >= fields.len() {
        if numeric || human {
            // Non-numeric fields sort as 0/f64::NAN-like
            let s = if fold_case {
                orig_line.to_lowercase()
            } else {
                orig_line.to_string()
            };
            return SortKeyValue::Str(s);
        } else {
            let s = if fold_case {
                orig_line.to_lowercase()
            } else {
                orig_line.to_string()
            };
            return SortKeyValue::Str(s);
        }
    }

    let end_idx = key_end.unwrap_or(key_start).saturating_sub(1);
    let end_idx = end_idx.min(fields.len() - 1).max(start_idx);

    let sep_str: String;
    let sep: &str = match delimiter {
        Some(d) => {
            sep_str = d.to_string();
            &sep_str
        }
        None => " ",
    };
    let key_str: String = fields[start_idx..=end_idx].join(sep);
    let key_str = key_str.trim().to_string();
    let key_ref = if fold_case {
        key_str.to_lowercase()
    } else {
        key_str.clone()
    };

    if human {
        match parse_human_size(&key_ref) {
            Some(v) => SortKeyValue::HumanNum(v),
            None => SortKeyValue::Str(key_ref),
        }
    } else if numeric {
        match key_ref.parse::<f64>() {
            Ok(v) => SortKeyValue::Num(v),
            Err(_) => SortKeyValue::Str(key_ref),
        }
    } else if version {
        SortKeyValue::Version(key_ref)
    } else if month {
        match month_number(&key_ref) {
            Some(m) => SortKeyValue::Month(m),
            None => SortKeyValue::Str(key_ref),
        }
    } else {
        SortKeyValue::Str(key_ref)
    }
}

impl Shell {
    pub fn cmd_sort(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"--help") {
            return CommandOutput::success(SORT_HELP_TEXT.to_string());
        }
        let mut files = Vec::new();
        let mut numeric = false;
        let mut reverse = false;
        let mut unique = false;
        let mut human = false;
        let mut version = false;
        let mut month = false;
        let mut fold_case = false;
        let mut stable_flag = false;
        let mut random = false;
        let mut output_file: Option<String> = None;
        // `-k` may be repeated; keys are compared left-to-right.
        let mut keys: Vec<(usize, Option<usize>)> = Vec::new();
        let mut delimiter: Option<char> = None;

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg == "--" {
                i += 1;
                while i < args.len() {
                    files.push(args[i].to_string());
                    i += 1;
                }
                break;
            }
            if arg.starts_with("--") {
                match arg {
                    "--numeric-sort" => numeric = true,
                    "--reverse" => reverse = true,
                    "--unique" => unique = true,
                    "--human-numeric-sort" => human = true,
                    "--version-sort" => version = true,
                    "--month-sort" => month = true,
                    "--ignore-case" => fold_case = true,
                    "--stable" => stable_flag = true,
                    "--random-sort" => random = true,
                    "--general-numeric-sort" => numeric = true,
                    "--output" => {
                        if i + 1 < args.len() {
                            output_file = Some(args[i + 1].to_string());
                            i += 1;
                        }
                    }
                    a if a.starts_with("--output=") => output_file = Some(a[9..].to_string()),
                    a if a.starts_with("--key=") => {
                        let (s, e, mods) = parse_key_spec(&a[6..]);
                        apply_key_modifiers(
                            &mods,
                            &mut numeric,
                            &mut reverse,
                            &mut human,
                            &mut version,
                            &mut month,
                            &mut fold_case,
                        );
                        keys.push((s, e));
                    }
                    _ => crate::warn!("sort: warning: unsupported option '{}'", arg),
                }
                i += 1;
                continue;
            }
            if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg.chars().skip(1).collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'n' | 'g' => numeric = true,
                        'r' => reverse = true,
                        'u' => unique = true,
                        'h' => human = true,
                        'V' => version = true,
                        'M' => month = true,
                        'f' => fold_case = true,
                        's' => stable_flag = true,
                        'R' => random = true,
                        'k' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let spec = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            let (s, e, mods) = parse_key_spec(&spec);
                            apply_key_modifiers(
                                &mods,
                                &mut numeric,
                                &mut reverse,
                                &mut human,
                                &mut version,
                                &mut month,
                                &mut fold_case,
                            );
                            keys.push((s, e));
                            j = chars.len();
                            continue;
                        }
                        't' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let sep = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            delimiter = sep.chars().next();
                            j = chars.len();
                            continue;
                        }
                        'o' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let out = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            output_file = Some(out);
                            j = chars.len();
                            continue;
                        }
                        _ => crate::warn!("sort: warning: unsupported option '-{}'", chars[j]),
                    }
                    j += 1;
                }
                i += 1;
                continue;
            }
            files.push(arg.to_string());
            i += 1;
        }

        let mut all_lines = Vec::new();

        if files.is_empty() {
            match stdin {
                Some(s) => {
                    for line in s.lines() {
                        all_lines.push(line.to_string());
                    }
                }
                None => return CommandOutput::error("sort: missing file operand\n".to_string(), 1),
            }
        } else {
            for file in &files {
                match self.read_text_lossy(file) {
                    Ok(content) => {
                        for line in content.lines() {
                            all_lines.push(line.to_string());
                        }
                    }
                    Err(e) => return CommandOutput::error(format!("sort: {}: {}\n", file, e), 1),
                }
            }
        }

        if !keys.is_empty() {
            let mut indexed: Vec<(Vec<SortKeyValue>, String)> = all_lines
                .into_iter()
                .map(|line| {
                    let ks: Vec<SortKeyValue> = keys
                        .iter()
                        .map(|(s, e)| {
                            extract_sort_key(
                                &line, *s, *e, delimiter, fold_case, numeric, human, version,
                                month, &line,
                            )
                        })
                        .collect();
                    (ks, line)
                })
                .collect();

            indexed.sort_by(|a, b| {
                for (ka, kb) in a.0.iter().zip(b.0.iter()) {
                    let cmp = compare_keys(ka, kb);
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                }
                if !stable_flag {
                    compare_strings(&a.1, &b.1, fold_case)
                } else {
                    Ordering::Equal
                }
            });

            all_lines = indexed.into_iter().map(|(_, line)| line).collect();
        } else {
            if version {
                all_lines.sort_by(|a, b| {
                    let cmp = compare_versions(a, b);
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                    if !stable_flag {
                        a.cmp(b)
                    } else {
                        Ordering::Equal
                    }
                });
            } else if month {
                all_lines.sort_by(|a, b| {
                    let ma = month_number(a).unwrap_or(0);
                    let mb = month_number(b).unwrap_or(0);
                    let cmp = ma.cmp(&mb);
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                    if !stable_flag {
                        a.cmp(b)
                    } else {
                        Ordering::Equal
                    }
                });
            } else if numeric || human {
                all_lines.sort_by(|a, b| {
                    if human {
                        let ha = parse_human_size(a);
                        let hb = parse_human_size(b);
                        match (ha, hb) {
                            (Some(va), Some(vb)) => va.cmp(&vb),
                            (Some(_), None) => Ordering::Less,
                            (None, Some(_)) => Ordering::Greater,
                            (None, None) => {
                                let aa = if fold_case {
                                    a.to_lowercase()
                                } else {
                                    a.clone()
                                };
                                let bb = if fold_case {
                                    b.to_lowercase()
                                } else {
                                    b.clone()
                                };
                                aa.cmp(&bb)
                            }
                        }
                    } else {
                        let na = a.parse::<f64>().unwrap_or(f64::NAN);
                        let nb = b.parse::<f64>().unwrap_or(f64::NAN);
                        let cmp = na.partial_cmp(&nb).unwrap_or(Ordering::Equal);
                        if cmp != Ordering::Equal {
                            return cmp;
                        }
                        if !stable_flag {
                            let aa = if fold_case {
                                a.to_lowercase()
                            } else {
                                a.clone()
                            };
                            let bb = if fold_case {
                                b.to_lowercase()
                            } else {
                                b.clone()
                            };
                            aa.cmp(&bb)
                        } else {
                            Ordering::Equal
                        }
                    }
                });
            } else if fold_case {
                all_lines.sort_by(|a, b| {
                    let al = a.to_lowercase();
                    let bl = b.to_lowercase();
                    let cmp = al.cmp(&bl);
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                    if !stable_flag {
                        a.cmp(b)
                    } else {
                        Ordering::Equal
                    }
                });
            } else {
                all_lines.sort();
            }

            if reverse {
                all_lines.reverse();
            }
        }

        if reverse && !keys.is_empty() {
            all_lines.reverse();
        }

        if random {
            shuffle_lines(&mut all_lines);
        }

        let mut output = String::new();
        let mut prev: Option<&str> = None;
        for line in &all_lines {
            if unique {
                if prev == Some(line.as_str()) {
                    continue;
                }
                prev = Some(line.as_str());
            }
            output.push_str(line);
            output.push('\n');
        }

        if let Some(f) = output_file {
            match self.vfs.write(&f, &self.cwd, &output) {
                Ok(_) => CommandOutput::success(String::new()),
                Err(e) => CommandOutput::error(format!("sort: {}: {}\n", f, e), 1),
            }
        } else {
            CommandOutput::success(output)
        }
    }
}

/// Fisher–Yates shuffle with a time-seeded xorshift PRNG (no external deps).
fn shuffle_lines(lines: &mut [String]) {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15)
        .wrapping_add(std::process::id() as u64);
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in (1..lines.len()).rev() {
        let j = (next() as usize) % (i + 1);
        lines.swap(i, j);
    }
}

fn compare_keys(a: &SortKeyValue, b: &SortKeyValue) -> Ordering {
    match (a, b) {
        (SortKeyValue::Num(na), SortKeyValue::Num(nb)) => na.partial_cmp(nb).unwrap_or_else(|| {
            if na.is_nan() && !nb.is_nan() {
                Ordering::Greater
            } else if !na.is_nan() && nb.is_nan() {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        }),
        (SortKeyValue::HumanNum(ha), SortKeyValue::HumanNum(hb)) => ha.cmp(hb),
        (SortKeyValue::Version(va), SortKeyValue::Version(vb)) => compare_versions(va, vb),
        (SortKeyValue::Month(ma), SortKeyValue::Month(mb)) => ma.cmp(mb),
        (SortKeyValue::Str(sa), SortKeyValue::Str(sb)) => sa.cmp(sb),
        // Cross-type comparisons: numbers before strings.
        (SortKeyValue::Num(_), SortKeyValue::HumanNum(_)) => Ordering::Greater,
        (SortKeyValue::Num(_), SortKeyValue::Str(_)) => Ordering::Less,
        (SortKeyValue::HumanNum(_), SortKeyValue::Num(_)) => Ordering::Less,
        (SortKeyValue::HumanNum(_), SortKeyValue::Str(_)) => Ordering::Less,
        (SortKeyValue::Version(_), SortKeyValue::Str(_)) => Ordering::Less,
        (SortKeyValue::Month(_), SortKeyValue::Str(_)) => Ordering::Less,
        (SortKeyValue::Str(_), SortKeyValue::Num(_)) => Ordering::Greater,
        (SortKeyValue::Str(_), SortKeyValue::HumanNum(_)) => Ordering::Greater,
        (SortKeyValue::Str(_), SortKeyValue::Version(_)) => Ordering::Greater,
        (SortKeyValue::Str(_), SortKeyValue::Month(_)) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

/// Compare version numbers field-wise: `1.10` > `1.9`, `2.0.1` > `2.0`.
fn compare_versions(a: &str, b: &str) -> Ordering {
    let na: Vec<u64> = a
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u64>().unwrap_or(0))
        .collect();
    let nb: Vec<u64> = b
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u64>().unwrap_or(0))
        .collect();
    let len = na.len().max(nb.len());
    for i in 0..len {
        let x = na.get(i).copied().unwrap_or(0);
        let y = nb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

fn month_number(s: &str) -> Option<u8> {
    let lower = s.trim().to_lowercase();
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    months
        .iter()
        .position(|m| lower.starts_with(m))
        .map(|i| (i + 1) as u8)
}

fn compare_strings(a: &str, b: &str, fold_case: bool) -> Ordering {
    if fold_case {
        a.to_lowercase().cmp(&b.to_lowercase())
    } else {
        a.cmp(b)
    }
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
    fn test_sort_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("sort", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_sort_version() {
        let shell = mk_shell();
        let out = shell.cmd_sort(&["-V"], Some("v1.10\nv1.9\nv1.2\n"));
        let lines: Vec<&str> = out.stdout.lines().collect();
        assert_eq!(
            lines,
            vec!["v1.2", "v1.9", "v1.10"],
            "version sort should order numerically"
        );
    }

    #[test]
    fn test_sort_month() {
        let shell = mk_shell();
        let out = shell.cmd_sort(&["-M"], Some("Mar\nJan\nFeb\n"));
        let lines: Vec<&str> = out.stdout.lines().collect();
        assert_eq!(
            lines,
            vec!["Jan", "Feb", "Mar"],
            "month sort should order by calendar"
        );
    }
}
