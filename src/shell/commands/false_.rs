use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_false_(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 1,
        }
    }
}
