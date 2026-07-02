// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_sed(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut expression: Option<String> = None;
        let mut files = Vec::new();
        let mut in_place = false;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-e" => {
                    if i + 1 < args.len() {
                        expression = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-i" => in_place = true,
                arg if arg.starts_with("-e") => {
                    expression = Some(arg[2..].to_string());
                }
                arg if !arg.starts_with('-') && expression.is_none() => {
                    expression = Some(arg.to_string());
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let expr = match expression {
            Some(e) => e,
            None => return CommandOutput::error("sed: missing expression\n".to_string(), 1),
        };

        let parsed = parse_sed_command(&expr);

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    let processed = apply_sed_commands(input, &parsed);
                    return CommandOutput::success(processed);
                }
                None => return CommandOutput::error("sed: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        for file in &files {
            let content = match self.vfs.read_to_string(file, &self.cwd) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("sed: {}: {}\n", file, e), 1),
            };

            let processed = apply_sed_commands(&content, &parsed);

            if in_place {
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

enum SedCommand {
    Substitute {
        pattern: String,
        replacement: String,
        global: bool,
        regex: Option<String>,
    },
    Delete,
    DeleteByPattern(String),
}

fn parse_sed_command(expr: &str) -> Vec<SedCommand> {
    let mut commands = Vec::new();
    for part in expr.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some(inner) = part.strip_prefix('/') {
            if let Some(slash_pos) = inner.rfind('/') {
                let pattern = &inner[..slash_pos];
                let rest = &inner[slash_pos + 1..];
                if rest == "d" {
                    commands.push(SedCommand::DeleteByPattern(pattern.to_string()));
                    continue;
                }
            }
        }

        let first_char = part.chars().next().unwrap_or('s');
        if first_char == 's' {
            let delim = part.chars().nth(1).unwrap_or('/');
            let rest = &part[2..];
            let parts: Vec<&str> = rest.splitn(3, delim).collect();
            if parts.len() >= 2 {
                let pattern = parts[0].to_string();
                let replacement = if parts.len() > 1 {
                    parts[1].to_string()
                } else {
                    String::new()
                };
                let flags = parts.get(2).unwrap_or(&"");
                let global = flags.contains('g');

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
                    pattern,
                    replacement,
                    global,
                    regex,
                });
            }
        } else if first_char == 'd' || part == "d" {
            commands.push(SedCommand::Delete);
        }
    }
    commands
}

fn apply_sed_commands(content: &str, commands: &[SedCommand]) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut output = String::new();

    for &line in &lines {
        let mut keep = true;
        let mut modified = line.to_string();

        for cmd in commands {
            match cmd {
                SedCommand::Delete => {
                    keep = false;
                    break;
                }
                SedCommand::DeleteByPattern(ref pattern) => {
                    if line.contains(pattern) {
                        keep = false;
                        break;
                    }
                }
                SedCommand::Substitute {
                    ref pattern,
                    ref replacement,
                    global,
                    ref regex,
                } => {
                    let prev = modified.clone();
                    if let Some(re) = regex {
                        match regex::Regex::new(re) {
                            Ok(re) => {
                                if *global {
                                    modified =
                                        re.replace_all(&modified, replacement.as_str()).to_string();
                                } else {
                                    modified =
                                        re.replace(&modified, replacement.as_str()).to_string();
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
                }
            }
        }

        if keep {
            output.push_str(&modified);
            output.push('\n');
        }
    }

    output
}
