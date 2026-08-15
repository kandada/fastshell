// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::commands::devices::plugin;
use crate::shell::{CommandOutput, Shell};

const NODE_USAGE: &str = "\
Usage: node [OPTION]... [FILE|-]
Run JavaScript in the host app's WebView engine (Android V8 / iOS JavaScriptCore).

  -e, --eval CODE    evaluate CODE and print its result
  -p, --print CODE   evaluate CODE and print the last expression value
  -c, --check FILE   syntax-check FILE (oxc static parser, no execution)
  -v, --version      print version
  FILE               read and run a script file
  -                  read the script from stdin
  (no args)          read the script from stdin
";

impl Shell {
    /// `node` / `js` — execute JavaScript through the host WebView engine.
    pub fn cmd_node(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        match args.first().copied() {
            None | Some("-") => {
                match stdin {
                    Some(code) => self.run_js(code),
                    None => CommandOutput::error("node: no input (pipe a script)\n".to_string(), 1),
                }
            }
            Some("-h") | Some("--help") => CommandOutput::success(NODE_USAGE.to_string()),
            Some("-v") | Some("--version") => {
                CommandOutput::success("javascript (host webview engine)\n".to_string())
            }
            Some("-e") | Some("--eval") | Some("-p") | Some("--print") => {
                let code = args.get(1).copied().unwrap_or("");
                if code.is_empty() {
                    return CommandOutput::error(
                        format!("node: {} requires code\n", args[0]),
                        1,
                    );
                }
                self.run_js(code)
            }
            Some("-c") | Some("--check") => {
                let file = args.get(1).copied().unwrap_or("");
                if file.is_empty() {
                    return CommandOutput::error("node: --check requires a file\n".to_string(), 1);
                }
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(code) => self.check_js(&code, file),
                    Err(e) => CommandOutput::error(format!("node: {file}: {e}\n"), 1),
                }
            }
            Some(file) => match self.vfs.read_to_string(file, &self.cwd) {
                Ok(code) => self.run_js(&code),
                Err(e) => CommandOutput::error(format!("node: {file}: {e}\n"), 1),
            },
        }
    }

    /// `jscheck` / `jslint` — static syntax check via oxc (feature `js-oxc`).
    pub fn cmd_jscheck(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut hint = "stdin.js";
        let mut file: Option<&str> = None;
        for &a in args {
            match a {
                "--ts" | "--typescript" => hint = "stdin.ts",
                "--tsx" => hint = "stdin.tsx",
                "--jsx" => hint = "stdin.jsx",
                "--mjs" | "--module" => hint = "stdin.mjs",
                arg if !arg.starts_with('-') => file = Some(arg),
                _ => {}
            }
        }

        let (code, filename) = if let Some(f) = file {
            match self.vfs.read_to_string(f, &self.cwd) {
                Ok(c) => (c, f.to_string()),
                Err(e) => return CommandOutput::error(format!("jscheck: {f}: {e}\n"), 1),
            }
        } else if let Some(s) = stdin {
            (s.to_string(), hint.to_string())
        } else {
            return CommandOutput::error(
                "jscheck: no input (pass a file or pipe source)\n".to_string(),
                1,
            );
        };

        self.check_js(&code, &filename)
    }

    /// `render` — render HTML in the host WebView and capture a screenshot.
    pub fn cmd_render(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut html_file: Option<&str> = None;
        let mut out_path = "/render.png";
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            match a {
                "-o" | "--output" => {
                    if i + 1 < args.len() {
                        out_path = args[i + 1];
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => {
                    if html_file.is_none() {
                        html_file = Some(arg);
                    } else {
                        out_path = arg;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        let html = if let Some(f) = html_file {
            match self.vfs.read_to_string(f, &self.cwd) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("render: {f}: {e}\n"), 1),
            }
        } else if let Some(s) = stdin {
            s.to_string()
        } else {
            return CommandOutput::error(
                "render: no input (pass an HTML file or pipe HTML)\n".to_string(),
                1,
            );
        };

        let host = match self.device_host_path(out_path) {
            Ok(h) => h,
            Err(e) => return e,
        };
        match plugin(self, |p| p.render_html(&html, &host)) {
            Ok(()) => CommandOutput::success(format!("Rendered to {out_path}\n")),
            Err(e) => e,
        }
    }

    fn run_js(&self, code: &str) -> CommandOutput {
        match plugin(self, |p| p.eval_js(code)) {
            Ok(result) => CommandOutput::success(if result.ends_with('\n') {
                result
            } else {
                format!("{result}\n")
            }),
            Err(e) => e,
        }
    }

    fn check_js(&self, code: &str, filename: &str) -> CommandOutput {
        match crate::js::check_syntax(code, filename) {
            Ok(()) => CommandOutput::success(format!("{filename}: syntax OK\n")),
            Err(e) => CommandOutput::error(e, 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn shell() -> Shell {
        let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fs_js_{}_{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::create_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn node_without_plugin_returns_not_supported() {
        let s = shell();
        let out = s.cmd_node(&["-e", "1+1"], None);
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("not supported"), "stderr={}", out.stderr);
    }

    #[test]
    fn node_version_and_help() {
        let s = shell();
        let out = s.cmd_node(&["--version"], None);
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("javascript"), "stdout={}", out.stdout);

        let out = s.cmd_node(&["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage"), "stdout={}", out.stdout);
    }

    #[test]
    fn node_missing_code_errors() {
        let s = shell();
        let out = s.cmd_node(&["-e"], None);
        assert_ne!(out.exit_code, 0, "-e without code must fail");

        let out = s.cmd_node(&[], None);
        assert_ne!(out.exit_code, 0, "no input must fail");
    }

    #[cfg(feature = "js-oxc")]
    #[test]
    fn jscheck_reports_syntax_error() {
        let s = shell();
        let out = s.cmd_jscheck(&["--js"], Some("const = 1;"));
        assert_ne!(out.exit_code, 0);
        assert!(!out.stderr.is_empty());
    }

    #[cfg(feature = "js-oxc")]
    #[test]
    fn jscheck_accepts_valid_source() {
        let s = shell();
        let out = s.cmd_jscheck(&["--js"], Some("const x = 1;"));
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("syntax OK"));
    }

    #[cfg(feature = "js-oxc")]
    #[test]
    fn jscheck_reads_file_and_selects_dialect_by_extension() {
        let s = shell();
        let _ = s.vfs.write("bad.ts", "/", "const n: number = ;");
        let out = s.cmd_jscheck(&["bad.ts"], None);
        assert_ne!(out.exit_code, 0, "ts file must be parsed and flagged");
        assert!(!out.stderr.is_empty());

        let _ = s.vfs.write("ok.tsx", "/", "const C: React.FC = () => <div/>;");
        let out = s.cmd_jscheck(&["ok.tsx"], None);
        assert_eq!(out.exit_code, 0, "tsx must parse clean: {}", out.stderr);
    }

    #[cfg(feature = "js-oxc")]
    #[test]
    fn jscheck_ts_flag_with_stdin() {
        let s = shell();
        let out = s.cmd_jscheck(&["--ts"], Some("const n: number = 1;"));
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("syntax OK"));
    }

    #[test]
    fn render_without_plugin_returns_not_supported() {
        let s = shell();
        let out = s.cmd_render(&["-o", "/out.png"], Some("<html></html>"));
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("not supported"), "stderr={}", out.stderr);
    }
}
