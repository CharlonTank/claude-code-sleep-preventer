use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

pub fn inject_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    // Save current clipboard content
    let original_clipboard = get_clipboard_content();

    // Set text to clipboard
    set_clipboard_content(text)?;

    // Small delay to ensure clipboard is ready
    thread::sleep(Duration::from_millis(50));

    // Simulate Cmd+V using AppleScript
    let script = r#"tell application "System Events" to keystroke "v" using command down"#;
    let output = Command::new("osascript")
        .args(["-e", script])
        .output()
        .map_err(|e| format!("Failed to execute paste: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Paste command failed: {}", stderr));
    }

    // Restore original clipboard after a short delay (in background)
    if let Some(content) = original_clipboard {
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            let _ = set_clipboard_content(&content);
        });
    }

    Ok(())
}

fn get_clipboard_content() -> Option<String> {
    Command::new("pbpaste")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
}

fn set_clipboard_content(text: &str) -> Result<(), String> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn pbcopy: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("Failed to write to pbcopy: {}", e))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for pbcopy: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err("pbcopy failed".to_string())
    }
}
