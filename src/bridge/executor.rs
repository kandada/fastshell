use crate::python::PythonEngine;
use crate::shell::{CommandOutput, Shell};

pub struct Runtime {
    shell: Shell,
    python: Option<Box<dyn PythonEngine>>,
}

impl Runtime {
    pub fn new(shell: Shell, python: Option<Box<dyn PythonEngine>>) -> Self {
        Runtime { shell, python }
    }

    pub fn execute(&mut self, input: &str) -> CommandOutput {
        let input = input.trim();

        if input.is_empty() {
            return CommandOutput::success(String::new());
        }

        if is_python_command(input) {
            return self.execute_python_inner(input);
        }

        if input.contains('|') {
            return self.execute_pipeline(input);
        }

        let tokens = parse_command(input);
        if tokens.is_empty() {
            return CommandOutput::success(String::new());
        }
        let mut parts = self.expand_globs(tokens);
        let cmd = parts.remove(0);
        let args: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
        self.shell.execute(&cmd, &args, None)
    }

    fn expand_globs(&self, tokens: Vec<ParsedToken>) -> Vec<String> {
        let mut expanded = Vec::new();
        for (i, token) in tokens.into_iter().enumerate() {
            if i == 0 || token.quoted || !has_glob_chars(&token.value) {
                expanded.push(token.value);
                continue;
            }
            let matches = self.try_glob(&token.value);
            if matches.is_empty() {
                expanded.push(token.value);
            } else {
                expanded.extend(matches);
            }
        }
        expanded
    }

    fn execute_pipeline(&mut self, input: &str) -> CommandOutput {
        let mut stages = parse_pipeline(input);
        if stages.is_empty() {
            return CommandOutput::success(String::new());
        }
        for stage in &mut stages {
            let expanded = self.expand_globs(std::mem::take(stage));
            *stage = expanded.into_iter().map(|s| ParsedToken::new(s, false)).collect();
        }
        if stages.len() == 1 {
            let mut flat: Vec<String> = stages[0].iter().map(|t| t.value.clone()).collect();
            let cmd = flat.remove(0);
            let args: Vec<&str> = flat.iter().map(|s| s.as_str()).collect();
            return self.shell.execute(&cmd, &args, None);
        }

        let mut stdin: Option<String> = None;
        let mut stderr_all = String::new();

        let last_idx = stages.len() - 1;
        for (i, stage) in stages.iter().enumerate() {
            if stage.is_empty() {
                continue;
            }
            let cmd = &stage[0].value;
            let args: Vec<&str> = stage[1..].iter().map(|t| t.value.as_str()).collect();

            let mut result = self.shell.execute(cmd, &args, stdin.as_deref());

            if i < last_idx {
                stdin = Some(result.stdout.clone());
            }
            if !result.stderr.is_empty() {
                stderr_all.push_str(&result.stderr);
                if i < last_idx {
                    stderr_all.push('\n');
                }
            }

            if i == last_idx {
                result.stderr = stderr_all;
                return result;
            }
        }

        CommandOutput::success(String::new())
    }

    fn try_glob(&self, pattern: &str) -> Vec<String> {
        let cwd_path = if self.shell.cwd == "/" {
            self.shell_root_dir()
        } else {
            self.shell_root_dir().join(self.shell.cwd.trim_start_matches('/'))
        };
        let glob_pattern = cwd_path.join(pattern).to_string_lossy().to_string();
        match glob::glob(&glob_pattern) {
            Ok(paths) => {
                paths
                    .filter_map(|entry| entry.ok())
                    .filter_map(|p| {
                        p.strip_prefix(&cwd_path).ok().map(|r| r.to_string_lossy().to_string())
                    })
                    .collect()
            }
            Err(_) => vec![],
        }
    }

    pub fn python_available(&self) -> bool {
        self.python.as_ref().map_or(false, |p| p.is_available())
    }

    fn execute_python_inner(&mut self, input: &str) -> CommandOutput {
        let code = extract_python_code(input);
        let cwd = self.shell_root_dir();
        let python = match &mut self.python {
            Some(p) => p,
            None => {
                return CommandOutput::error("Python engine not configured".to_string(), 127);
            }
        };

        if !python.is_available() {
            return CommandOutput::error("Python is not available".to_string(), 127);
        }

        let result = python.execute(&code, &cwd);

        CommandOutput {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        }
    }

