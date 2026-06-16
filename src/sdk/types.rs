#[derive(Debug, Clone)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl CommandResult {
    pub fn success(stdout: String) -> Self {
        CommandResult { stdout, stderr: String::new(), exit_code: 0 }
    }

    pub fn error(message: String) -> Self {
        CommandResult { stdout: String::new(), stderr: message, exit_code: 1 }
    }

    pub fn from_code(stdout: String, stderr: String, exit_code: i32) -> Self {
        CommandResult { stdout, stderr, exit_code }
    }

    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub sandbox_path: String,
    pub python_enabled: bool,
    pub command_timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config { sandbox_path: String::new(), python_enabled: true, command_timeout_ms: 30_000 }
    }
}

#[derive(Debug, Clone)]
pub struct SdkInfo {
    pub version: String,
    pub platform: String,
    pub python_available: bool,
    pub sandbox_path: String,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}
