// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::collections::BTreeMap;

impl Shell {
    pub fn cmd_set(&mut self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            let mut sorted: BTreeMap<&String, &String> = BTreeMap::new();
            for (k, v) in &self.vars {
                sorted.insert(k, v);
            }
            let mut out = String::new();
            for (k, v) in &sorted {
                out.push_str(&format!("{}={}\n", k, v));
            }
            return CommandOutput::success(out);
        }

        let arg = args[0];

        match arg {
            "-e" => {
                self.errexit = true;
                CommandOutput::success(String::new())
            }
            "+e" => {
                self.errexit = false;
                CommandOutput::success(String::new())
            }
            "-x" => {
                self.xtrace = true;
                CommandOutput::success(String::new())
            }
            "+x" => {
                self.xtrace = false;
                CommandOutput::success(String::new())
            }
            "-u" => {
                self.nounset = true;
                CommandOutput::success(String::new())
            }
            "+u" => {
                self.nounset = false;
                CommandOutput::success(String::new())
            }
            "-o" => {
                if args.len() == 1 {
                    let mut out = String::new();
                    out.push_str(&format!(
                        "errexit\t\t{}\n",
                        if self.errexit { "on" } else { "off" }
                    ));
                    out.push_str(&format!(
                        "noclobber\t{}\n",
                        if self.noclobber { "on" } else { "off" }
                    ));
                    out.push_str(&format!(
                        "nounset\t\t{}\n",
                        if self.nounset { "on" } else { "off" }
                    ));
                    out.push_str(&format!(
                        "pipefail\t{}\n",
                        if self.pipefail { "on" } else { "off" }
                    ));
                    out.push_str(&format!(
                        "xtrace\t\t{}\n",
                        if self.xtrace { "on" } else { "off" }
                    ));
                    CommandOutput::success(out)
                } else {
                    match args[1] {
                        "pipefail" => {
                            self.pipefail = true;
                            CommandOutput::success(String::new())
                        }
                        "noclobber" => {
                            self.noclobber = true;
                            CommandOutput::success(String::new())
                        }
                        "errexit" => {
                            self.errexit = true;
                            CommandOutput::success(String::new())
                        }
                        "nounset" => {
                            self.nounset = true;
                            CommandOutput::success(String::new())
                        }
                        "xtrace" => {
                            self.xtrace = true;
                            CommandOutput::success(String::new())
                        }
                        other => CommandOutput::error(
                            format!("set: {}: invalid option name\n", other),
                            1,
                        ),
                    }
                }
            }
            "+o" => {
                if args.len() == 1 {
                    let mut out = String::new();
                    out.push_str("set +o errexit\n");
                    out.push_str("set +o noclobber\n");
                    out.push_str("set +o nounset\n");
                    out.push_str("set +o pipefail\n");
                    out.push_str("set +o xtrace\n");
                    CommandOutput::success(out)
                } else {
                    match args[1] {
                        "pipefail" => {
                            self.pipefail = false;
                            CommandOutput::success(String::new())
                        }
                        "noclobber" => {
                            self.noclobber = false;
                            CommandOutput::success(String::new())
                        }
                        "errexit" => {
                            self.errexit = false;
                            CommandOutput::success(String::new())
                        }
                        "nounset" => {
                            self.nounset = false;
                            CommandOutput::success(String::new())
                        }
                        "xtrace" => {
                            self.xtrace = false;
                            CommandOutput::success(String::new())
                        }
                        other => CommandOutput::error(
                            format!("set: {}: invalid option name\n", other),
                            1,
                        ),
                    }
                }
            }
            other => CommandOutput::error(format!("set: {}: invalid option\n", other), 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fastshell_set_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        Shell::new(Vfs::new(dir).unwrap())
    }

    #[test]
    fn test_set_no_args() {
        let mut shell = mk_shell();
        shell.vars.insert("X".to_string(), "foo".to_string());
        shell.vars.insert("A".to_string(), "bar".to_string());
        let out = shell.cmd_set(&[]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("A=bar"));
        assert!(out.stdout.contains("X=foo"));
    }

    #[test]
    fn test_set_e_toggle() {
        let mut shell = mk_shell();
        assert!(!shell.errexit);
        let out = shell.cmd_set(&["-e"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.errexit);
        let out = shell.cmd_set(&["+e"]);
        assert_eq!(out.exit_code, 0);
        assert!(!shell.errexit);
    }

    #[test]
    fn test_set_x_toggle() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-x"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.xtrace);
        let out = shell.cmd_set(&["+x"]);
        assert_eq!(out.exit_code, 0);
        assert!(!shell.xtrace);
    }

    #[test]
    fn test_set_u_toggle() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-u"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.nounset);
        let out = shell.cmd_set(&["+u"]);
        assert_eq!(out.exit_code, 0);
        assert!(!shell.nounset);
    }

    #[test]
    fn test_set_o_pipefail() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-o", "pipefail"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.pipefail);
        let out = shell.cmd_set(&["+o", "pipefail"]);
        assert_eq!(out.exit_code, 0);
        assert!(!shell.pipefail);
    }

    #[test]
    fn test_set_o_noclobber() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-o", "noclobber"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.noclobber);
        let out = shell.cmd_set(&["+o", "noclobber"]);
        assert_eq!(out.exit_code, 0);
        assert!(!shell.noclobber);
    }

    #[test]
    fn test_set_o_list() {
        let mut shell = mk_shell();
        shell.xtrace = true;
        let out = shell.cmd_set(&["-o"]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("errexit"));
        assert!(out.stdout.contains("xtrace"));
    }

    #[test]
    fn test_set_plus_o_list() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["+o"]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("set +o"));
    }

    #[test]
    fn test_set_unknown_option() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-z"]);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_set_o_invalid_name() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-o", "nonexistent"]);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_set_o_errexit() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-o", "errexit"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.errexit);
    }

    #[test]
    fn test_set_o_nounset() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-o", "nounset"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.nounset);
    }

    #[test]
    fn test_set_o_xtrace() {
        let mut shell = mk_shell();
        let out = shell.cmd_set(&["-o", "xtrace"]);
        assert_eq!(out.exit_code, 0);
        assert!(shell.xtrace);
    }
}
