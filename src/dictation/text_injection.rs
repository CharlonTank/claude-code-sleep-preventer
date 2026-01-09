use core_foundation::base::{CFRelease, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;
use std::ptr;

use crate::logging;

// AXUIElement FFI bindings
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXIsProcessTrusted() -> bool;
}

type AXUIElementRef = *mut c_void;
type CFTypeRef = *mut c_void;

// AX error codes
const K_AX_ERROR_SUCCESS: i32 = 0;

pub fn inject_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    // Check if we have accessibility permission
    let trusted = unsafe { AXIsProcessTrusted() };
    if !trusted {
        return Err("Accessibility permission required. Please enable in System Preferences > Security & Privacy > Privacy > Accessibility".to_string());
    }

    unsafe {
        // Get the system-wide accessibility element
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return Err("Failed to create system-wide element".to_string());
        }

        // Get the focused UI element
        let focused_attr = CFString::new("AXFocusedUIElement");
        let mut focused_element: CFTypeRef = ptr::null_mut();

        let result = AXUIElementCopyAttributeValue(
            system_wide,
            focused_attr.as_concrete_TypeRef(),
            &mut focused_element,
        );

        CFRelease(system_wide as *mut c_void);

        if result != K_AX_ERROR_SUCCESS || focused_element.is_null() {
            return Err(format!("No focused element found (error: {})", result));
        }

        // Try to set AXSelectedText (replaces selection or inserts at cursor)
        let selected_text_attr = CFString::new("AXSelectedText");
        let text_value = CFString::new(text);

        let set_result = AXUIElementSetAttributeValue(
            focused_element as AXUIElementRef,
            selected_text_attr.as_concrete_TypeRef(),
            text_value.as_concrete_TypeRef() as CFTypeRef,
        );

        CFRelease(focused_element);

        if set_result != K_AX_ERROR_SUCCESS {
            // Fallback: try setting AXValue (replaces entire content)
            logging::log(&format!(
                "[text_injection] AXSelectedText failed ({}), trying fallback",
                set_result
            ));
            return Err(format!(
                "Failed to inject text via AXSelectedText (error: {})",
                set_result
            ));
        }

        logging::log(&format!(
            "[text_injection] Successfully injected {} chars",
            text.len()
        ));
        Ok(())
    }
}

/// Check if accessibility is enabled for this app
pub fn check_accessibility_permission() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Prompt user to enable accessibility permission
pub fn request_accessibility_permission() {
    // Open System Preferences to Accessibility pane
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();
}
