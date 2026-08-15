// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_truncate(&self, args: &[&str]) -> CommandOutput {
        let mut size: Option<u64> = None;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-s" | "--size" => {
                    if i + 1 < args.len() {
                        size = parse_truncate_size(args[i + 1]);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-s") && arg.len() > 2 => {
                    size = parse_truncate_size(&arg[2..]);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        if files.is_empty() {
            return CommandOutput::error("truncate: missing file operand\n".to_string(), 1);
        }

        let target_size = size.unwrap_or(0);

        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("truncate: {}: {}\n", file, e), 1),
            };

            let f = match std::fs::OpenOptions::new().write(true).open(&resolved) {
                Ok(f) => f,
                Err(e) => return CommandOutput::error(format!("truncate: {}: {}\n", file, e), 1),
            };

            if let Err(e) = f.set_len(target_size) {
                return CommandOutput::error(format!("truncate: {}: {}\n", file, e), 1);
            }
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_cmp(&self, args: &[&str]) -> CommandOutput {
        let mut silent = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-s" | "--silent" | "--quiet" => silent = true,
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        if files.len() < 2 {
            return CommandOutput::error("cmp: missing file operand\n".to_string(), 1);
        }

        let data1 = match self.vfs.read(&files[0], &self.cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("cmp: {}: {}\n", files[0], e), 1),
        };
        let data2 = match self.vfs.read(&files[1], &self.cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("cmp: {}: {}\n", files[1], e), 1),
        };

        let len = data1.len().min(data2.len());
        for k in 0..len {
            if data1[k] != data2[k] {
                let line = (k / 16) + 1;
                let byte = (k % 16) + 1;
                if !silent {
                    return CommandOutput {
                        stdout: format!(
                            "{} {} differ: byte {}, line {}\n",
                            files[0], files[1], byte, line
                        ),
                        stderr: String::new(),
                        exit_code: 1,
                    };
                }
                return CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 1,
                };
            }
        }

        if data1.len() != data2.len() {
            let shorter = if data1.len() < data2.len() {
                &files[0]
            } else {
                &files[1]
            };
            if !silent {
                return CommandOutput {
                    stdout: format!("cmp: EOF on {} after byte {}\n", shorter, len + 1),
                    stderr: String::new(),
                    exit_code: 1,
                };
            }
            return CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 1,
            };
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_strings(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut min_len = 4usize;
        let mut print_filename = false;
        let mut radix: Option<char> = None;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-h" | "--help" => {
                    return CommandOutput::success(STRINGS_HELP_TEXT.to_string());
                }
                "-n" => {
                    if i + 1 < args.len() {
                        min_len = args[i + 1].parse().unwrap_or(4);
                        i += 1;
                    }
                }
                "-f" | "--print-file-name" => print_filename = true,
                "-t" => {
                    if i + 1 < args.len() {
                        radix = args[i + 1].chars().next();
                        i += 1;
                    }
                }
                "--radix" => {
                    if i + 1 < args.len() {
                        radix = Some(match args[i + 1] {
                            "o" | "octal" => 'o',
                            "d" | "decimal" => 'd',
                            "x" | "hex" => 'x',
                            _ => 'd',
                        });
                        i += 1;
                    }
                }
                arg if arg.starts_with("-n") && arg.len() > 2 => {
                    min_len = arg[2..].parse().unwrap_or(4);
                }
                arg if arg.starts_with("-t") && arg.len() > 2 => {
                    radix = arg[2..].chars().next();
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        if files.is_empty() {
            match stdin {
                Some(s) => {
                    let output = extract_strings(s.as_bytes(), min_len, None, radix);
                    return CommandOutput::success(output);
                }
                None => {
                    return CommandOutput::error("strings: missing input\n".to_string(), 1);
                }
            }
        }

        let mut output = String::new();
        for file in &files {
            let data = match self.vfs.read(file, &self.cwd) {
                Ok(d) => d,
                Err(e) => {
                    return CommandOutput::error(format!("strings: {}: {}\n", file, e), 1)
                }
            };
            if print_filename && files.len() > 1 {
                output.push_str(&format!("\n{}:\n", file));
            }
            let label = if print_filename { Some(file.as_str()) } else { None };
            output.push_str(&extract_strings(&data, min_len, label, radix));
        }

        CommandOutput::success(output)
    }

    pub fn cmd_fold(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut width = 80usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-w" | "--width" => {
                    if i + 1 < args.len() {
                        width = args[i + 1].parse().unwrap_or(80);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-w") && arg.len() > 2 => {
                    width = arg[2..].parse().unwrap_or(80);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let content = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("fold: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = String::new();
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(c) => all.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("fold: {}: {}\n", file, e), 1),
                }
            }
            all
        };

        let mut output = String::new();
        for line in content.lines() {
            let mut pos = 0;
            let chars: Vec<char> = line.chars().collect();
            while pos < chars.len() {
                let end = (pos + width).min(chars.len());
                output.extend(&chars[pos..end]);
                output.push('\n');
                pos = end;
            }
        }

        CommandOutput::success(output)
    }

    pub fn cmd_expand(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut tab_size = 8usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-t" | "--tabs" => {
                    if i + 1 < args.len() {
                        tab_size = args[i + 1].parse().unwrap_or(8);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-t") && arg.len() > 2 => {
                    tab_size = arg[2..].parse().unwrap_or(8);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let content = match get_file_content(self, &files, stdin, "expand") {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut output = String::new();
        for line in content.lines() {
            let mut col = 0;
            for ch in line.chars() {
                if ch == '\t' {
                    let spaces = tab_size - (col % tab_size);
                    output.push_str(&" ".repeat(spaces));
                    col += spaces;
                } else {
                    output.push(ch);
                    col += 1;
                }
            }
            output.push('\n');
        }

        CommandOutput::success(output)
    }

    pub fn cmd_unexpand(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut tab_size = 8usize;
        let mut all = false;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-t" | "--tabs" => {
                    if i + 1 < args.len() {
                        tab_size = args[i + 1].parse().unwrap_or(8);
                        i += 1;
                    }
                }
                "-a" | "--all" => all = true,
                arg if arg.starts_with("-t") && arg.len() > 2 => {
                    tab_size = arg[2..].parse().unwrap_or(8);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let content = match get_file_content(self, &files, stdin, "unexpand") {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut output = String::new();
        for line in content.lines() {
            let chars: Vec<char> = line.chars().collect();
            let mut result = String::new();
            let mut col = 0;
            let mut space_run = 0;

            for &ch in &chars {
                if ch == ' ' {
                    space_run += 1;
                } else {
                    result.push_str(&" ".repeat(space_run));
                    result.push(ch);
                    col += space_run + 1;
                    space_run = 0;
                }

                if space_run > 0 && ((col + space_run) % tab_size == 0) {
                    if all || space_run >= 2 {
                        result.push('\t');
                        col += space_run;
                        space_run = 0;
                    }
                }
            }
            result.push_str(&" ".repeat(space_run));
            result.push('\n');
            output.push_str(&result);
        }

        CommandOutput::success(output)
    }

    pub fn cmd_yes(&self, args: &[&str]) -> CommandOutput {
        let msg = if args.is_empty() {
            "y".to_string()
        } else {
            args.join(" ")
        };

        let mut output = String::new();
        for _ in 0..10000 {
            output.push_str(&msg);
            output.push('\n');
        }

        CommandOutput::success(output)
    }
}

const STRINGS_HELP_TEXT: &str = "\
Usage: strings [OPTION]... [FILE]...
Print the sequences of printable characters in files.

  -n N         minimum string length (default 4)
  -f           print the name of the file before each string
  -t {o,d,x}   print the offset (octal, decimal, hex) before each string
      --radix={o,d,x}  same as -t
  -h, --help     display this help and exit
";

fn extract_strings(data: &[u8], min_len: usize, label: Option<&str>, radix: Option<char>) -> String {
    let mut output = String::new();
    let mut current = String::new();
    let mut start_offset: usize = 0;
    let mut in_string = false;

    for (i, &byte) in data.iter().enumerate() {
        if byte >= 0x20 && byte < 0x7f {
            if !in_string {
                start_offset = i;
                in_string = true;
            }
            current.push(byte as char);
        } else {
            if in_string {
                if current.len() >= min_len {
                    if let Some(name) = label {
                        output.push_str(name);
                        output.push_str(": ");
                    }
                    if let Some(r) = radix {
                        match r {
                            'o' => output.push_str(&format!("{:>7o} ", start_offset)),
                            'd' => output.push_str(&format!("{:>7} ", start_offset)),
                            'x' | _ => output.push_str(&format!("{:>7x} ", start_offset)),
                        }
                    }
                    output.push_str(&current);
                    output.push('\n');
                }
                current.clear();
                in_string = false;
            }
        }
    }
    if in_string && current.len() >= min_len {
        if let Some(name) = label {
            output.push_str(name);
            output.push_str(": ");
        }
        if let Some(r) = radix {
            match r {
                'o' => output.push_str(&format!("{:>7o} ", start_offset)),
                'd' => output.push_str(&format!("{:>7} ", start_offset)),
                'x' | _ => output.push_str(&format!("{:>7x} ", start_offset)),
            }
        }
        output.push_str(&current);
        output.push('\n');
    }

    output
}

fn get_file_content(
    shell: &Shell,
    files: &[String],
    stdin: Option<&str>,
    cmd: &str,
) -> Result<String, CommandOutput> {
    if files.is_empty() {
        match stdin {
            Some(s) => Ok(s.to_string()),
            None => Err(CommandOutput::error(format!("{}: missing input\n", cmd), 1)),
        }
    } else {
        let mut all = String::new();
        for file in files {
            match shell.vfs.read_to_string(file, &shell.cwd) {
                Ok(c) => all.push_str(&c),
                Err(e) => {
                    return Err(CommandOutput::error(
                        format!("{}: {}: {}\n", cmd, file, e),
                        1,
                    ))
                }
            }
        }
        Ok(all)
    }
}

fn parse_truncate_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(_rest) = s.strip_prefix('+') {
        None // extend mode not supported
    } else if let Some(_rest) = s.strip_prefix('-') {
        None // shrink mode
    } else if let Some(rest) = s.strip_suffix('K') {
        rest.parse::<u64>().ok().map(|n| n * 1024)
    } else if let Some(rest) = s.strip_suffix('M') {
        rest.parse::<u64>().ok().map(|n| n * 1024 * 1024)
    } else if let Some(rest) = s.strip_suffix('G') {
        rest.parse::<u64>().ok().map(|n| n * 1024 * 1024 * 1024)
    } else {
        s.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fastshell_strings_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_strings_help() {
        let mut s = mk_shell();
        let out = s.execute("strings", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: strings"));
    }

    #[test]
    fn test_strings_help_long() {
        let mut s = mk_shell();
        let out = s.execute("strings", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: strings"));
    }

    #[test]
    fn test_strings_basic() {
        let mut s = mk_shell();
        let out = s.execute("strings", &["-n", "3"], Some("hello\0\x01world\0\x02test"));
        assert!(out.exit_code == 0);
        assert!(out.stdout.contains("hello"));
        assert!(out.stdout.contains("world"));
        assert!(out.stdout.contains("test"));
    }

    #[test]
    fn test_strings_offset_hex() {
        let mut s = mk_shell();
        let out = s.execute("strings", &["-t", "x", "-n", "3"], Some("ab\0\x01hello"));
        assert!(out.exit_code == 0);
        // "ab" is 2 chars, filtered; "hello" starts at offset 4
        assert!(out.stdout.contains("hello"));
    }

    #[test]
    fn test_strings_filename_flag() {
        let mut s = mk_shell();
        let file = "test_strings_f.txt";
        s.vfs.write(file, &s.cwd, "hello\0world").unwrap();
        let out = s.execute("strings", &["-f", file], None);
        assert!(out.exit_code == 0);
        assert!(out.stdout.contains("test_strings_f.txt"));
    }
}
