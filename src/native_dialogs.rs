//! Native macOS dialogs using Cocoa NSAlert
//! Replaces osascript "display dialog" calls

use cocoa::base::{id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSString};
use objc::{class, msg_send, sel, sel_impl};

/// Show an informational dialog with OK button
pub fn show_dialog(message: &str, title: &str) {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        // Activate the app to bring dialog to front
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];

        let alert: id = msg_send![class!(NSAlert), new];

        // NSAlertStyleInformational = 1
        let _: () = msg_send![alert, setAlertStyle: 1i64];

        let title_str = NSString::alloc(nil).init_str(title);
        let _: () = msg_send![alert, setMessageText: title_str];

        let message_str = NSString::alloc(nil).init_str(message);
        let _: () = msg_send![alert, setInformativeText: message_str];

        let ok_str = NSString::alloc(nil).init_str("OK");
        let _: () = msg_send![alert, addButtonWithTitle: ok_str];

        let _: i64 = msg_send![alert, runModal];
    }
}

/// Show a confirmation dialog with two buttons, returns true if confirmed
pub fn show_confirm_dialog(message: &str, title: &str, confirm: &str, cancel: &str) -> bool {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];

        let alert: id = msg_send![class!(NSAlert), new];

        // NSAlertStyleWarning = 0
        let _: () = msg_send![alert, setAlertStyle: 0i64];

        let title_str = NSString::alloc(nil).init_str(title);
        let _: () = msg_send![alert, setMessageText: title_str];

        let message_str = NSString::alloc(nil).init_str(message);
        let _: () = msg_send![alert, setInformativeText: message_str];

        // First button is default (confirm)
        let confirm_str = NSString::alloc(nil).init_str(confirm);
        let _: () = msg_send![alert, addButtonWithTitle: confirm_str];

        // Second button (cancel)
        let cancel_str = NSString::alloc(nil).init_str(cancel);
        let _: () = msg_send![alert, addButtonWithTitle: cancel_str];

        let response: i64 = msg_send![alert, runModal];

        // NSAlertFirstButtonReturn = 1000
        response == 1000
    }
}

/// Show a warning dialog (for destructive actions like uninstall)
pub fn show_warning_dialog(message: &str, title: &str, confirm: &str, cancel: &str) -> bool {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];

        let alert: id = msg_send![class!(NSAlert), new];

        // NSAlertStyleCritical = 2
        let _: () = msg_send![alert, setAlertStyle: 2i64];

        let title_str = NSString::alloc(nil).init_str(title);
        let _: () = msg_send![alert, setMessageText: title_str];

        let message_str = NSString::alloc(nil).init_str(message);
        let _: () = msg_send![alert, setInformativeText: message_str];

        let confirm_str = NSString::alloc(nil).init_str(confirm);
        let _: () = msg_send![alert, addButtonWithTitle: confirm_str];

        let cancel_str = NSString::alloc(nil).init_str(cancel);
        let _: () = msg_send![alert, addButtonWithTitle: cancel_str];

        let response: i64 = msg_send![alert, runModal];
        response == 1000
    }
}

/// Show a notification (non-blocking)
pub fn show_notification(message: &str, title: &str) {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        let center: id = msg_send![class!(NSUserNotificationCenter), defaultUserNotificationCenter];
        let notification: id = msg_send![class!(NSUserNotification), new];

        let title_str = NSString::alloc(nil).init_str(title);
        let _: () = msg_send![notification, setTitle: title_str];

        let message_str = NSString::alloc(nil).init_str(message);
        let _: () = msg_send![notification, setInformativeText: message_str];

        let _: () = msg_send![center, deliverNotification: notification];
    }
}
