use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_cat(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut show_numbers = false;
        let mut output = String::new();
        let mut found_file = false;
        let mut line_num = 1usize;

        if args.is_empty() {
            match stdin {
                Some(s) => {
                    output.push_str(s);
                    return CommandOutput::success(output);
                }
                None => return CommandOutput::error("cat: missing file operand\n".to_string(), 1),
            }
        }

        for arg in args {
            match *arg {
                "-n" => show_numbers = true,
                "-" => {
                    found_file = true;
                    if let Some(ref s) = stdin {
                        for line in s.lines() {
                            if show_numbers {
                                output.push_str(&format!("{:>6}\t{}\n", line_num, line));
                                line_num += 1;
                            } else {
                                output.push_str(line);
                                output.push('\n');
                            }
                        }
                    }
                }
                arg if arg.starts_with('-') && arg.len() > 1 => {}
                _ => {
                    found_file = true;
                    match self.vfs.read_to_string(arg, &self.cwd) {
                        Ok(content) => {
                            if show_numbers {
                                for line in content.lines() {
                                    output.push_str(&format!("{:>6}\t{}\n", line_num, line));
                                    line_num += 1;
                                }
                            } else {
                                output.push_str(&content);
                            }
                        }
                        Err(e) => {
                            return CommandOutput::error(format!("cat: {}: {}\n", arg, e), 1);
                        }
                    }
                }
            }
        }

        if !found_file && stdin.is_some() {
            output.push_str(stdin.unwrap());
        }

        CommandOutput::success(output)
    }
}
