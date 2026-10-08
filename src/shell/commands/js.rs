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
            None | Some("-") => match stdin {
                Some(code) => self.run_js(code, false),
                None => CommandOutput::error("node: no input (pipe a script)\n".to_string(), 1),
            },
            Some("-h") | Some("--help") => CommandOutput::success(NODE_USAGE.to_string()),
            Some("-v") | Some("--version") => {
                CommandOutput::success("javascript (host webview engine)\n".to_string())
            }
            Some("-e") | Some("--eval") | Some("-p") | Some("--print") => {
                let code = args.get(1).copied().unwrap_or("");
                if code.is_empty() {
                    return CommandOutput::error(format!("node: {} requires code\n", args[0]), 1);
                }
                self.run_js(code, true)
            }
            Some("-c") | Some("--check") => {
                let file = args.get(1).copied().unwrap_or("");
                if file.is_empty() {
                    return CommandOutput::error("node: --check requires a file\n".to_string(), 1);
                }
                match self.read_text_lossy(file) {
                    Ok(code) => self.check_js(&code, file),
                    Err(e) => CommandOutput::error(format!("node: {file}: {e}\n"), 1),
                }
            }
            Some(file) => match self.read_text_lossy(file) {
                Ok(code) => self.run_js(&code, false),
                Err(e) => CommandOutput::error(format!("node: {file}: {e}\n"), 1),
            },
        }
    }

    /// `jscheck` / `jslint` — static syntax check via oxc (feature `js-oxc`).
    pub fn cmd_jscheck(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut hint = "stdin.js";
        let mut file: Option<&str> = None;
        let mut eval: Option<&str> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--ts" | "--typescript" => hint = "stdin.ts",
                "--tsx" => hint = "stdin.tsx",
                "--jsx" => hint = "stdin.jsx",
                "--mjs" | "--module" => hint = "stdin.mjs",
                "-e" | "--eval" => {
                    if i + 1 < args.len() {
                        eval = Some(args[i + 1]);
                        i += 1;
                    }
                }
                arg if !arg.starts_with('-') => file = Some(arg),
                _ => {}
            }
            i += 1;
        }

        let (code, filename) = if let Some(e) = eval {
            (e.to_string(), hint.to_string())
        } else if let Some(f) = file {
            match self.read_text_lossy(f) {
                Ok(c) => (c, f.to_string()),
                Err(e) => return CommandOutput::error(format!("jscheck: {f}: {e}\n"), 1),
            }
        } else if let Some(s) = stdin {
            (s.to_string(), hint.to_string())
        } else {
            return CommandOutput::error(
                "jscheck: no input (pass a file, -e CODE, or pipe source)\n".to_string(),
                1,
            );
        };

        self.check_js(&code, &filename)
    }

    /// `render` — render HTML in the host WebView and capture a screenshot.
    pub fn cmd_render(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(
                "Usage: render [-o OUT.png] [FILE]\n\
                 Render HTML (from FILE or stdin) in the offscreen WebView and save a PNG.\n\
                 NOTE: external network resources (http(s) images / styles / scripts) are NOT\n\
                 loaded by the renderer — fetch them first (e.g. `curl -o img.jpg`) and\n\
                 reference the local files.\n"
                    .to_string(),
            );
        }
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
            match self.read_text_lossy(f) {
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
        // The offscreen renderer has no network stack, so fetch and inline
        // external http(s) resources (images / stylesheets / scripts / CSS
        // url(...)) before rendering. Anything we could not fetch still warns.
        let (html, inlined) = inline_external_resources(&html);
        let warn = if inlined > 0 {
            String::new()
        } else if html.contains("src=\"http")
            || html.contains("src='http")
            || html.contains("url(http")
            || html.contains("src=&quot;http")
        {
            "render: note: some external http(s) resources could not be fetched and were left unresolved.\n"
                .to_string()
        } else {
            String::new()
        };
        match plugin(self, |p| p.render_html(&html, &host)) {
            Ok(()) => CommandOutput {
                stdout: format!("Rendered to {out_path}\n"),
                stderr: warn,
                exit_code: 0,
            },
            Err(e) => e,
        }
    }

    fn run_js(&self, code: &str, expr: bool) -> CommandOutput {
        // Always wrap in an async runner so `console.*` output is captured and
        // Promises/timers (`await`, `.then`, `setTimeout`, `setInterval`) are
        // actually driven to completion before we return. The wrapper returns a
        // JSON string `{"logs":[...],"result":...}` (a string keeps the host
        // contract identical across Android/iOS, whose JS result is a String).
        let script = async_eval_script(code, expr);
        match plugin(self, |p| p.eval_js(&script)) {
            Ok(result) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(result.trim()) {
                    let logs = v.get("logs").and_then(|l| l.as_array());
                    let res = v.get("result");
                    let mut out = String::new();
                    if let Some(logs) = logs {
                        for l in logs {
                            out.push_str(l.as_str().unwrap_or(""));
                            out.push('\n');
                        }
                    }
                    if out.is_empty() {
                        if let Some(r) = res {
                            if !r.is_null() {
                                match r {
                                    serde_json::Value::String(s) => {
                                        out.push_str(s);
                                        out.push('\n');
                                    }
                                    other => {
                                        out.push_str(
                                            &serde_json::to_string(other).unwrap_or_default(),
                                        );
                                        out.push('\n');
                                    }
                                }
                            }
                        }
                    }
                    return CommandOutput::success(out);
                }
                // Host did not return the wrapper object (e.g. an older bridge):
                // print the raw result.
                CommandOutput::success(if result.ends_with('\n') {
                    result
                } else {
                    format!("{result}\n")
                })
            }
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

/// Build the async evaluation wrapper. It:
///   * captures `console.{log,info,warn,error,debug}` into a log buffer;
///   * runs `code` (as an expression when `expr`, else as statements);
///   * awaits the returned promise and drives `setTimeout`/`setInterval`
///     callbacks (bounded to 3s) before resolving;
///   * returns `JSON.stringify({logs, result})`.
///
/// The result is an expression that the host evaluates as an async function so
/// discrete-timer / promise work is settled before the value is read.
fn async_eval_script(code: &str, expr: bool) -> String {
    let body = if expr {
        format!("__result = await (async function() {{ return ({code}); }})();")
    } else {
        format!("__result = await (async function() {{ {code}\n }})();")
    };
    format!(
        "(async function(){{\
var __logs=[];var __o={{}};\
['log','info','warn','error','debug'].forEach(function(m){{\
__o[m]=console[m];console[m]=function(){{var a=Array.prototype.slice.call(arguments).map(function(x){{\
try{{return typeof x==='string'?x:JSON.stringify(x);}}catch(e){{return String(x);}}}});\
__logs.push(a.join(' '));try{{__o[m].apply(console,arguments);}}catch(e){{}}}};}});\
var __timers=0;var __st=window.setTimeout,__si=window.setInterval,__ct=window.clearTimeout,__ci=window.clearInterval;\
window.setTimeout=function(fn,d){{var a=Array.prototype.slice.call(arguments,2);var id=__st(function(){{__timers=Math.max(0,__timers-1);try{{fn&&fn.apply(null,a);}}catch(e){{__logs.push('Uncaught '+String(e));}}}},d);__timers++;return id;}};\
window.setInterval=function(fn,d){{var a=Array.prototype.slice.call(arguments,2);var id=__si(function(){{try{{fn&&fn.apply(null,a);}}catch(e){{__logs.push('Uncaught '+String(e));}}}},d);__timers++;return id;}};\
window.clearTimeout=function(id){{__timers=Math.max(0,__timers-1);return __ct(id);}};\
window.clearInterval=function(id){{__timers=Math.max(0,__timers-1);return __ci(id);}};\
var __result;try{{{body}}}catch(e1){{__logs.push('Uncaught '+String(e1));}}\
var __start=Date.now();while(__timers>0&&Date.now()-__start<3000){{await new Promise(function(r){{__st(r,5);}});}}\
for(var __i=0;__i<3;__i++){{await new Promise(function(r){{__st(r,0);}});}}\
var __r;try{{__r=JSON.stringify({{logs:__logs,result:(__result===undefined?null:__result)}});}}\
catch(e){{__r=JSON.stringify({{logs:__logs,result:String(__result)}});}}\
return __r;}})()"
    )
}

/// Fetch a URL's raw bytes through the shared HTTP client (used by `render`).
fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    use crate::shell::commands::curl::{http_request_ex, HttpConfig};
    let cfg = HttpConfig {
        method: "GET".to_string(),
        url: url.to_string(),
        follow_redirects: true,
        ..Default::default()
    };
    http_request_ex(&cfg).ok().map(|r| r.body)
}

