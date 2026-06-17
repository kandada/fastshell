use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_ps(&self, args: &[&str]) -> CommandOutput {
        let mut format: Option<String> = None;
        let mut pids: Vec<u32> = Vec::new();
        let mut user_filter: Option<u32> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-o" => {
                    if i + 1 < args.len() {
                        format = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-p" => {
                    if i + 1 < args.len() {
                        for pid_str in args[i + 1].split(',') {
                            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                                pids.push(pid);
                            }
                        }
                        i += 1;
                    }
                }
                "-u" => {
                    if i + 1 < args.len() {
                        user_filter = args[i + 1].parse::<u32>().ok();
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        let processes = crate::shell::list_processes();
        match processes {
            Ok(procs) => {
                let filtered: Vec<&crate::shell::ProcInfo> = procs.iter().filter(|p| {
                    let pid_ok = pids.is_empty() || pids.contains(&p.pid);
                    let user_ok = match user_filter {
                        Some(uid) => p.uid == uid,
                        None => true,
                    };
                    pid_ok && user_ok
                }).collect();

                if let Some(ref fmt) = format {
                    let fields: Vec<&str> = fmt.split(',').map(|s| s.trim()).collect();
                    let mut output = String::new();
                    for p in &filtered {
                        let mut parts = Vec::new();
                        for field in &fields {
                            parts.push(match *field {
                                "pid" => format!("{}", p.pid),
                                "ppid" => format!("{}", p.ppid),
                                "rss" => format!("{}", p.rss),
                                "pcpu" => format!("{:.1}", p.cpu_pct),
                                "comm" => p.comm.clone(),
                                "state" => "?".to_string(),
                                _ => String::new(),
                            });
                        }
                        output.push_str(&parts.join(" "));
                        output.push('\n');
                    }
                    return CommandOutput::success(output);
                }

                let mut output = String::new();
                output.push_str(&format!(
                    "{:>8} {:>8} {:>8} {:>8} {}\n",
                    "PID", "PPID", "%CPU", "RSS", "COMMAND"
                ));
                for p in &filtered {
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
