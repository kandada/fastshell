use crate::sdk::types::Config;
use crate::sdk::Fastshell;
use pyo3::prelude::*;
use std::sync::Mutex;

static SDK: Mutex<Option<Fastshell>> = Mutex::new(None);

#[pyfunction]
fn init(sandbox_path: String) -> PyResult<bool> {
    let config = Config {
        sandbox_path,
        python_enabled: true,
        python_home: String::new(),
        allow_subprocess: false,
        network_ask_permission: true,
        command_timeout_ms: 300_000,
    };
    let mut sdk = Fastshell::new();
    sdk.init(config).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e))?;
    if let Ok(mut guard) = SDK.lock() {
        *guard = Some(sdk);
    }
    Ok(true)
}

#[pyfunction]
fn execute(command: String) -> PyResult<String> {
    let guard = SDK.lock().map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;
    let sdk = guard.as_ref().ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("not initialized"))?;
    let result = sdk.execute(&command);
    let json = serde_json::json!({
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exit_code": result.exit_code,
    });
    Ok(json.to_string())
}

#[pyfunction]
fn execute_python(code: String) -> PyResult<String> {
    let guard = SDK.lock().map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;
    let sdk = guard.as_ref().ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("not initialized"))?;
    let result = sdk.execute_python(&code);
    let json = serde_json::json!({
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exit_code": result.exit_code,
    });
    Ok(json.to_string())
}

#[pyfunction]
fn get_cwd() -> PyResult<String> {
    let guard = SDK.lock().map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;
    let sdk = guard.as_ref().ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("not initialized"))?;
    Ok(sdk.get_cwd())
}

#[pyfunction]
fn read_file(path: String) -> PyResult<String> {
    let guard = SDK.lock().map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;
    let sdk = guard.as_ref().ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("not initialized"))?;
    sdk.read_file(&path).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e))
}

#[pyfunction]
fn write_file(path: String, content: String) -> PyResult<()> {
    let guard = SDK.lock().map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;
    let sdk = guard.as_ref().ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("not initialized"))?;
    sdk.write_file(&path, &content).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e))
}

#[pymodule]
fn fastshell(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(init, m)?)?;
    m.add_function(wrap_pyfunction!(execute, m)?)?;
    m.add_function(wrap_pyfunction!(execute_python, m)?)?;
    m.add_function(wrap_pyfunction!(get_cwd, m)?)?;
    m.add_function(wrap_pyfunction!(read_file, m)?)?;
    m.add_function(wrap_pyfunction!(write_file, m)?)?;
    Ok(())
}
