// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const PING_HELP_TEXT: &str = "\
ping: TCP connectivity test
Usage: ping [OPTIONS] <host>[:port]
Options:
  -c COUNT    Number of probes to send (default 4)
  -W SEC      Timeout in seconds (default 2)
  -q          Quiet output
  -h, --help  Show this help message
";

impl Shell {
    pub fn cmd_ping(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(PING_HELP_TEXT.to_string());
        }
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
                _ => crate::warn!("ping: warning: unsupported option '{}'", args[i]),
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

        // ICMP is unavailable in the sandbox (no raw sockets / mobile
        // entitlements), so this is a TCP connect probe. When the caller does
        // not pin a port, probe a set of common ones so a host that only answers
        // on 443 (e.g. HTTPS-only) is still reported as reachable.
        let ports: Vec<u16> = if host.contains(':') {
            vec![port]
        } else {
            vec![443, 80, 22, 7]
        };
        let resolve = |p: u16| -> Option<std::net::SocketAddr> {
            format!("{}:{}", hostname, p).parse().ok().or_else(|| {
                (hostname, p)
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut it| it.next())
            })
        };
        // Validate the host resolves at all before reporting per-seq timeouts.
        if ports.iter().all(|&p| resolve(p).is_none()) {
            return CommandOutput::error(format!("ping: cannot resolve {}\n", hostname), 1);
        }

        let mut success = 0;
        let mut total_time = Duration::new(0, 0);
        let mut min_time = Duration::MAX;
        let mut max_time = Duration::new(0, 0);

        // Cap each connect attempt by the command's remaining budget so a
        // timed-out ping releases the runtime promptly.
        let base_timeout = Duration::from_secs(timeout_secs);
        let timeout = self
            .remaining_budget()
            .map(|b| b.min(base_timeout))
            .unwrap_or(base_timeout);

        let mut detail = String::new();
        let mut used_port = ports[0];

        for seq in 1..=count {
            let mut hit: Option<(u16, Duration)> = None;
            for &p in &ports {
                let Some(sa) = resolve(p) else { continue };
                let start = Instant::now();
                if TcpStream::connect_timeout(&sa, timeout).is_ok() {
                    hit = Some((p, start.elapsed()));
                    break;
                }
            }
            match hit {
                Some((p, rtt)) => {
                    used_port = p;
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
                            "TCP seq={} from {}:{} time={:.3} ms\n",
                            seq,
                            hostname,
                            p,
                            rtt.as_secs_f64() * 1000.0,
                        );
                    }
                }
                None => {
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
        output += &format!("TCP ping {} ({}:{})\n", hostname, hostname, used_port);
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

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;

    fn mk_shell() -> Shell {
        use std::fs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_ping_help() {
        let mut shell = mk_shell();
        let out = shell.execute("ping", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_ping_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("ping", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
