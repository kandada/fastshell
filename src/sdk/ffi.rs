use crate::sdk::Fastshell;
use std::sync::{Mutex, OnceLock};

static SDK_INSTANCE: OnceLock<Mutex<Fastshell>> = OnceLock::new();

fn get_sdk() -> &'static Mutex<Fastshell> {
    SDK_INSTANCE.get_or_init(|| Mutex::new(Fastshell::new()))
}

pub(crate) fn get_sdk_internal() -> &'static Mutex<Fastshell> {
    SDK_INSTANCE.get_or_init(|| Mutex::new(Fastshell::new()))
}

fn result_to_cstring(result: &crate::sdk::types::CommandResult) -> *const std::os::raw::c_char {
    let json = serde_json::json!({
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exit_code": result.exit_code,
    });
    std::ffi::CString::new(json.to_string()).unwrap().into_raw()
}

fn error_to_cstring(msg: &str, code: i32) -> *const std::os::raw::c_char {
    let json = serde_json::json!({
        "stdout": "",
        "stderr": msg,
        "exit_code": code,
    });
    std::ffi::CString::new(json.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub extern "C" fn fastshell_free_string(ptr: *mut std::os::raw::c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = std::ffi::CString::from_raw(ptr);
    }
}

#[cfg(target_os = "android")]
pub mod android {
    use super::*;

    #[no_mangle]
    pub extern "C" fn Java_com_fastshell_Sdk_nativeInit(
        _env: *mut std::os::raw::c_void,
        _class: *mut std::os::raw::c_void,
        sandbox_path: *const std::os::raw::c_char,
    ) -> *const std::os::raw::c_char {
        let sandbox_path = unsafe {
            std::ffi::CStr::from_ptr(sandbox_path)
                .to_string_lossy()
                .to_string()
        };

        let mut sdk = get_sdk().lock().unwrap();
        let config = crate::sdk::types::Config {
            sandbox_path,
            python_enabled: true,
            command_timeout_ms: 30_000,
        };

        match sdk.init(config) {
            Ok(()) => error_to_cstring("", 0),
            Err(e) => error_to_cstring(&e, 1),
        }
    }

    #[no_mangle]
    pub extern "C" fn Java_com_fastshell_Sdk_nativeExecute(
        _env: *mut std::os::raw::c_void,
        _class: *mut std::os::raw::c_void,
        command: *const std::os::raw::c_char,
    ) -> *const std::os::raw::c_char {
        let command = unsafe {
            std::ffi::CStr::from_ptr(command)
                .to_string_lossy()
                .to_string()
        };

        let sdk = get_sdk().lock().unwrap();
        let result = sdk.execute(&command);
        result_to_cstring(&result)
    }

    #[no_mangle]
    pub extern "C" fn Java_com_fastshell_Sdk_nativeGetCwd(
        _env: *mut std::os::raw::c_void,
        _class: *mut std::os::raw::c_void,
    ) -> *const std::os::raw::c_char {
        let sdk = get_sdk().lock().unwrap();
        let cwd = sdk.get_cwd();
        std::ffi::CString::new(cwd).unwrap().into_raw()
    }
}

#[cfg(target_os = "ios")]
pub mod ios {
    use super::*;

    #[no_mangle]
    pub extern "C" fn fastshell_ios_init(sandbox_path: *const std::os::raw::c_char) -> *const std::os::raw::c_char {
        let sandbox_path = unsafe {
            std::ffi::CStr::from_ptr(sandbox_path)
                .to_string_lossy()
                .to_string()
        };

        let mut sdk = get_sdk().lock().unwrap();
        let config = crate::sdk::types::Config {
            sandbox_path,
            python_enabled: true,
            command_timeout_ms: 30_000,
        };

        match sdk.init(config) {
            Ok(()) => error_to_cstring("", 0),
            Err(e) => error_to_cstring(&e, 1),
        }
    }

    #[no_mangle]
    pub extern "C" fn fastshell_ios_execute(command: *const std::os::raw::c_char) -> *const std::os::raw::c_char {
        let command = unsafe {
            std::ffi::CStr::from_ptr(command)
                .to_string_lossy()
                .to_string()
        };

        let sdk = get_sdk().lock().unwrap();
        let result = sdk.execute(&command);
        result_to_cstring(&result)
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod c_ffi {
    use super::*;

    #[no_mangle]
    pub extern "C" fn fastshell_init(sandbox_path: *const std::os::raw::c_char) -> *const std::os::raw::c_char {
        let sandbox_path = unsafe {
            std::ffi::CStr::from_ptr(sandbox_path)
                .to_string_lossy()
                .to_string()
        };

        let mut sdk = get_sdk().lock().unwrap();
        let config = crate::sdk::types::Config {
            sandbox_path,
            python_enabled: true,
            command_timeout_ms: 30_000,
        };

        match sdk.init(config) {
            Ok(()) => error_to_cstring("", 0),
            Err(e) => error_to_cstring(&e, 1),
        }
    }

    #[no_mangle]
    pub extern "C" fn fastshell_execute(command: *const std::os::raw::c_char) -> *const std::os::raw::c_char {
        let command = unsafe {
            std::ffi::CStr::from_ptr(command)
                .to_string_lossy()
                .to_string()
        };

        let sdk = get_sdk().lock().unwrap();
        let result = sdk.execute(&command);
        result_to_cstring(&result)
    }
}
