use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_true(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success(String::new())
    }
}