    pub fn execute_python_code(&mut self, code: &str) -> CommandOutput {
        let cwd = self.shell_root_dir();
        let python = match &mut self.python {
            Some(p) => p,
            None => {
                return CommandOutput::error("Python engine not configured".to_string(), 127);
            }
        };

        if !python.is_available() {
            return CommandOutput::error("Python is not available".to_string(), 127);
        }

        let result = python.execute(code, &cwd);

        CommandOutput {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        }
    }

    pub fn execute_python_script(&mut self, script_path: &str) -> CommandOutput {
        let full_path = self.shell_root_dir().join(script_path.trim_start_matches('/'));
        let cwd = self.shell_root_dir();
        let python = match &mut self.python {
            Some(p) => p,
            None => { return CommandOutput::error("Python engine not configured".to_string(), 127); }
        };
        if !python.is_available() { return CommandOutput::error("Python is not available".to_string(), 127); }
        let result = python.execute_script(&full_path, &cwd);

        CommandOutput {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        }
    }

    pub fn shell_root_dir(&self) -> std::path::PathBuf {
        self.shell.vfs.root().to_path_buf()
    }

    pub fn cwd(&self) -> &str {
        &self.shell.cwd
    }

    pub fn quick_execute(&mut self, command: &str) -> CommandOutput {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return CommandOutput::success(String::new());
        }
        let cmd = parts[0];
        let args = &parts[1..];
        self.shell.execute(cmd, args, None)
    }
}

fn is_python_command(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.starts_with("python3 ") || trimmed.starts_with("python ") {
        return true;
    }
    if trimmed == "python" || trimmed == "python3" {
        return true;
    }
    false
}

fn extract_python_code(input: &str) -> String {
    let trimmed = input.trim();

    if let Some(code) = trimmed.strip_prefix("python3 -c ") {
        return strip_quotes(code);
    }
    if let Some(code) = trimmed.strip_prefix("python -c ") {
        return strip_quotes(code);
    }
    if let Some(code) = trimmed.strip_prefix("python3 -c") {
        return strip_quotes(code.trim());
    }
    if let Some(code) = trimmed.strip_prefix("python -c") {
        return strip_quotes(code.trim());
    }

    trimmed.to_string()
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let first = s.chars().next().unwrap();
        let last = s.chars().last().unwrap();
        if (first == '\'' && last == '\'') || (first == '"' && last == '"') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedToken {
    value: String,
    quoted: bool,
}

impl ParsedToken {
    fn new(value: String, quoted: bool) -> Self {
        ParsedToken { value, quoted }
    }
}

fn parse_command(input: &str) -> Vec<ParsedToken> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut quoted = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            } else {
                current.push(ch);
            }
        } else if in_double_quote {
            if ch == '"' {
                in_double_quote = false;
            } else if ch == '\\' && chars.peek().map_or(false, |&n| n == '"' || n == '\\') {
                chars.next();
                current.push(chars.next().unwrap_or(ch));
            } else {
                current.push(ch);
            }
        } else if ch == '\'' {
            in_single_quote = true;
            quoted = true;
        } else if ch == '"' {
            in_double_quote = true;
            quoted = true;
        } else if ch == ' ' || ch == '\t' {
            if !current.is_empty() {
                parts.push(ParsedToken::new(current.clone(), quoted));
                current.clear();
                quoted = false;
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        parts.push(ParsedToken::new(current, quoted));
    }

    parts
}

fn parse_pipeline(input: &str) -> Vec<Vec<ParsedToken>> {
    let mut stages: Vec<Vec<ParsedToken>> = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_single_quote {
            current.push(ch);
            if ch == '\'' {
                in_single_quote = false;
            }
        } else if in_double_quote {
            current.push(ch);
            if ch == '"' {
                in_double_quote = false;
            } else if ch == '\\' && chars.peek().map_or(false, |&n| n == '"' || n == '\\') {
                current.push(chars.next().unwrap());
            }
        } else if ch == '\'' {
            current.push(ch);
            in_single_quote = true;
        } else if ch == '"' {
            current.push(ch);
            in_double_quote = true;
        } else if ch == '|' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                stages.push(parse_command(&trimmed));
            }
            current.clear();
        } else {
            current.push(ch);
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        stages.push(parse_command(&trimmed));
    }

    stages
}

