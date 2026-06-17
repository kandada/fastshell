pub mod ffi;
pub mod types;

use crate::bridge::Runtime;
use crate::python::{self, PythonEngine};
use crate::python::cpython;
use crate::shell::Shell;
use crate::vfs::Vfs;
use types::*;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use std::time::Duration;
use std::ffi::{c_char, CStr, CString};

pub struct Fastshell {
    runtime: Arc<Mutex<Runtime>>,
    config: Config,
    initialized: bool,
    env_vars: std::collections::HashMap<String, String>,
    permissions: Arc<Mutex<HashMap<String, bool>>>,
    cancel_flag: Arc<AtomicBool>,
}

impl Fastshell {
    pub fn new() -> Self {
        Fastshell {
            runtime: Arc::new(Mutex::new(Runtime::new(
                Shell::new(Vfs::new(std::env::temp_dir().join("fastshell")).unwrap_or_else(|_| {
                    panic!("Failed to create default VFS")
                })),
                None,
            ))),
            config: Config::default(),
            initialized: false,
            env_vars: std::collections::HashMap::new(),
            permissions: Arc::new(Mutex::new(HashMap::new())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn init(&mut self, config: Config) -> Result<(), String> {
        if config.sandbox_path.is_empty() {
            return Err("sandbox_path is required".to_string());
        }
        let sandbox_path = std::path::PathBuf::from(&config.sandbox_path);
        let vfs = Vfs::new(sandbox_path.clone()).map_err(|e| format!("Failed to initialize VFS: {}", e))?;

        let permissions = Arc::new(Mutex::new(HashMap::new()));
        let shell = Shell::new_with_config(vfs, config.allow_subprocess, config.network_ask_permission, permissions.clone());

        let python: Option<Box<dyn PythonEngine>> = if config.python_enabled {
            Some(python::detect_python_engine(&sandbox_path))
        } else {
            None
        };

        self.runtime = Arc::new(Mutex::new(Runtime::new(shell, python)));
        self.config = config;
        self.initialized = true;
        self.env_vars.clear();
        self.permissions = permissions;

        cpython::register_shell_execute(fastshell_shell_exec_c);
        cpython::register_shell_free(fastshell_shell_free_c);

        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn execute(&self, command: &str) -> CommandResult {
        if !self.initialized {
            return CommandResult::error("SDK not initialized. Call init() first.".to_string());
        }

        let timeout_ms = self.config.command_timeout_ms;

        if timeout_ms == 0 {
            let mut rt = self.runtime.lock().unwrap();
            let output = rt.execute(command);
            return CommandResult::from_code(output.stdout, output.stderr, output.exit_code);
        }

        self.cancel_flag.store(false, Ordering::SeqCst);
        let rt = self.runtime.clone();
        let cancel = self.cancel_flag.clone();
        let cmd = command.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if cancel.load(Ordering::SeqCst) {
                let _ = tx.send(crate::shell::CommandOutput::error("cancelled".to_string(), 143));
                return;
            }
            let mut runtime = rt.lock().unwrap();
            if cancel.load(Ordering::SeqCst) {
                drop(runtime);
                let _ = tx.send(crate::shell::CommandOutput::error("cancelled".to_string(), 143));
                return;
            }
            let output = runtime.execute(&cmd);
            let _ = tx.send(output);
        });

        match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(output) => CommandResult::from_code(output.stdout, output.stderr, output.exit_code),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                self.cancel_flag.store(true, Ordering::SeqCst);
                CommandResult {
                    stdout: String::new(),
                    stderr: "command timed out\n".to_string(),
                    exit_code: 124,
                }
            }
            Err(_) => CommandResult::error("internal error".to_string()),
        }
    }

    pub fn cancel_execution(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    pub fn execute_python(&self, code: &str) -> CommandResult {
        if !self.initialized {
            return CommandResult::error("SDK not initialized. Call init() first.".to_string());
        }
        let mut rt = self.runtime.lock().unwrap();
        let output = rt.execute_python_code(code);
        CommandResult::from_code(output.stdout, output.stderr, output.exit_code)
    }

    pub fn execute_python_script(&self, script_path: &str) -> CommandResult {
        if !self.initialized {
            return CommandResult::error("SDK not initialized. Call init() first.".to_string());
        }
        let mut rt = self.runtime.lock().unwrap();
        let output = rt.execute_python_script(script_path);
        CommandResult::from_code(output.stdout, output.stderr, output.exit_code)
    }

    pub fn get_cwd(&self) -> String {
        if !self.initialized {
            return "/".to_string();
        }
        let rt = self.runtime.lock().unwrap();
        rt.cwd().to_string()
    }

    pub fn read_file(&self, path: &str) -> Result<String, String> {
        if !self.initialized {
            return Err("SDK not initialized".to_string());
        }
        let rt = self.runtime.lock().unwrap();
        rt.read_file(path)
    }

    pub fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        if !self.initialized {
            return Err("SDK not initialized".to_string());
        }
        let rt = self.runtime.lock().unwrap();
        rt.write_file(path, content)
    }

