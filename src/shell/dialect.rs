// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Command "dialect" compatibility.
//!
//! Accepts common alternative command names (DOS / other-shell variants) and
//! harmless flag spellings (GNU long options, colour flags) so scripts written
//! for other environments run unchanged. Normalization is intentionally
//! conservative — it never changes a command's semantics, only the surface
//! spelling.

/// Normalize a command + args: canonical command name and rewritten args.
pub fn normalize(command: &str, args: &[&str]) -> (String, Vec<String>) {
    let mut out: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    // ── command-name dialects ──
    let cmd = match command {
        "dir" => "ls",
        "copy" => "cp",
        "move" | "ren" | "rename" => "mv",
        "del" | "erase" => "rm",
        "md" => "mkdir",
        "rd" => "rmdir",
        "cls" => "clear",
        "where" => "which",
        "gawk" | "mawk" | "nawk" => "awk",
        "less" | "more" | "view" | "most" => "cat",
        "findstr" => "grep",
        "fc" | "comp" => "diff",
        "xcopy" => "cp",
        "pcre2grep" | "pcregrep" => "grep",
        // common shell aliases
        "ll" => {
            out.insert(0, "-l".to_string());
            "ls"
        }
        "la" => {
            out.insert(0, "-la".to_string());
            "ls"
        }
        "whereis" => "which",
        // python toolchain alias (editors often invoke `pip3`)
        "pip3" => "pip",
        // checksum shorthands (busybox/BSD style)
        "md5" => "md5sum",
        "sha1" => "sha1sum",
        "sha256" => "sha256sum",
        "sha512" => "sha512sum",
        // compression aliases
        "uncompress" => "gunzip",
        "lzcat" | "xzcat" | "unlzma" | "unlzcat" => "unxz",
        "bzcat" => "bunzip2",
        // grep family
        "zgrep" | "bzgrep" | "xzgrep" => "grep",
        "egrep" => {
            // egrep is `grep -E` (currently mapped to plain grep).
            out.insert(0, "-E".to_string());
            "grep"
        }
        other => other,
    };

    // ── GNU long options → short form ──
    // Only for commands whose short form fastshell actually understands. Keeps
    // scripts written against GNU coreutils/grep working unchanged.
    let table = long_options(cmd);
    if !table.is_empty() {
        let mut rewritten: Vec<String> = Vec::with_capacity(out.len());
        for a in out {
            let mut hit = false;
            for (long, short) in table {
                if a == *long {
                    rewritten.push((*short).to_string());
                    hit = true;
                    break;
                }
                if let Some(rest) = a.strip_prefix(&format!("{long}=")) {
                    rewritten.push((*short).to_string());
                    rewritten.push(rest.to_string());
                    hit = true;
                    break;
                }
            }
            if !hit {
                rewritten.push(a);
            }
        }
        out = rewritten;
    }

    // `ls --sort=time|size|extension|name` → the matching short flag.
    if cmd == "ls" {
        out = out
            .into_iter()
            .filter_map(|a| match a.as_str() {
                "--sort=time" => Some("-t".to_string()),
                "--sort=size" => Some("-S".to_string()),
                "--sort=extension" => Some("-X".to_string()),
                "--sort=none" | "--sort=name" => None,
                _ => Some(a),
            })
            .collect();
    }

    // ── flag dialects ──
    // We never colourise, so drop harmless colour options instead of erroring.
    if matches!(cmd, "ls" | "grep" | "diff" | "rg") {
        out.retain(|a| !is_color_flag(a));
    }
    // `head/tail --lines=N` / `--bytes=N` → `-n N` / `-c N`.
    if matches!(cmd, "head" | "tail") {
        out = out.into_iter().flat_map(normalize_lines_bytes).collect();
    }
    // BSD `stat -f <fmt>` means "format" (GNU `-c`); GNU `-f` is "file-system".
    // Only rewrite when the next arg looks like a format string (contains `%`).
    if cmd == "stat" {
        let mut i = 0;
        while i + 1 < out.len() {
            if out[i] == "-f" && out[i + 1].contains('%') {
                out[i] = "-c".to_string();
            }
            i += 1;
        }
    }
    // BSD `sed -i ''` supplies an (empty) backup suffix as a separate arg.
    if cmd == "sed" {
        let mut i = 0;
        while i + 1 < out.len() {
            if out[i] == "-i" && out[i + 1].is_empty() {
                out.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    (cmd.to_string(), out)
}

/// GNU long options → short form, per command. Only options whose short form
/// fastshell actually understands are listed (never drop unknown options —
/// that would hide real errors).
fn long_options(cmd: &str) -> &'static [(&'static str, &'static str)] {
    match cmd {
        "cp" => &[
            ("--recursive", "-r"),
            ("--force", "-f"),
            ("--preserve", "-p"),
            ("--verbose", "-v"),
            ("--no-clobber", "-n"),
        ],
        "mv" => &[
            ("--force", "-f"),
            ("--verbose", "-v"),
            ("--no-clobber", "-n"),
        ],
        "rm" => &[
            ("--recursive", "-r"),
            ("--force", "-f"),
            ("--verbose", "-v"),
            ("--dir", "-d"),
        ],
        "mkdir" => &[("--parents", "-p"), ("--verbose", "-v")],
        "uniq" => &[
            ("--count", "-c"),
            ("--repeated", "-d"),
            ("--unique", "-u"),
            ("--ignore-case", "-i"),
        ],
        "wc" => &[
            ("--lines", "-l"),
            ("--words", "-w"),
            ("--bytes", "-c"),
            ("--chars", "-m"),
        ],
        "cut" => &[
            ("--delimiter", "-d"),
            ("--fields", "-f"),
            ("--characters", "-c"),
        ],
        "chmod" => &[("--recursive", "-R"), ("--verbose", "-v")],
        "du" | "df" => &[("--human-readable", "-h"), ("--all", "-a")],
        "tail" => &[("--follow", "-f")],
        "sort" => &[
            ("--reverse", "-r"),
            ("--numeric-sort", "-n"),
            ("--unique", "-u"),
            ("--general-numeric-sort", "-g"),
            ("--version-sort", "-V"),
            ("--key", "-k"),
            ("--field-separator", "-t"),
            ("--stable", "-s"),
            ("--ignore-case", "-f"),
            ("--dictionary-order", "-d"),
        ],
        "tr" => &[
            ("--delete", "-d"),
            ("--squeeze-repeats", "-s"),
            ("--complement", "-c"),
        ],
        "stat" => &[("--format", "-c"), ("--file-system", "-f")],
        "date" => &[("--date", "-d"), ("--utc", "-u"), ("--reference", "-r")],
        "grep" => &[
            ("--ignore-case", "-i"),
            ("--recursive", "-r"),
            ("--line-number", "-n"),
            ("--invert-match", "-v"),
            ("--count", "-c"),
            ("--files-with-matches", "-l"),
            ("--word-regexp", "-w"),
            ("--extended-regexp", "-E"),
            ("--fixed-strings", "-F"),
            ("--only-matching", "-o"),
            ("--quiet", "-q"),
            ("--silent", "-q"),
            ("--line-regexp", "-x"),
        ],
        "xargs" => &[
            ("--max-args", "-n"),
            ("--max-lines", "-L"),
            ("--replace", "-I"),
            ("--no-run-if-empty", "-r"),
        ],
        "ln" => &[("--symbolic", "-s"), ("--force", "-f")],
        "ls" => &[
            ("--all", "-a"),
            ("--almost-all", "-A"),
            ("--long", "-l"),
            ("--human-readable", "-h"),
            ("--recursive", "-R"),
            ("--reverse", "-r"),
            ("--directory", "-d"),
            ("--inode", "-i"),
            ("--numeric-uid-gid", "-n"),
            ("--classify", "-F"),
        ],
        "sed" => &[
            ("--in-place", "-i"),
            ("--regexp-extended", "-E"),
            ("--extended-regexp", "-E"),
            ("--quiet", "-n"),
            ("--silent", "-n"),
            ("--expression", "-e"),
        ],
        "awk" => &[("--field-separator", "-F"), ("--file", "-f")],
        "cat" => &[
            ("--number", "-n"),
            ("--squeeze-blank", "-s"),
            ("--show-ends", "-E"),
        ],
        "diff" => &[
            ("--unified", "-u"),
            ("--recursive", "-r"),
            ("--brief", "-q"),
            ("--ignore-case", "-i"),
        ],
        _ => &[],
    }
}

fn is_color_flag(a: &str) -> bool {
    matches!(a, "--color" | "--colour" | "--no-color" | "--no-colour")
        || a.starts_with("--color=")
        || a.starts_with("--colour=")
}

fn normalize_lines_bytes(a: String) -> Vec<String> {
    if let Some(v) = a.strip_prefix("--lines=") {
        return vec!["-n".to_string(), v.to_string()];
    }
    if let Some(v) = a.strip_prefix("--bytes=") {
        return vec!["-c".to_string(), v.to_string()];
    }
    if a == "--lines" {
        return vec!["-n".to_string()];
    }
    if a == "--bytes" {
        return vec!["-c".to_string()];
    }
    vec![a]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_aliases() {
        assert_eq!(normalize("dir", &[]).0, "ls");
        assert_eq!(normalize("copy", &["a", "b"]).0, "cp");
        assert_eq!(normalize("move", &["a", "b"]).0, "mv");
        assert_eq!(normalize("del", &["x"]).0, "rm");
        assert_eq!(normalize("md", &["d"]).0, "mkdir");
        assert_eq!(normalize("rd", &["d"]).0, "rmdir");
        assert_eq!(normalize("cls", &[]).0, "clear");
        assert_eq!(normalize("where", &["ls"]).0, "which");
        // Unknown commands pass through untouched.
        assert_eq!(normalize("ls", &["-la"]).0, "ls");
    }

    #[test]
    fn colour_flags_dropped() {
        let (c, a) = normalize("ls", &["--color=auto", "-la"]);
        assert_eq!(c, "ls");
        assert_eq!(a, vec!["-la"]);
        let (_, a) = normalize("grep", &["--colour=never", "x", "f"]);
        assert_eq!(a, vec!["x", "f"]);
        // Colour flags on unrelated commands are kept (no surprise mutation).
        let (_, a) = normalize("echo", &["--color=auto"]);
        assert_eq!(a, vec!["--color=auto"]);
    }

    #[test]
    fn more_command_aliases() {
        assert_eq!(normalize("gawk", &["{print}"]).0, "awk");
        assert_eq!(normalize("mawk", &[]).0, "awk");
        assert_eq!(normalize("less", &["f"]).0, "cat");
        assert_eq!(normalize("more", &["f"]).0, "cat");
        // egrep → grep -E
        let (c, a) = normalize("egrep", &["foo", "f"]);
        assert_eq!(c, "grep");
        assert_eq!(a, vec!["-E", "foo", "f"]);
        // ll → ls -l, la → ls -la, whereis → which
        let (c, a) = normalize("ll", &[]);
        assert_eq!(c, "ls");
        assert_eq!(a, vec!["-l"]);
        let (c, a) = normalize("ll", &["-a"]);
        assert_eq!(a, vec!["-l", "-a"]);
        let (c, a) = normalize("la", &[]);
        assert_eq!(c, "ls");
        assert_eq!(a, vec!["-la"]);
        assert_eq!(normalize("whereis", &["ls"]).0, "which");
    }

    #[test]
    fn bsd_stat_and_sed() {
        // BSD `stat -f %N` → `-c %N` (format), but GNU `-f` (file-system) kept.
        let (_, a) = normalize("stat", &["-f", "%N", "f"]);
        assert_eq!(a, vec!["-c", "%N", "f"]);
        let (_, a) = normalize("stat", &["-f", "f"]);
        assert_eq!(a, vec!["-f", "f"]);
        // BSD `sed -i ''` empty suffix is dropped.
        let (_, a) = normalize("sed", &["-i", "", "-e", "s/x/y/"]);
        assert_eq!(a, vec!["-i", "-e", "s/x/y/"]);
    }

    #[test]
    fn more_command_aliases_v2() {
        assert_eq!(normalize("findstr", &["x"]).0, "grep");
        assert_eq!(normalize("view", &["f"]).0, "cat");
        assert_eq!(normalize("most", &["f"]).0, "cat");
        assert_eq!(normalize("fc", &["a", "b"]).0, "diff");
        assert_eq!(normalize("xcopy", &["a", "b"]).0, "cp");
        assert_eq!(normalize("pcre2grep", &["x", "f"]).0, "grep");
    }

    #[test]
    fn gnu_long_options_to_short() {
        let (_, a) = normalize("ls", &["--all", "--long", "d"]);
        assert_eq!(a, vec!["-a", "-l", "d"]);
        let (_, a) = normalize("grep", &["--ignore-case", "--line-number", "x", "f"]);
        assert_eq!(a, vec!["-i", "-n", "x", "f"]);
        let (_, a) = normalize("rm", &["--recursive", "--force", "d"]);
        assert_eq!(a, vec!["-r", "-f", "d"]);
        let (_, a) = normalize("mkdir", &["--parents", "a/b"]);
        assert_eq!(a, vec!["-p", "a/b"]);
        // `--opt=value` → short flag + separate value
        let (_, a) = normalize("cut", &["--delimiter=,", "--fields=1", "f"]);
        assert_eq!(a, vec!["-d", ",", "-f", "1", "f"]);
        // Unknown long options are left untouched (never silently dropped).
        let (_, a) = normalize("ls", &["--totally-unknown"]);
        assert_eq!(a, vec!["--totally-unknown"]);
        // tail: --follow → -f, and --lines still handled by the other pass.
        let (_, a) = normalize("tail", &["--follow", "--lines=3", "f"]);
        assert_eq!(a, vec!["-f", "-n", "3", "f"]);
    }

    #[test]
    fn head_tail_long_options() {
        let (c, a) = normalize("head", &["--lines=2", "f"]);
        assert_eq!(c, "head");
        assert_eq!(a, vec!["-n", "2", "f"]);
        let (_, a) = normalize("tail", &["--bytes=3"]);
        assert_eq!(a, vec!["-c", "3"]);
    }
}
