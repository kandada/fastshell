use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_pwd(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success(format!("{}\n", self.cwd))
    }
}