fn mime_for(url: &str) -> &'static str {
    let u = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if u.ends_with(".png") {
        "image/png"
    } else if u.ends_with(".jpg") || u.ends_with(".jpeg") {
        "image/jpeg"
    } else if u.ends_with(".gif") {
        "image/gif"
    } else if u.ends_with(".webp") {
        "image/webp"
    } else if u.ends_with(".svg") {
        "image/svg+xml"
    } else if u.ends_with(".bmp") {
        "image/bmp"
    } else if u.ends_with(".ico") {
        "image/x-icon"
    } else if u.ends_with(".css") {
        "text/css"
    } else if u.ends_with(".js") {
        "application/javascript"
    } else {
        "application/octet-stream"
    }
}

fn data_uri(url: &str) -> Option<String> {
    let bytes = fetch_bytes(url)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Some(format!("data:{};base64,{}", mime_for(url), b64))
}

/// Inline external `http(s)` resources (images, stylesheets, scripts, CSS
/// `url(...)`) so `render` produces a faithful screenshot without a network
/// stack inside the renderer. Returns the rewritten HTML and the count inlined.
fn inline_external_resources(html: &str) -> (String, usize) {
    use regex::Regex;
    let mut count = 0usize;
    let mut out = html.to_string();

    // <img src="http...">
    let img = Regex::new(r#"(?is)(<img\b[^>]*?\bsrc=)(["'])(https?://[^"']+)(["'])"#).unwrap();
    out = img
        .replace_all(&out, |c: &regex::Captures| match data_uri(&c[3]) {
            Some(uri) => {
                count += 1;
                format!("{}{}{}{}", &c[1], &c[2], uri, &c[4])
            }
            None => c[0].to_string(),
        })
        .into_owned();

    // <link ... rel="stylesheet" href="http...">  →  <style>…</style>
    let link = Regex::new(r#"(?is)<link\b[^>]*>"#).unwrap();
    out = link
        .replace_all(&out, |c: &regex::Captures| {
            let tag = &c[0];
            let href = Regex::new(r#"(?is)\bhref=["'](https?://[^"']+)["']"#)
                .unwrap()
                .captures(tag)
                .map(|m| m[1].to_string());
            let is_css = tag.to_ascii_lowercase().contains("stylesheet")
                || href.as_deref().map(|h| h.contains(".css")).unwrap_or(false);
            match (is_css, href) {
                (true, Some(h)) => match fetch_bytes(&h) {
                    Some(bytes) => {
                        count += 1;
                        format!("<style>{}</style>", String::from_utf8_lossy(&bytes))
                    }
                    None => tag.to_string(),
                },
                _ => tag.to_string(),
            }
        })
        .into_owned();

    // <script ... src="http..."></script>  →  <script>…</script>
    let script =
        Regex::new(r#"(?is)<script\b[^>]*?\bsrc=["'](https?://[^"']+)["'][^>]*>\s*</script>"#)
            .unwrap();
    out = script
        .replace_all(&out, |c: &regex::Captures| match fetch_bytes(&c[1]) {
            Some(bytes) => {
                count += 1;
                format!("<script>{}</script>", String::from_utf8_lossy(&bytes))
            }
            None => c[0].to_string(),
        })
        .into_owned();

    // CSS url(http...) in inline styles / <style> blocks
    let cssurl = Regex::new(r#"(?i)url\(['"]?(https?://[^)'"]+)['"]?\)"#).unwrap();
    out = cssurl
        .replace_all(&out, |c: &regex::Captures| match data_uri(&c[1]) {
            Some(uri) => {
                count += 1;
                format!("url({uri})")
            }
            None => c[0].to_string(),
        })
        .into_owned();

    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_eval_script_wraps_and_collects() {
        let js = async_eval_script("console.log('x');", false);
        assert!(js.starts_with("(async function(){"), "js={js}");
        assert!(js.contains("__logs"), "js={js}");
        assert!(js.contains("console.log('x');"), "js={js}");
        assert!(js.contains("window.setTimeout=function"), "js={js}");
        assert!(js.contains("await "), "js={js}");
        assert!(js.contains("JSON.stringify({logs:__logs"), "js={js}");
        assert!(js.trim_end().ends_with("})()"), "js={js}");

        let expr = async_eval_script("1+1", true);
        assert!(expr.contains("return (1+1);"), "expr={expr}");
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn shell() -> Shell {
        let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fs_js_{}_{}", std::process::id(), n));
        let _ = std::fs::create_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn node_without_plugin_returns_not_supported() {
        let s = shell();
        let out = s.cmd_node(&["-e", "1+1"], None);
        assert_ne!(out.exit_code, 0);
        assert!(
            out.stderr.contains("not supported"),
            "stderr={}",
            out.stderr
        );
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

        let _ = s
            .vfs
            .write("ok.tsx", "/", "const C: React.FC = () => <div/>;");
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
        assert!(
            out.stderr.contains("not supported"),
            "stderr={}",
            out.stderr
        );
    }
}