    pub fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, String> {
        if !self.initialized {
            return Err("SDK not initialized".to_string());
        }
        let rt = self.runtime.lock().unwrap();
        rt.list_dir(path)
    }

    pub fn exists(&self, path: &str) -> bool {
        if !self.initialized { return false; }
        let rt = self.runtime.lock().unwrap();
        rt.exists(path)
    }

    pub fn is_dir(&self, path: &str) -> bool {
        if !self.initialized { return false; }
        let rt = self.runtime.lock().unwrap();
        rt.is_dir(path)
    }

    pub fn set_env(&mut self, key: &str, value: &str) {
        self.env_vars.insert(key.to_string(), value.to_string());
        std::env::set_var(key, value);
    }

    pub fn get_env(&self, key: &str) -> Option<String> {
        self.env_vars.get(key).cloned()
    }

    pub fn get_info(&self) -> SdkInfo {
        let python_available = if self.initialized {
            let rt = self.runtime.lock().unwrap();
            rt.python_available()
        } else {
            false
        };

        SdkInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
            python_available,
            sandbox_path: self.config.sandbox_path.clone(),
            allow_subprocess: self.config.allow_subprocess,
        }
    }

    pub fn set_permission(&self, resource: &str, allowed: bool) {
        if let Ok(mut perms) = self.permissions.lock() {
            perms.insert(resource.to_string(), allowed);
        }
    }

    pub fn check_permission(&self, resource: &str) -> Option<bool> {
        self.permissions.lock().ok().and_then(|perms| perms.get(resource).copied())
    }

    pub fn clear_permissions(&self) {
        if let Ok(mut perms) = self.permissions.lock() {
            perms.clear();
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn vfs_root(&self) -> String {
        let rt = self.runtime.lock().unwrap();
        rt.shell_root_dir().to_string_lossy().to_string()
    }

    pub fn shutdown(&mut self) {
        self.initialized = false;
        self.env_vars.clear();
        let rt = self.runtime.lock().unwrap();
        let root = rt.shell_root_dir();
        let _ = std::fs::remove_dir_all(&root);
    }

    pub fn runtime_ref(&self) -> Arc<Mutex<Runtime>> {
        self.runtime.clone()
    }
}

unsafe extern "C" fn fastshell_shell_exec_c(cmd: *const c_char) -> *const c_char {
    let cmd_str = if cmd.is_null() { String::new() } else { unsafe { CStr::from_ptr(cmd) }.to_string_lossy().to_string() };
    let sdk = crate::sdk::ffi::get_sdk_internal();
    let sdk = sdk.lock().unwrap();
    let result = sdk.execute(&cmd_str);
    let json = serde_json::json!({
        "stdout": result.stdout,
        "stderr": result.stderr,
        "returncode": result.exit_code,
    });
    CString::new(json.to_string()).unwrap().into_raw()
}

unsafe extern "C" fn fastshell_shell_free_c(ptr: *mut c_char) {
    if !ptr.is_null() { unsafe { let _ = CString::from_raw(ptr); } }
}

impl Default for Fastshell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_sdk() -> Fastshell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_sdk_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);

        let mut sdk = Fastshell::new();
        let config = Config {
            sandbox_path: dir.to_string_lossy().to_string(),
            python_enabled: true,
            allow_subprocess: true,
            network_ask_permission: false,
            ..Default::default()
        };
        sdk.init(config).unwrap();
        sdk
    }

    #[test]
    fn test_init() {
        let sdk = setup_sdk();
        assert!(sdk.is_initialized());
    }

    #[test]
    fn test_execute_not_initialized() {
        let sdk = Fastshell::new();
        let result = sdk.execute("ls");
        assert_ne!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_shell() {
        let sdk = setup_sdk();
        let result = sdk.execute("echo hello_fastshell");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello_fastshell"));
    }

    #[test]
    fn test_execute_ls() {
        let sdk = setup_sdk();
        let result = sdk.execute("ls -la");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_mkdir_cd_pwd() {
        let sdk = setup_sdk();
        sdk.execute("mkdir testdir");
        sdk.execute("cd testdir");
        let result = sdk.execute("pwd");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("testdir"));
        assert!(sdk.get_cwd().contains("testdir"));
    }

    #[test]
    fn test_execute_file_operations() {
        let sdk = setup_sdk();
        sdk.execute("touch hello.txt");
        let result = sdk.execute("ls");
        assert!(result.stdout.contains("hello.txt"));
    }

    #[test]
    fn test_direct_file_api() {
        let sdk = setup_sdk();
        sdk.write_file("test.txt", "hello direct").unwrap();
        assert_eq!(sdk.read_file("test.txt").unwrap(), "hello direct");
        assert!(sdk.exists("test.txt"));
        assert!(!sdk.exists("nope.txt"));

        sdk.execute("mkdir subdir");
        let entries = sdk.list_dir("subdir").unwrap();
        assert!(entries.is_empty());

        sdk.write_file("subdir/a.txt", "a").unwrap();
        let entries = sdk.list_dir("subdir").unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_env_api() {
        let mut sdk = setup_sdk();
        sdk.set_env("MY_VAR", "my_value");
        assert_eq!(sdk.get_env("MY_VAR"), Some("my_value".to_string()));
    }

    #[test]
    fn test_execute_python_direct() {
        let sdk = setup_sdk();
        let result = sdk.execute_python("print(100 + 23)");
        {
            let rt = sdk.runtime_ref();
            let rt = rt.lock().unwrap();
            if rt.python_available() {
                assert_eq!(result.exit_code, 0);
                assert!(result.stdout.contains("123"));
            }
        }
    }

    #[test]
    fn test_get_info() {
        let sdk = setup_sdk();
        let info = sdk.get_info();
        assert_eq!(info.version, "0.2.0");
        assert!(!info.platform.is_empty());
    }

    #[test]
    fn test_shutdown() {
        let mut sdk = setup_sdk();
        let root = sdk.vfs_root();
        assert!(std::path::Path::new(&root).exists());
        sdk.shutdown();
        assert!(!sdk.is_initialized());
    }

    #[test]
    fn test_init_empty_path() {
        let mut sdk = Fastshell::new();
        let config = Config { sandbox_path: String::new(), ..Default::default() };
        assert!(sdk.init(config).is_err());
    }

    #[test]
    fn test_subprocess_disabled_by_default_on_mobile() {
        let default_config = Config::default();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        assert!(!default_config.allow_subprocess);
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        assert!(default_config.allow_subprocess);
    }

    #[test]
    fn test_network_ask_permission_default_on_mobile() {
        let default_config = Config::default();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        assert!(default_config.network_ask_permission);
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        assert!(!default_config.network_ask_permission);
    }

    #[test]
    fn test_permission_management() {
        let sdk = setup_sdk();
        assert_eq!(sdk.check_permission("network:example.com"), None);
        sdk.set_permission("network:example.com", true);
        assert_eq!(sdk.check_permission("network:example.com"), Some(true));
        sdk.set_permission("network:example.com", false);
        assert_eq!(sdk.check_permission("network:example.com"), Some(false));
        sdk.clear_permissions();
        assert_eq!(sdk.check_permission("network:example.com"), None);
    }

    #[test]
    fn test_network_permission_denied() {
        let mut sdk = Fastshell::new();
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_sdk_perm_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let config = Config {
            sandbox_path: dir.to_string_lossy().to_string(),
            python_enabled: false,
            allow_subprocess: false,
            network_ask_permission: true,
            command_timeout_ms: 5_000,
        };
        sdk.init(config).unwrap();

        let result = sdk.execute("curl http://example.com");
        assert_eq!(result.exit_code, EXIT_NEED_PERMISSION);
        assert!(result.stderr.contains("PERMISSION_NEEDED:network:example.com"));

        sdk.set_permission("network:example.com", true);
        let result = sdk.execute("curl http://example.com");
        assert_ne!(result.exit_code, EXIT_NEED_PERMISSION);
    }

    #[test]
    fn test_command_not_found_subprocess_disabled() {
        let mut sdk = Fastshell::new();
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_sdk_nosub_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let config = Config {
            sandbox_path: dir.to_string_lossy().to_string(),
            python_enabled: false,
            allow_subprocess: false,
            network_ask_permission: false,
            command_timeout_ms: 5_000,
        };
        sdk.init(config).unwrap();

        let result = sdk.execute("nonexistent_xyz_123");
        assert_eq!(result.exit_code, 127);
        assert!(result.stderr.contains("subprocess disabled"));
    }

    #[test]
    fn test_sdk_info_includes_allow_subprocess() {
        let sdk = setup_sdk();
        let info = sdk.get_info();
        assert_eq!(info.version, "0.2.0");
        assert!(info.allow_subprocess);
    }
}
