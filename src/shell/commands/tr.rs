use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_tr(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut delete = false;
        let mut squeeze = false;
        let mut set1 = String::new();
        let mut set2 = String::new();

        for arg in args {
            match *arg {
                "-d" => delete = true,
                "-s" => squeeze = true,
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

        let chars1 = expand_set(&unescape_tr(&set1));
        let chars2 = expand_set(&unescape_tr(&set2));

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
                    while j < chars.len() && j < i + 4 && chars[j].is_ascii_digit() && chars[j] < '8' {
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
