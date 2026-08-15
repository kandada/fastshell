// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! JavaScript static analysis utilities.
//!
//! Unlike the Python engine (which embeds RustPython for in-process
//! execution), fastshell does **not** embed a JS engine. JavaScript
//! execution on mobile is delegated to the host app's WebView (Android V8 /
//! iOS JavaScriptCore) through the [`DevicePlugin::eval_js`] device command.
//!
//! What fastshell provides in-process is static checking via [oxc] — a pure
//! Rust JS/TS parser — backing the `jscheck` / `node --check` commands so an
//! agent gets syntax feedback without any runtime and without any C engine.
//!
//! [oxc]: https://github.com/oxc-project/oxc
//! [`DevicePlugin::eval_js`]: crate::sdk::plugin::DevicePlugin::eval_js

/// Syntax-check JavaScript/TypeScript source using oxc.
///
/// `filename_hint` is only used to infer the dialect (`SourceType`) — pass a
/// real path (e.g. `src/app.tsx`) or a synthetic one (e.g. `stdin.ts`) to
/// select TypeScript/JSX parsing. Returns `Ok(())` when the source parses
/// cleanly, or `Err` with rendered diagnostics (message + code frame).
#[cfg(feature = "js-oxc")]
pub fn check_syntax(code: &str, filename_hint: &str) -> Result<(), String> {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(filename_hint).unwrap_or_default();
    let ret = Parser::new(&allocator, code, source_type).parse();

    if ret.diagnostics.is_empty() && !ret.panicked {
        return Ok(());
    }

    let mut out = String::new();
    for error in ret.diagnostics {
        out.push_str(&error.render_with_source_code(code.to_string()));
        out.push('\n');
    }
    if ret.panicked {
        out.push_str("jscheck: parser panicked (unrecoverable syntax error)\n");
    }
    Err(out)
}

#[cfg(not(feature = "js-oxc"))]
pub fn check_syntax(_code: &str, _filename_hint: &str) -> Result<(), String> {
    Err("jscheck: not compiled in (build with the `js-oxc` feature)\n".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "js-oxc")]
    #[test]
    fn valid_js_parses_clean() {
        assert!(check_syntax("const x = 1; console.log(x);", "a.js").is_ok());
    }

    #[cfg(feature = "js-oxc")]
    #[test]
    fn invalid_js_reports_error() {
        let err = check_syntax("const = 1;", "a.js").unwrap_err();
        assert!(!err.is_empty());
    }

    #[cfg(feature = "js-oxc")]
    #[test]
    fn typescript_tsx_is_supported() {
        let src = "const C: React.FC = () => <div>{x}</div>;";
        assert!(check_syntax(src, "c.tsx").is_ok());
    }

    #[cfg(feature = "js-oxc")]
    #[test]
    fn stdin_hint_selects_dialect() {
        // `stdin.ts` makes oxc parse TypeScript type syntax.
        assert!(check_syntax("const n: number = 1;", "stdin.ts").is_ok());
        // Same code as plain `.js` is a syntax error (type annotation).
        assert!(check_syntax("const n: number = 1;", "stdin.js").is_err());
    }
}
