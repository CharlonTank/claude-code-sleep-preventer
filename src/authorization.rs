//! Native macOS authorization for privileged commands
//! Uses osascript to run AppleScript with admin privileges

use std::process::Command;

/// Execute a shell script with administrator privileges
/// Returns Ok(true) if successful, Ok(false) if user cancelled, Err on failure
pub fn execute_script_with_privileges(script: &str) -> Result<bool, String> {
    // Build AppleScript that runs shell command with admin privileges
    let applescript = format!(
        "do shell script \"{}\" with administrator privileges",
        script.replace("\\", "\\\\").replace("\"", "\\\"")
    );

    let output = Command::new("osascript")
        .args(["-e", &applescript])
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if output.status.success() {
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // User cancelled
        if stderr.contains("-128") || stderr.contains("User canceled") {
            return Ok(false);
        }
        Err(stderr.trim().to_string())
    }
}

/// Run multiple commands with a single authentication prompt
pub fn execute_commands_with_privileges(commands: &[&str]) -> Result<bool, String> {
    let script = commands.join(" && ");
    execute_script_with_privileges(&script)
}