fn has_glob_chars(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::python::SubprocessPython;
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_runtime() -> Runtime {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_bridge_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        let shell = Shell::new(vfs);
        let python = Box::new(SubprocessPython::new());
        Runtime::new(shell, Some(python))
    }

    #[test]
    fn test_parse_command_simple() {
        let parts = parse_command("ls -la /tmp");
        let values: Vec<String> = parts.iter().map(|t| t.value.clone()).collect();
        assert_eq!(values, vec!["ls", "-la", "/tmp"]);
    }

    #[test]
    fn test_parse_command_quotes() {
        let parts = parse_command("echo \"hello world\"");
        assert_eq!(parts[0].value, "echo");
        assert_eq!(parts[0].quoted, false);
        assert_eq!(parts[1].value, "hello world");
        assert_eq!(parts[1].quoted, true);
    }

    #[test]
    fn test_parse_command_single_quotes() {
        let parts = parse_command("echo 'foo bar'");
        assert_eq!(parts[1].value, "foo bar");
        assert_eq!(parts[1].quoted, true);
    }

    #[test]
    fn test_parse_command_empty() {
        let parts = parse_command("");
        assert!(parts.is_empty());
    }

    #[test]
    fn test_parse_command_glob_not_quoted() {
        let parts = parse_command("ls *.rs");
        assert_eq!(parts[1].value, "*.rs");
        assert_eq!(parts[1].quoted, false);
    }

    #[test]
    fn test_parse_command_glob_quoted() {
        let parts = parse_command("find . -name '*.txt'");
        let glob_token = &parts[3];
        assert_eq!(glob_token.value, "*.txt");
        assert_eq!(glob_token.quoted, true);
    }

    #[test]
    fn test_parse_pipeline() {
        let stages = parse_pipeline("ls -la | grep foo | wc -l");
        assert_eq!(stages.len(), 3);
        let s0: Vec<String> = stages[0].iter().map(|t| t.value.clone()).collect();
        let s1: Vec<String> = stages[1].iter().map(|t| t.value.clone()).collect();
        let s2: Vec<String> = stages[2].iter().map(|t| t.value.clone()).collect();
        assert_eq!(s0, vec!["ls", "-la"]);
        assert_eq!(s1, vec!["grep", "foo"]);
        assert_eq!(s2, vec!["wc", "-l"]);
    }

    #[test]
    fn test_parse_pipeline_quotes() {
        let stages = parse_pipeline("echo \"hello | world\" | cat");
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0][1].value, "hello | world");
        assert_eq!(stages[1][0].value, "cat");
    }

    #[test]
    fn test_is_python_command() {
        assert!(is_python_command("python -c 'print(1)'"));
        assert!(is_python_command("python3 -c 'print(1)'"));
        assert!(is_python_command("python script.py"));
        assert!(!is_python_command("ls -la"));
    }

    #[test]
    fn test_execute_shell_command() {
        let mut rt = setup_runtime();
        let result = rt.execute("echo hello");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello"));
    }

    #[test]
    fn test_execute_ls() {
        let mut rt = setup_runtime();
        let result = rt.execute("ls");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_glob() {
        let mut rt = setup_runtime();
        rt.execute("touch a.txt");
        rt.execute("touch b.txt");
        let result = rt.execute("echo *.txt");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("a.txt"));
        assert!(result.stdout.contains("b.txt"));
    }

    #[test]
    fn test_execute_glob_quoted() {
        let mut rt = setup_runtime();
        let result = rt.execute("echo '*.txt'");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "*.txt");
    }

    #[test]
    fn test_execute_pipeline() {
        let mut rt = setup_runtime();
        let result = rt.execute("echo hello world | wc -w");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.trim().contains("2"));
    }

    #[test]
    fn test_execute_pipeline_grep() {
        let mut rt = setup_runtime();
        let result = rt.execute("echo \"hello\nworld\nhello again\" | grep hello | wc -l");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "2");
    }

    #[test]
    fn test_execute_python() {
        let mut rt = setup_runtime();
        let result = rt.execute("python -c 'print(42)'");
        if rt.python_available() {
            assert_eq!(result.exit_code, 0);
            assert!(result.stdout.contains("42"));
        }
    }

    #[test]
    fn test_execute_python_code_direct() {
        let mut rt = setup_runtime();
        let result = rt.execute_python_code("print('direct call')");
        if rt.python_available() {
            assert_eq!(result.exit_code, 0);
            assert!(result.stdout.contains("direct call"));
        }
    }

    #[test]
    fn test_execute_empty() {
        let mut rt = setup_runtime();
        let result = rt.execute("");
        assert_eq!(result.exit_code, 0);
    }
}
