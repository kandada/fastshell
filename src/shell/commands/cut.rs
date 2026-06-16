use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_cut(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut delimiter = '\t';
        let mut fields: Vec<FieldRange> = Vec::new();
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-d" => {
                    if i + 1 < args.len() {
                        delimiter = args[i + 1].chars().next().unwrap_or('\t');
                        i += 1;
                    }
                }
                "-f" => {
                    if i + 1 < args.len() {
                        fields = parse_field_spec(args[i + 1]);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-d") && arg.len() > 2 => {
                    delimiter = arg[2..].chars().next().unwrap_or('\t');
                }
                arg if arg.starts_with("-f") && arg.len() > 2 => {
                    fields = parse_field_spec(&arg[2..]);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        if fields.is_empty() {
            return CommandOutput::error("cut: missing field list\n".to_string(), 1);
        }

        let input = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("cut: missing input\n".to_string(), 1),
            }
        } else {
            let mut content = String::new();
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(c) => content.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("cut: {}: {}\n", file, e), 1),
                }
            }
            content
        };

        let mut output = String::new();
        for line in input.lines() {
            let cols: Vec<&str> = line.split(delimiter).collect();
            let total_cols = cols.len();
            let mut parts = Vec::new();
            for range in &fields {
                let start = range.start.saturating_sub(1);
                let end = match range.end {
                    Some(e) => (e.saturating_sub(1)).min(total_cols.saturating_sub(1)),
                    None => total_cols.saturating_sub(1),
                };
                if start <= end && start < total_cols {
                    for idx in start..=end {
                        parts.push(cols[idx].to_string());
                    }
                }
            }
            if !parts.is_empty() {
                output.push_str(&parts.join(&delimiter.to_string()));
            }
            output.push('\n');
        }

        CommandOutput::success(output)
    }
}

struct FieldRange {
    start: usize,
    end: Option<usize>,
}

fn parse_field_spec(spec: &str) -> Vec<FieldRange> {
    let mut fields = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part == "-" {
            continue;
        }
        if part.contains('-') {
            let mut range = part.splitn(2, '-');
            let start_str = range.next().unwrap_or("");
            let end_str = range.next().unwrap_or("");
            let start: usize = if start_str.is_empty() {
                1
            } else {
                start_str.parse().unwrap_or(1)
            };
            let end: Option<usize> = if end_str.is_empty() {
                None
            } else {
                Some(end_str.parse().unwrap_or(1))
            };
            fields.push(FieldRange { start, end });
        } else {
            let n: usize = part.parse().unwrap_or(1);
            fields.push(FieldRange { start: n, end: Some(n) });
        }
    }
    fields
}
