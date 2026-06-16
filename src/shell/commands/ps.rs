use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_ps(&self, _args: &[&str]) -> CommandOutput {
        let processes = crate::shell::list_processes();
        match processes {
            Ok(procs) => {
                let mut output = String::new();
                output.push_str(&format!(
                    "{:>8} {:>8} {:>8} {:>8} {}\n",
                    "PID", "PPID", "%CPU", "RSS", "COMMAND"
                ));
                for p in &procs {
                    output.push_str(&format!(
                        "{:>8} {:>8} {:>7.1} {:>8} {}\n",
                        p.pid, p.ppid, p.cpu_pct, crate::shell::human_size(p.rss * 1024), p.comm
                    ));
                }
                CommandOutput::success(output)
            }
            Err(e) => CommandOutput::error(format!("ps: {}\n", e), 1),
        }
    }
}
