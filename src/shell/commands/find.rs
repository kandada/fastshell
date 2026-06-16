use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_find(&self, args: &[&str]) -> CommandOutput {
        let mut path = ".".to_string();
        let mut name_pattern: Option<String> = None;
        let mut type_filter: Option<char> = None;
        let mut i = 0;

        while i < args.len() {
            match args[i] {
                "-name" => {
                    if i + 1 < args.len() {
                        name_pattern = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-type" => {
                    if i + 1 < args.len() {
                        type_filter = args[i + 1].chars().next();
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') && i == 0 => {
                    path = arg.to_string();
                }
                _ => {}
            }
            i += 1;
        }

        let compiled_pattern = name_pattern.map(|p| glob_to_regex(&p));

        let mut output = String::new();
        if let Err(e) = self.find_recursive(&path, &compiled_pattern, &type_filter, &mut output) {
            return CommandOutput::error(format!("find: {}\n", e), 1);
        }
        CommandOutput::success(output)
    }

    fn find_recursive(
        &self,
        path: &str,
        name_regex: &Option<regex::Regex>,
        type_filter: &Option<char>,
        output: &mut String,
    ) -> Result<(), crate::vfs::VfsError> {
        let entries = self.vfs.list_dir(path, &self.cwd)?;

        for entry in &entries {
            let entry_path = format!("{}/{}", path.trim_end_matches('/'), entry.name);

            let name_matches = match name_regex {
                Some(re) => re.is_match(&entry.name),
                None => true,
            };

            if name_matches {
                let type_matches = match type_filter {
                    Some('d') => entry.is_dir,
                    Some('f') => !entry.is_dir,
                    _ => true,
                };

                if type_matches {
                    output.push_str(&format!("{}\n", entry_path));
                }
            }

            if entry.is_dir {
                let _ = self.find_recursive(&entry_path, name_regex, type_filter, output);
            }
        }

        Ok(())
    }
}

fn glob_to_regex(pattern: &str) -> regex::Regex {
    let mut regex_str = String::new();
    regex_str.push('^');
    for ch in pattern.chars() {
        match ch {
            '*' => regex_str.push_str(".*"),
            '?' => regex_str.push('.'),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                regex_str.push('\\');
                regex_str.push(ch);
            }
            _ => regex_str.push(ch),
        }
    }
    regex_str.push('$');
    regex::Regex::new(&regex_str).unwrap_or_else(|_| {
        let escaped = regex::escape(pattern);
        regex::Regex::new(&format!("^{}$", escaped)).unwrap()
    })
}
