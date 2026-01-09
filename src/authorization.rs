//! Native macOS authorization for privileged commands
//! Uses NSAppleScript which shows the app name in the auth dialog

use cocoa::base::{id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSString};
use objc::{class, msg_send, sel, sel_impl};

/// Execute a shell script with administrator privileges
/// Uses NSAppleScript so the auth dialog shows "Claude Sleep Preventer" not "osascript"
/// Returns Ok(true) if successful, Ok(false) if user cancelled, Err on failure
pub fn execute_script_with_privileges(script: &str) -> Result<bool, String> {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        // Escape single quotes in the script
        let escaped_script = script.replace("'", "'\"'\"'");

        // Build AppleScript that runs shell command with admin privileges
        let applescript_source = format!(
            "do shell script '{}' with administrator privileges",
            escaped_script
        );

        let source_nsstring = NSString::alloc(nil).init_str(&applescript_source);

        // Create NSAppleScript
        let script_obj: id = msg_send![class!(NSAppleScript), alloc];
        let script_obj: id = msg_send![script_obj, initWithSource: source_nsstring];

        if script_obj == nil {
            return Err("Failed to create NSAppleScript".to_string());
        }

        // Execute the script
        let mut error_dict: id = nil;
        let _result: id = msg_send![script_obj, executeAndReturnError: &mut error_dict];

        if error_dict != nil {
            // Get error number
            let error_number_key = NSString::alloc(nil).init_str("NSAppleScriptErrorNumber");
            let error_number: id = msg_send![error_dict, objectForKey: error_number_key];

            if error_number != nil {
                let error_code: i64 = msg_send![error_number, integerValue];
                // -128 = user cancelled
                if error_code == -128 {
                    return Ok(false);
                }
            }

            // Get error message
            let error_msg_key = NSString::alloc(nil).init_str("NSAppleScriptErrorMessage");
            let error_msg: id = msg_send![error_dict, objectForKey: error_msg_key];

            if error_msg != nil {
                let msg_cstr: *const i8 = msg_send![error_msg, UTF8String];
                if !msg_cstr.is_null() {
                    let msg = std::ffi::CStr::from_ptr(msg_cstr)
                        .to_string_lossy()
                        .to_string();
                    return Err(msg);
                }
            }

            return Err("AppleScript execution failed".to_string());
        }

        Ok(true)
    }
}

/// Run multiple commands with a single authentication prompt
pub fn execute_commands_with_privileges(commands: &[&str]) -> Result<bool, String> {
    let script = commands.join(" && ");
    execute_script_with_privileges(&script)
}
