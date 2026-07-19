// fastshell standalone IPC server
// Communication: stdin (one JSON per line) → stdout (one JSON per line)
//
// Commands:
//   {"cmd":"init","sandbox":"/path","python":true}
//   {"cmd":"execute","command":"ls -la"}
//   {"cmd":"python","code":"print(1)"}
//   {"cmd":"python_script","path":"script.py"}
//   {"cmd":"cwd"}
//   {"cmd":"cancel"}
//   {"cmd":"shutdown"}

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::io::{BufRead, BufReader, Write};

fn main() {
    let mut sdk = Fastshell::new();
    let mut stdin = BufReader::new(std::io::stdin());
    let mut line = String::new();

    while stdin.read_line(&mut line).is_ok() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }
        let response = handle_command(&mut sdk, trimmed);
        println!("{}", response);
        std::io::stdout().flush().ok();
        line.clear();
    }
}

fn handle_command(sdk: &mut Fastshell, input: &str) -> String {
    let v: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => return err_json(&format!("invalid json: {}", e)),
    };

    let cmd = v["cmd"].as_str().unwrap_or("");

    match cmd {
        "init" => {
            let sandbox = v["sandbox"].as_str().unwrap_or("");
            if sandbox.is_empty() {
                return err_json("sandbox path required");
            }
            let python = v["python"].as_bool().unwrap_or(true);
            let config = Config {
                sandbox_path: sandbox.to_string(),
                python_enabled: python,
                python_home: format!("{}/python", sandbox),
                allow_subprocess: false,
                network_ask_permission: true,
                command_timeout_ms: 300_000,
            };
            match sdk.init(config) {
                Ok(()) => ok_json(),
                Err(e) => err_json(&e),
            }
        }
        "execute" => {
            let command = v["command"].as_str().unwrap_or("");
            let result = sdk.execute(command);
            serde_json::json!({
                "ok": true,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exit_code": result.exit_code,
            }).to_string()
        }
        "python" => {
            let code = v["code"].as_str().unwrap_or("");
            let result = sdk.execute_python(code);
            serde_json::json!({
                "ok": true,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exit_code": result.exit_code,
            }).to_string()
        }
        "python_script" => {
            let path = v["path"].as_str().unwrap_or("");
            let result = sdk.execute_python_script(path);
            serde_json::json!({
                "ok": true,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exit_code": result.exit_code,
            }).to_string()
        }
        "cwd" => {
            serde_json::json!({"ok": true, "cwd": sdk.get_cwd()}).to_string()
        }
        "cancel" => {
            sdk.cancel_execution();
            ok_json()
        }
        "shutdown" => {
            std::process::exit(0);
        }
        _ => err_json(&format!("unknown command: {}", cmd)),
    }
}

fn ok_json() -> String {
    r#"{"ok":true}"#.to_string()
}

fn err_json(msg: &str) -> String {
    serde_json::json!({"ok": false, "error": msg}).to_string()
}
