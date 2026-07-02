// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::cmp::Ordering;

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
    Str(String),
}

fn extract_sort_key(
    line: &str,
    key_start: usize,
    key_end: Option<usize>,
    delimiter: Option<char>,
    fold_case: bool,
    numeric: bool,
    human: bool,
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
    } else {
        SortKeyValue::Str(key_ref)
    }
}

impl Shell {
    pub fn cmd_sort(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut files = Vec::new();
        let mut numeric = false;
        let mut reverse = false;
        let mut unique = false;
        let mut human = false;
        let mut fold_case = false;
        let mut stable_flag = false;
        let mut key_start: usize = 0;
        let mut key_end: Option<usize> = None;
        let mut delimiter: Option<char> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => numeric = true,
                "-r" => reverse = true,
                "-u" => unique = true,
                "-h" => human = true,
                "-f" => fold_case = true,
                "-s" => stable_flag = true,
                "-k" => {
                    if i + 1 < args.len() {
                        let spec = args[i + 1];
                        let parts: Vec<&str> = spec.splitn(2, ',').collect();
                        key_start = parts[0].parse().unwrap_or(0);
                        key_end = if parts.len() > 1 && !parts[1].is_empty() {
                            parts[1].parse().ok()
                        } else {
                            None
                        };
                        i += 1;
                    }
                }
                "-t" => {
                    if i + 1 < args.len() {
                        let sep = args[i + 1];
                        delimiter = sep.chars().next();
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
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
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(content) => {
                        for line in content.lines() {
                            all_lines.push(line.to_string());
                        }
                    }
                    Err(e) => return CommandOutput::error(format!("sort: {}: {}\n", file, e), 1),
                }
            }
        }

        if key_start > 0 {
            let mut indexed: Vec<(SortKeyValue, String)> = all_lines
                .into_iter()
                .map(|line| {
                    let key = extract_sort_key(
                        &line, key_start, key_end, delimiter, fold_case, numeric, human, &line,
                    );
                    (key, line)
                })
                .collect();

            indexed.sort_by(|a, b| {
                let cmp = compare_keys(&a.0, &b.0);
                if cmp != Ordering::Equal {
                    return cmp;
                }
                if !stable_flag {
                    compare_strings(&a.1, &b.1, fold_case)
                } else {
                    Ordering::Equal
                }
            });

            all_lines = indexed.into_iter().map(|(_, line)| line).collect();
        } else {
            if numeric || human {
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

        if reverse && key_start > 0 {
            all_lines.reverse();
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

        CommandOutput::success(output)
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
        (SortKeyValue::Str(sa), SortKeyValue::Str(sb)) => sa.cmp(sb),
        // Cross-type comparisons: numbers before strings
        (SortKeyValue::Num(_), SortKeyValue::HumanNum(_)) => Ordering::Greater,
        (SortKeyValue::Num(_), SortKeyValue::Str(_)) => Ordering::Less,
        (SortKeyValue::HumanNum(_), SortKeyValue::Num(_)) => Ordering::Less,
        (SortKeyValue::HumanNum(_), SortKeyValue::Str(_)) => Ordering::Less,
        (SortKeyValue::Str(_), SortKeyValue::Num(_)) => Ordering::Greater,
        (SortKeyValue::Str(_), SortKeyValue::HumanNum(_)) => Ordering::Greater,
    }
}

fn compare_strings(a: &str, b: &str, fold_case: bool) -> Ordering {
    if fold_case {
        a.to_lowercase().cmp(&b.to_lowercase())
    } else {
        a.cmp(b)
    }
}
