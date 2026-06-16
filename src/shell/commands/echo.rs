use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_echo(&self, args: &[&str]) -> CommandOutput {
        let mut no_newline = false;
        let mut start = 0;

        while start < args.len() {
            if args[start] == "-n" {
                no_newline = true;
                start += 1;
            } else {
                break;
            }
        }

        let output = args[start..].join(" ");
        if no_newline {
            CommandOutput::success(output)
        } else {
            CommandOutput::success(output + "\n")
        }
    }
}
