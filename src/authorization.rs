//! Native macOS authorization for privileged commands
//! Replaces osascript "do shell script with administrator privileges"

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::ptr;

// Security framework FFI
#[link(name = "Security", kind = "framework")]
extern "C" {
    fn AuthorizationCreate(
        rights: *const c_void,
        environment: *const c_void,
        flags: u32,
        authorization: *mut AuthorizationRef,
    ) -> i32;

    fn AuthorizationFree(authorization: AuthorizationRef, flags: u32) -> i32;

    fn AuthorizationExecuteWithPrivileges(
        authorization: AuthorizationRef,
        path_to_tool: *const c_char,
        options: u32,
        arguments: *const *const c_char,
        communicationsPipe: *mut *mut c_void,
    ) -> i32;
}

type AuthorizationRef = *mut c_void;

const K_AUTHORIZATION_FLAG_DEFAULTS: u32 = 0;
const K_AUTHORIZATION_FLAG_INTERACTION_ALLOWED: u32 = 1 << 0;
const K_AUTHORIZATION_FLAG_EXTEND_RIGHTS: u32 = 1 << 1;
const K_AUTHORIZATION_FLAG_PREAUTHORIZE: u32 = 1 << 4;

const ERR_AUTHORIZATION_SUCCESS: i32 = 0;
const ERR_AUTHORIZATION_CANCELED: i32 = -60006;

/// Execute a command with administrator privileges
/// Returns Ok(true) if successful, Ok(false) if user cancelled, Err on failure
pub fn execute_with_privileges(command: &str, args: &[&str]) -> Result<bool, String> {
    unsafe {
        let mut auth_ref: AuthorizationRef = ptr::null_mut();

        // Create authorization reference with interaction allowed
        let flags = K_AUTHORIZATION_FLAG_DEFAULTS
            | K_AUTHORIZATION_FLAG_INTERACTION_ALLOWED
            | K_AUTHORIZATION_FLAG_EXTEND_RIGHTS
            | K_AUTHORIZATION_FLAG_PREAUTHORIZE;

        let result = AuthorizationCreate(ptr::null(), ptr::null(), flags, &mut auth_ref);

        if result != ERR_AUTHORIZATION_SUCCESS {
            if result == ERR_AUTHORIZATION_CANCELED {
                return Ok(false);
            }
            return Err(format!("AuthorizationCreate failed: {}", result));
        }

        // Prepare command path
        let cmd_cstring =
            CString::new(command).map_err(|e| format!("Invalid command: {}", e))?;

        // Prepare arguments
        let args_cstrings: Vec<CString> = args
            .iter()
            .map(|s| CString::new(*s).unwrap())
            .collect();

        let mut args_ptrs: Vec<*const c_char> = args_cstrings.iter().map(|s| s.as_ptr()).collect();
        args_ptrs.push(ptr::null()); // NULL terminator

        // Execute with privileges
        let exec_result = AuthorizationExecuteWithPrivileges(
            auth_ref,
            cmd_cstring.as_ptr(),
            0,
            args_ptrs.as_ptr(),
            ptr::null_mut(),
        );

        // Free authorization
        AuthorizationFree(auth_ref, 0);

        if exec_result == ERR_AUTHORIZATION_SUCCESS {
            Ok(true)
        } else if exec_result == ERR_AUTHORIZATION_CANCELED {
            Ok(false)
        } else {
            Err(format!(
                "AuthorizationExecuteWithPrivileges failed: {}",
                exec_result
            ))
        }
    }
}

/// Execute a shell script with administrator privileges
pub fn execute_script_with_privileges(script: &str) -> Result<bool, String> {
    execute_with_privileges("/bin/sh", &["-c", script])
}

/// Run multiple commands with a single authentication prompt
pub fn execute_commands_with_privileges(commands: &[&str]) -> Result<bool, String> {
    // Join commands with && so they all run with one auth prompt
    let script = commands.join(" && ");
    execute_script_with_privileges(&script)
}
