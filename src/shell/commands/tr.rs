// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_tr(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut delete = false;
        let mut squeeze = false;
        let mut complement = false;
        let mut set1 = String::new();
        let mut set2 = String::new();

        for arg in args {
            match *arg {
                "-d" => delete = true,
                "-s" => squeeze = true,
                "-c" => complement = true,
                _ if !arg.starts_with('-') && set1.is_empty() => set1 = arg.to_string(),
                _ if !arg.starts_with('-') && set2.is_empty() => set2 = arg.to_string(),
                _ => {}
            }
        }

        if set1.is_empty() {
            return CommandOutput::error("tr: missing operand\n".to_string(), 1);
        }

        let input = match stdin {
            Some(s) => s.to_string(),
            None => return CommandOutput::error("tr: missing input\n".to_string(), 1),
        };

        let set1_expanded = resolve_char_classes(&set1);
        let chars1_raw = expand_set(&unescape_tr(&set1_expanded));
        let chars1: Vec<char> = if complement {
            complement_set(&chars1_raw)
        } else {
            chars1_raw
        };

        let set2_expanded = resolve_char_classes(&set2);
        let chars2 = expand_set(&unescape_tr(&set2_expanded));

        let squeeze_chars: &[char] = if squeeze { &chars1 } else { &[] };

        let mut output = String::new();
        for ch in input.chars() {
            if delete {
                if !chars1.contains(&ch) {
                    output.push(ch);
                }
            } else {
                if let Some(pos) = chars1.iter().position(|&c| c == ch) {
                    if pos < chars2.len() {
                        output.push(chars2[pos]);
                    } else {
                        output.push(*chars2.last().unwrap_or(&ch));
                    }
                } else {
                    output.push(ch);
                }
            }
        }

        if squeeze {
            let mut squeezed = String::new();
            let mut prev: Option<char> = None;
            for ch in output.chars() {
                if squeeze_chars.contains(&ch) {
                    if Some(ch) != prev {
                        squeezed.push(ch);
                    }
                } else {
                    squeezed.push(ch);
                }
                prev = Some(ch);
            }
            output = squeezed;
        }

        CommandOutput::success(output)
    }
}

fn resolve_char_classes(s: &str) -> String {
    let classes: &[(&str, &str)] = &[
        ("[:alpha:]", "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"),
        ("[:digit:]", "0123456789"),
        ("[:alnum:]", "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"),
        ("[:lower:]", "abcdefghijklmnopqrstuvwxyz"),
        ("[:upper:]", "ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
        ("[:space:]", " \t\n\r\x0b\x0c"),
        ("[:punct:]", "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"),
        ("[:print:]", "!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~ "),
        ("[:graph:]", "!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"),
        ("[:cntrl:]", "\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f\x7f"),
        ("[:xdigit:]", "0123456789ABCDEFabcdef"),
    ];

    let mut result = s.to_string();
    for (name, chars) in classes {
        result = result.replace(name, chars);
    }
    result
}

fn complement_set(chars: &[char]) -> Vec<char> {
    let mut result = Vec::new();
    for c in (0u32..=127).filter_map(char::from_u32) {
        if !chars.contains(&c) {
            result.push(c);
        }
    }
    result
}

fn expand_set(s: &str) -> Vec<char> {
    let mut result = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i] < chars[i + 2] {
            for c in (chars[i] as u32)..=(chars[i + 2] as u32) {
                if let Some(ch) = char::from_u32(c) {
                    result.push(ch);
                }
            }
            i += 3;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn unescape_tr(s: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'n' => result.push('\n'),
                't' => result.push('\t'),
                'r' => result.push('\r'),
                '\\' => result.push('\\'),
                '0' => {
                    let mut octal = String::new();
                    let mut j = i + 1;
                    while j < chars.len()
                        && j < i + 4
                        && chars[j].is_ascii_digit()
                        && chars[j] < '8'
                    {
                        octal.push(chars[j]);
                        j += 1;
                    }
                    if let Ok(n) = u32::from_str_radix(&octal, 8) {
                        if let Some(ch) = char::from_u32(n) {
                            result.push(ch);
                        }
                    }
                    i = j - 1;
                }
                _ => {
                    result.push(chars[i + 1]);
                }
            }
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}
