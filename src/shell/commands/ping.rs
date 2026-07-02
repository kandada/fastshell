// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::net::TcpStream;
use std::time::{Duration, Instant};

impl Shell {
    pub fn cmd_ping(&self, args: &[&str]) -> CommandOutput {
        let mut count: usize = 4;
        let mut timeout_secs: u64 = 2;
        let mut quiet = false;
        let mut host: Option<String> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-c" => {
                    if i + 1 < args.len() {
                        count = args[i + 1].parse().unwrap_or(4);
                        i += 1;
                    }
                }
                "-W" => {
                    if i + 1 < args.len() {
                        timeout_secs = args[i + 1].parse().unwrap_or(2);
                        i += 1;
                    }
                }
                "-q" => quiet = true,
                arg if !arg.starts_with('-') => host = Some(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let host = match host {
            Some(h) => h,
            None => return CommandOutput::error("ping: missing host\n".to_string(), 1),
        };

        let hostname = host.split(':').next().unwrap_or(&host);
        if let Some(perm) = self.check_network_permission(hostname) {
            return perm;
        }

        let port = if host.contains(':') {
            let parts: Vec<&str> = host.split(':').collect();
            parts[1].parse().unwrap_or(80)
        } else {
            80
        };

        let addr = format!("{}:{}", hostname, port);

        let mut success = 0;
        let mut total_time = Duration::new(0, 0);
        let mut min_time = Duration::MAX;
        let mut max_time = Duration::new(0, 0);

        let timeout = Duration::from_secs(timeout_secs);

        let socket_addr = match addr.parse() {
            Ok(a) => a,
            Err(_) => {
                return CommandOutput::error(format!("ping: cannot resolve {}\n", hostname), 1)
            }
        };

        let mut detail = String::new();

        for seq in 1..=count {
            let start = Instant::now();
            match TcpStream::connect_timeout(&socket_addr, timeout) {
                Ok(_) => {
                    let rtt = start.elapsed();
                    success += 1;
                    total_time += rtt;
                    if rtt < min_time {
                        min_time = rtt;
                    }
                    if rtt > max_time {
                        max_time = rtt;
                    }
                    if !quiet {
                        detail += &format!(
                            "TCP seq={} from {} time={:.3} ms\n",
                            seq,
                            addr,
                            rtt.as_secs_f64() * 1000.0,
                        );
                    }
                }
                Err(_) => {
                    if !quiet {
                        detail += &format!("ping: seq={} timeout\n", seq);
                    }
                }
            }
        }

        let loss_pct = if count > 0 {
            ((count - success) as f64 / count as f64) * 100.0
        } else {
            0.0
        };

        let avg_time = if success > 0 {
            total_time / success as u32
        } else {
            Duration::new(0, 0)
        };

        let mut output = detail;
        output += &format!("TCP ping {} ({}:{})\n", hostname, hostname, port);
        output += &format!(
            "{} packets transmitted, {} received, {:.0}% loss\n",
            count, success, loss_pct
        );
        if success > 0 {
            output += &format!(
                "min/avg/max = {:.3}/{:.3}/{:.3} ms\n",
                min_time.as_secs_f64() * 1000.0,
                avg_time.as_secs_f64() * 1000.0,
                max_time.as_secs_f64() * 1000.0,
            );
        }

        if success == 0 {
            CommandOutput::error(output, 1)
        } else {
            CommandOutput::success(output)
        }
    }
}
