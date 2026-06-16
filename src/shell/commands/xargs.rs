use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_xargs(&mut self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut target_cmd: Vec<String> = Vec::new();
        let mut parsing_cmd = false;

        for arg in args {
            if !arg.starts_with('-') {
                parsing_cmd = true;
            }
            if parsing_cmd {
                target_cmd.push(arg.to_string());
            }
        }

        if target_cmd.is_empty() {
            return CommandOutput::error("xargs: missing command\n".to_string(), 1);
        }

        let input = match stdin {
            Some(s) => s.to_string(),
            None => return CommandOutput::error("xargs: no input\n".to_string(), 1),
        };

        let args_from_stdin: Vec<&str> = input.split_whitespace().collect();
        let cmd_name = &target_cmd[0];
        let mut all_args: Vec<&str> = target_cmd[1..].iter().map(|s| s.as_str()).collect();
        all_args.extend(&args_from_stdin);

        self.execute(cmd_name, &all_args, None)
    }
}
