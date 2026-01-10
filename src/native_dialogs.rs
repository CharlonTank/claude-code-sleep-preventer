//! Native macOS dialogs using Cocoa NSAlert
//! Replaces osascript "display dialog" calls

use dispatch::Queue;
use objc::declare::ClassDecl;
use objc::runtime::{BOOL, Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

use crate::objc_utils::{
    nsstring, AutoreleasePool, Id, NSPoint, NSRect, NSSize, NIL, NS_BACKING_STORE_BUFFERED,
    NS_WINDOW_STYLE_MASK_TITLED,
};

fn is_main_thread() -> bool {
    unsafe {
        let is_main: BOOL = msg_send![class!(NSThread), isMainThread];
        is_main
    }
}

fn run_on_main_thread<T, F>(work: F) -> T
where
    F: Send + FnOnce() -> T,
    T: Send,
{
    if is_main_thread() {
        work()
    } else {
        Queue::main().exec_sync(work)
    }
}

fn run_on_main_async<F>(work: F)
where
    F: Send + 'static + FnOnce(),
{
    if is_main_thread() {
        work()
    } else {
        Queue::main().exec_async(work)
    }
}

/// Show an informational dialog with OK button
pub fn show_dialog(message: &str, title: &str) {
    run_on_main_thread(|| unsafe {
        let _pool = AutoreleasePool::new();

        // Activate the app to bring dialog to front
        // For LSUIElement apps, we need to set activation policy to Regular temporarily
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let previous_policy: i64 = msg_send![app, activationPolicy];
        // NSApplicationActivationPolicyRegular = 0
        let _: () = msg_send![app, setActivationPolicy: 0i64];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];

        let alert: Id = msg_send![class!(NSAlert), new];

        // NSAlertStyleInformational = 1
        let _: () = msg_send![alert, setAlertStyle: 1i64];

        let title_str = nsstring(&title);
        let _: () = msg_send![alert, setMessageText: title_str];

        let message_str = nsstring(&message);
        let _: () = msg_send![alert, setInformativeText: message_str];

        let ok_str = nsstring("OK");
        let _: () = msg_send![alert, addButtonWithTitle: ok_str];

        let _: i64 = msg_send![alert, runModal];
        let _: () = msg_send![app, setActivationPolicy: previous_policy];
    });
}

/// Show a confirmation dialog with two buttons, returns true if confirmed
pub fn show_confirm_dialog(message: &str, title: &str, confirm: &str, cancel: &str) -> bool {
    run_on_main_thread(|| unsafe {
        let _pool = AutoreleasePool::new();

        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let previous_policy: i64 = msg_send![app, activationPolicy];
        // NSApplicationActivationPolicyRegular = 0
        let _: () = msg_send![app, setActivationPolicy: 0i64];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];

        let alert: Id = msg_send![class!(NSAlert), new];

        // NSAlertStyleWarning = 0
        let _: () = msg_send![alert, setAlertStyle: 0i64];

        let title_str = nsstring(title);
        let _: () = msg_send![alert, setMessageText: title_str];

        let message_str = nsstring(message);
        let _: () = msg_send![alert, setInformativeText: message_str];

        // First button is default (confirm)
        let confirm_str = nsstring(confirm);
        let _: () = msg_send![alert, addButtonWithTitle: confirm_str];

        // Second button (cancel)
        let cancel_str = nsstring(cancel);
        let _: () = msg_send![alert, addButtonWithTitle: cancel_str];

        let response: i64 = msg_send![alert, runModal];

        // NSAlertFirstButtonReturn = 1000
        let confirmed = response == 1000;
        let _: () = msg_send![app, setActivationPolicy: previous_policy];
        confirmed
    })
}

#[derive(Clone, Copy)]
struct SendPtr(*mut c_void);

unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

impl SendPtr {
    fn into_ptr(self) -> *mut c_void {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupAction {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionToggle {
    InputMonitoring,
    Microphone,
    Accessibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionsAction {
    Primary,
    Secondary,
    Toggle(PermissionToggle),
}

struct DialogState {
    action: Mutex<Option<SetupAction>>,
    checkbox: Mutex<bool>,
}

impl DialogState {
    fn new() -> Self {
        Self {
            action: Mutex::new(None),
            checkbox: Mutex::new(false),
        }
    }

    fn clear(&self) {
        let mut action = self.action.lock().unwrap();
        *action = None;
    }

    fn set_action(&self, action_value: SetupAction) {
        let mut action = self.action.lock().unwrap();
        *action = Some(action_value);
    }

    fn take_action(&self) -> Option<SetupAction> {
        self.action.lock().unwrap().take()
    }

    fn set_checkbox(&self, checked: bool) {
        let mut checkbox = self.checkbox.lock().unwrap();
        *checkbox = checked;
    }

    fn checkbox_checked(&self) -> bool {
        *self.checkbox.lock().unwrap()
    }
}

struct PermissionsState {
    action: Mutex<Option<PermissionsAction>>,
}

impl PermissionsState {
    fn new() -> Self {
        Self {
            action: Mutex::new(None),
        }
    }

    fn clear(&self) {
        let mut action = self.action.lock().unwrap();
        *action = None;
    }

    fn set_action(&self, action_value: PermissionsAction) {
        let mut action = self.action.lock().unwrap();
        *action = Some(action_value);
    }

    fn take_action(&self) -> Option<PermissionsAction> {
        self.action.lock().unwrap().take()
    }
}

extern "C" fn setup_button_pressed(this: &Object, _: Sel, sender: Id) {
    unsafe {
        let state_ptr: *mut c_void = *this.get_ivar("rustState");
        if !state_ptr.is_null() {
            let state = &*(state_ptr as *const DialogState);
            let tag: i64 = msg_send![sender, tag];
            let action = if tag == 1 {
                SetupAction::Primary
            } else {
                SetupAction::Secondary
            };
            state.set_action(action);
        }

        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, stopModal];
    }
}

extern "C" fn setup_checkbox_toggled(this: &Object, _: Sel, sender: Id) {
    unsafe {
        let state_ptr: *mut c_void = *this.get_ivar("rustState");
        if !state_ptr.is_null() {
            let state = &*(state_ptr as *const DialogState);
            let checked: i64 = msg_send![sender, state];
            state.set_checkbox(checked != 0);
        }
    }
}

extern "C" fn permissions_button_pressed(this: &Object, _: Sel, sender: Id) {
    unsafe {
        let state_ptr: *mut c_void = *this.get_ivar("rustState");
        if !state_ptr.is_null() {
            let state = &*(state_ptr as *const PermissionsState);
            let tag: i64 = msg_send![sender, tag];
            let action = if tag == 1 {
                PermissionsAction::Primary
            } else {
                PermissionsAction::Secondary
            };
            state.set_action(action);
        }

        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, stopModal];
    }
}

extern "C" fn permissions_toggle_pressed(this: &Object, _: Sel, sender: Id) {
    unsafe {
        let state_ptr: *mut c_void = *this.get_ivar("rustState");
        if !state_ptr.is_null() {
            let state = &*(state_ptr as *const PermissionsState);
            let tag: i64 = msg_send![sender, tag];
            let toggle = match tag {
                1 => PermissionToggle::InputMonitoring,
                2 => PermissionToggle::Microphone,
                3 => PermissionToggle::Accessibility,
                _ => return,
            };
            state.set_action(PermissionsAction::Toggle(toggle));
        }

        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, stopModal];
    }
}

struct ClassPtr(*const Class);

unsafe impl Send for ClassPtr {}
unsafe impl Sync for ClassPtr {}

fn setup_target_class() -> &'static Class {
    static CLASS: OnceLock<ClassPtr> = OnceLock::new();
    let class_ptr = CLASS.get_or_init(|| {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("CCSPSetupTarget", superclass)
            .expect("Failed to create CCSPSetupTarget class");
        decl.add_ivar::<*mut c_void>("rustState");
        unsafe {
            decl.add_method(
                sel!(buttonPressed:),
                setup_button_pressed as extern "C" fn(&Object, Sel, Id),
            );
            decl.add_method(
                sel!(checkboxToggled:),
                setup_checkbox_toggled as extern "C" fn(&Object, Sel, Id),
            );
        }
        ClassPtr(decl.register() as *const Class)
    });

    unsafe { &*class_ptr.0 }
}

fn permissions_target_class() -> &'static Class {
    static CLASS: OnceLock<ClassPtr> = OnceLock::new();
    let class_ptr = CLASS.get_or_init(|| {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("CCSPPermissionsTarget", superclass)
            .expect("Failed to create CCSPPermissionsTarget class");
        decl.add_ivar::<*mut c_void>("rustState");
        unsafe {
            decl.add_method(
                sel!(buttonPressed:),
                permissions_button_pressed as extern "C" fn(&Object, Sel, Id),
            );
            decl.add_method(
                sel!(togglePressed:),
                permissions_toggle_pressed as extern "C" fn(&Object, Sel, Id),
            );
        }
        ClassPtr(decl.register() as *const Class)
    });

    unsafe { &*class_ptr.0 }
}

#[derive(Clone, Copy)]
pub struct SetupWindowHandle {
    window: SendPtr,
    message: SendPtr,
    progress: SendPtr,
    checkbox: SendPtr,
    primary_button: SendPtr,
    secondary_button: SendPtr,
}

impl SetupWindowHandle {
    pub fn set_message(&self, message: &str) {
        let message = message.to_string();
        let label = self.message;
        run_on_main_async(move || unsafe {
            let message_str = nsstring(&message);
            let label = label.into_ptr() as Id;
            let _: () = msg_send![label, setStringValue: message_str];
        });
    }

    pub fn set_title(&self, title: &str) {
        let title = title.to_string();
        let window = self.window;
        run_on_main_async(move || unsafe {
            let title_str = nsstring(&title);
            let window = window.into_ptr() as Id;
            let _: () = msg_send![window, setTitle: title_str];
        });
    }

    pub fn set_primary_button(&self, title: &str) {
        let title = title.to_string();
        let button = self.primary_button;
        run_on_main_async(move || unsafe {
            let title_str = nsstring(&title);
            let button = button.into_ptr() as Id;
            let _: () = msg_send![button, setTitle: title_str];
        });
    }

    pub fn set_secondary_button(&self, title: &str) {
        let title = title.to_string();
        let button = self.secondary_button;
        run_on_main_async(move || unsafe {
            let title_str = nsstring(&title);
            let button = button.into_ptr() as Id;
            let _: () = msg_send![button, setTitle: title_str];
        });
    }

    pub fn set_primary_enabled(&self, enabled: bool) {
        let button = self.primary_button;
        run_on_main_async(move || unsafe {
            let button = button.into_ptr() as Id;
            let _: () = msg_send![button, setEnabled: enabled as BOOL];
        });
    }

    pub fn set_secondary_visible(&self, visible: bool) {
        let button = self.secondary_button;
        run_on_main_async(move || unsafe {
            let button = button.into_ptr() as Id;
            let _: () = msg_send![button, setHidden: (!visible) as BOOL];
        });
    }

    pub fn show_progress(&self, show: bool) {
        let progress = self.progress;
        run_on_main_async(move || unsafe {
            let progress = progress.into_ptr() as Id;
            let _: () = msg_send![progress, setHidden: (!show) as BOOL];
        });
    }

    pub fn set_progress(&self, percent: f64) {
        let progress = self.progress;
        let value = percent.clamp(0.0, 100.0);
        run_on_main_async(move || unsafe {
            let progress = progress.into_ptr() as Id;
            let _: () = msg_send![progress, setDoubleValue: value];
        });
    }

    pub fn set_checkbox(&self, label: &str, checked: bool) {
        let label = label.to_string();
        let checkbox = self.checkbox;
        run_on_main_async(move || unsafe {
            let label_str = nsstring(&label);
            let checkbox = checkbox.into_ptr() as Id;
            let _: () = msg_send![checkbox, setTitle: label_str];
            let _: () = msg_send![checkbox, setState: if checked { 1i64 } else { 0i64 }];
        });
    }

    pub fn set_checkbox_visible(&self, visible: bool) {
        let checkbox = self.checkbox;
        run_on_main_async(move || unsafe {
            let checkbox = checkbox.into_ptr() as Id;
            let _: () = msg_send![checkbox, setHidden: (!visible) as BOOL];
        });
    }

    pub fn stop_modal(&self) {
        run_on_main_async(|| unsafe {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, stopModal];
        });
    }
}

pub struct SetupWindow {
    handle: SetupWindowHandle,
    state: Arc<DialogState>,
    state_ptr: *const DialogState,
    target: SendPtr,
    previous_policy: i64,
}

impl SetupWindow {
    pub fn new(title: &str, message: &str) -> Self {
        let title = title.to_string();
        let message = message.to_string();
        let state = Arc::new(DialogState::new());
        let state_ptr = Arc::into_raw(state.clone());
        let state_ptr_send = SendPtr(state_ptr as *mut c_void);

        let (handle, target, previous_policy) = run_on_main_thread(move || unsafe {
            let _pool = AutoreleasePool::new();

            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let previous_policy: i64 = msg_send![app, activationPolicy];
            let _: () = msg_send![app, setActivationPolicy: 0i64];
            let _: () = msg_send![app, activateIgnoringOtherApps: true];

            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(520.0, 220.0));
            let window: Id = msg_send![class!(NSWindow), alloc];
            let window: Id = msg_send![
                window,
                initWithContentRect: frame
                styleMask: NS_WINDOW_STYLE_MASK_TITLED
                backing: NS_BACKING_STORE_BUFFERED
                defer: false as BOOL
            ];

            let title_str = nsstring(&title);
            let _: () = msg_send![window, setTitle: title_str];

            let content_view: Id = msg_send![window, contentView];

            let label_frame = NSRect::new(NSPoint::new(20.0, 110.0), NSSize::new(480.0, 80.0));
            let label: Id = msg_send![class!(NSTextField), alloc];
            let label: Id = msg_send![label, initWithFrame: label_frame];
            let message_str = nsstring(&message);
            let _: () = msg_send![label, setStringValue: message_str];
            let _: () = msg_send![label, setBezeled: false as BOOL];
            let _: () = msg_send![label, setDrawsBackground: false as BOOL];
            let _: () = msg_send![label, setEditable: false as BOOL];
            let _: () = msg_send![label, setSelectable: false as BOOL];
            let _: () = msg_send![label, setUsesSingleLineMode: false as BOOL];
            let _: () = msg_send![label, setLineBreakMode: 0i64];

            let progress_frame = NSRect::new(NSPoint::new(20.0, 80.0), NSSize::new(480.0, 20.0));
            let progress: Id = msg_send![class!(NSProgressIndicator), alloc];
            let progress: Id = msg_send![progress, initWithFrame: progress_frame];
            let _: () = msg_send![progress, setIndeterminate: false as BOOL];
            let _: () = msg_send![progress, setMinValue: 0.0];
            let _: () = msg_send![progress, setMaxValue: 100.0];
            let _: () = msg_send![progress, setDoubleValue: 0.0];
            let _: () = msg_send![progress, setStyle: 0i64];
            let _: () = msg_send![progress, setHidden: true as BOOL];

            let checkbox_frame = NSRect::new(NSPoint::new(20.0, 55.0), NSSize::new(480.0, 20.0));
            let checkbox: Id = msg_send![class!(NSButton), alloc];
            let checkbox: Id = msg_send![checkbox, initWithFrame: checkbox_frame];
            let _: () = msg_send![checkbox, setButtonType: 3i64];
            let _: () = msg_send![checkbox, setTitle: nsstring("")];
            let _: () = msg_send![checkbox, setState: 0i64];
            let _: () = msg_send![checkbox, setHidden: true as BOOL];
            let _: () = msg_send![checkbox, setAllowsMixedState: false as BOOL];

            let secondary_frame = NSRect::new(NSPoint::new(200.0, 20.0), NSSize::new(140.0, 32.0));
            let secondary: Id = msg_send![class!(NSButton), alloc];
            let secondary: Id = msg_send![secondary, initWithFrame: secondary_frame];
            let _: () = msg_send![secondary, setBezelStyle: 1i64];
            let _: () = msg_send![secondary, setTitle: nsstring("Annuler")];
            let _: () = msg_send![secondary, setTag: 0i64];

            let primary_frame = NSRect::new(NSPoint::new(360.0, 20.0), NSSize::new(140.0, 32.0));
            let primary: Id = msg_send![class!(NSButton), alloc];
            let primary: Id = msg_send![primary, initWithFrame: primary_frame];
            let _: () = msg_send![primary, setBezelStyle: 1i64];
            let _: () = msg_send![primary, setTitle: nsstring("OK")];
            let _: () = msg_send![primary, setTag: 1i64];
            let _: () = msg_send![primary, setKeyEquivalent: nsstring("\r")];

            let target: Id = msg_send![setup_target_class(), new];
            let target_obj = target as *mut Object;
            (*target_obj).set_ivar("rustState", state_ptr_send.into_ptr());

            let _: () = msg_send![primary, setTarget: target];
            let _: () = msg_send![primary, setAction: sel!(buttonPressed:)];
            let _: () = msg_send![secondary, setTarget: target];
            let _: () = msg_send![secondary, setAction: sel!(buttonPressed:)];
            let _: () = msg_send![checkbox, setTarget: target];
            let _: () = msg_send![checkbox, setAction: sel!(checkboxToggled:)];

            let _: () = msg_send![content_view, addSubview: label];
            let _: () = msg_send![content_view, addSubview: progress];
            let _: () = msg_send![content_view, addSubview: checkbox];
            let _: () = msg_send![content_view, addSubview: primary];
            let _: () = msg_send![content_view, addSubview: secondary];

            let _: () = msg_send![window, center];
            let _: () = msg_send![window, makeKeyAndOrderFront: NIL];

            (
                SetupWindowHandle {
                    window: SendPtr(window as *mut c_void),
                    message: SendPtr(label as *mut c_void),
                    progress: SendPtr(progress as *mut c_void),
                    checkbox: SendPtr(checkbox as *mut c_void),
                    primary_button: SendPtr(primary as *mut c_void),
                    secondary_button: SendPtr(secondary as *mut c_void),
                },
                SendPtr(target as *mut c_void),
                previous_policy,
            )
        });

        Self {
            handle,
            state,
            state_ptr,
            target,
            previous_policy,
        }
    }

    pub fn handle(&self) -> SetupWindowHandle {
        self.handle
    }

    pub fn set_title(&self, title: &str) {
        self.handle.set_title(title);
    }

    pub fn set_message(&self, message: &str) {
        self.handle.set_message(message);
    }

    pub fn set_primary_button(&self, title: &str) {
        self.handle.set_primary_button(title);
    }

    pub fn set_secondary_button(&self, title: &str) {
        self.handle.set_secondary_button(title);
    }

    pub fn set_primary_enabled(&self, enabled: bool) {
        self.handle.set_primary_enabled(enabled);
    }

    pub fn set_secondary_visible(&self, visible: bool) {
        self.handle.set_secondary_visible(visible);
    }

    pub fn show_progress(&self, show: bool) {
        self.handle.show_progress(show);
    }

    pub fn set_progress(&self, percent: f64) {
        self.handle.set_progress(percent);
    }

    pub fn set_checkbox(&self, label: &str, checked: bool) {
        self.state.set_checkbox(checked);
        self.handle.set_checkbox(label, checked);
    }

    pub fn set_checkbox_visible(&self, visible: bool) {
        self.handle.set_checkbox_visible(visible);
    }

    pub fn checkbox_checked(&self) -> bool {
        self.state.checkbox_checked()
    }

    pub fn run_modal(&self) {
        let window = self.handle.window;
        run_on_main_thread(move || unsafe {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let window = window.into_ptr() as Id;
            let _: i64 = msg_send![app, runModalForWindow: window];
        });
    }

    pub fn wait_for_action(&self) -> SetupAction {
        self.state.clear();
        self.run_modal();
        self.state
            .take_action()
            .unwrap_or(SetupAction::Secondary)
    }

    pub fn close(&self) {
        let window = self.handle.window;
        let target = self.target;
        let previous_policy = self.previous_policy;
        let state_ptr = SendPtr(self.state_ptr as *mut c_void);
        run_on_main_thread(move || unsafe {
            let window = window.into_ptr() as Id;
            let _: () = msg_send![window, orderOut: NIL];
            let _: () = msg_send![window, close];
            let _: () = msg_send![window, release];

            let target = target.into_ptr() as Id;
            let _: () = msg_send![target, release];

            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, setActivationPolicy: previous_policy];

            drop(Arc::from_raw(state_ptr.into_ptr() as *const DialogState));
        });
    }
}

#[derive(Clone, Copy)]
pub struct PermissionsWindowHandle {
    window: SendPtr,
    message: SendPtr,
    input_toggle: SendPtr,
    mic_toggle: SendPtr,
    accessibility_toggle: SendPtr,
    primary_button: SendPtr,
    secondary_button: SendPtr,
}

impl PermissionsWindowHandle {
    pub fn set_message(&self, message: &str) {
        let message = message.to_string();
        let label = self.message;
        run_on_main_async(move || unsafe {
            let message_str = nsstring(&message);
            let label = label.into_ptr() as Id;
            let _: () = msg_send![label, setStringValue: message_str];
        });
    }

    pub fn set_title(&self, title: &str) {
        let title = title.to_string();
        let window = self.window;
        run_on_main_async(move || unsafe {
            let title_str = nsstring(&title);
            let window = window.into_ptr() as Id;
            let _: () = msg_send![window, setTitle: title_str];
        });
    }

    pub fn set_primary_button(&self, title: &str) {
        let title = title.to_string();
        let button = self.primary_button;
        run_on_main_async(move || unsafe {
            let title_str = nsstring(&title);
            let button = button.into_ptr() as Id;
            let _: () = msg_send![button, setTitle: title_str];
        });
    }

    pub fn set_secondary_button(&self, title: &str) {
        let title = title.to_string();
        let button = self.secondary_button;
        run_on_main_async(move || unsafe {
            let title_str = nsstring(&title);
            let button = button.into_ptr() as Id;
            let _: () = msg_send![button, setTitle: title_str];
        });
    }

    pub fn set_secondary_visible(&self, visible: bool) {
        let button = self.secondary_button;
        run_on_main_async(move || unsafe {
            let button = button.into_ptr() as Id;
            let _: () = msg_send![button, setHidden: (!visible) as BOOL];
        });
    }

    pub fn set_toggle(&self, toggle: PermissionToggle, label: &str, checked: bool) {
        let label = label.to_string();
        let button = match toggle {
            PermissionToggle::InputMonitoring => self.input_toggle,
            PermissionToggle::Microphone => self.mic_toggle,
            PermissionToggle::Accessibility => self.accessibility_toggle,
        };
        run_on_main_async(move || unsafe {
            let label_str = nsstring(&label);
            let button = button.into_ptr() as Id;
            let _: () = msg_send![button, setTitle: label_str];
            let _: () = msg_send![button, setState: if checked { 1i64 } else { 0i64 }];
        });
    }
}

pub struct PermissionsWindow {
    handle: PermissionsWindowHandle,
    state: Arc<PermissionsState>,
    state_ptr: *const PermissionsState,
    target: SendPtr,
    previous_policy: i64,
}

impl PermissionsWindow {
    pub fn new(title: &str, message: &str) -> Self {
        let title = title.to_string();
        let message = message.to_string();
        let state = Arc::new(PermissionsState::new());
        let state_ptr = Arc::into_raw(state.clone());
        let state_ptr_send = SendPtr(state_ptr as *mut c_void);

        let (handle, target, previous_policy) = run_on_main_thread(move || unsafe {
            let _pool = AutoreleasePool::new();

            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let previous_policy: i64 = msg_send![app, activationPolicy];
            let _: () = msg_send![app, setActivationPolicy: 0i64];
            let _: () = msg_send![app, activateIgnoringOtherApps: true];

            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(520.0, 280.0));
            let window: Id = msg_send![class!(NSWindow), alloc];
            let window: Id = msg_send![
                window,
                initWithContentRect: frame
                styleMask: NS_WINDOW_STYLE_MASK_TITLED
                backing: NS_BACKING_STORE_BUFFERED
                defer: false as BOOL
            ];

            let title_str = nsstring(&title);
            let _: () = msg_send![window, setTitle: title_str];

            let content_view: Id = msg_send![window, contentView];

            let label_frame = NSRect::new(NSPoint::new(20.0, 170.0), NSSize::new(480.0, 80.0));
            let label: Id = msg_send![class!(NSTextField), alloc];
            let label: Id = msg_send![label, initWithFrame: label_frame];
            let message_str = nsstring(&message);
            let _: () = msg_send![label, setStringValue: message_str];
            let _: () = msg_send![label, setBezeled: false as BOOL];
            let _: () = msg_send![label, setDrawsBackground: false as BOOL];
            let _: () = msg_send![label, setEditable: false as BOOL];
            let _: () = msg_send![label, setSelectable: false as BOOL];
            let _: () = msg_send![label, setUsesSingleLineMode: false as BOOL];
            let _: () = msg_send![label, setLineBreakMode: 0i64];

            let input_frame = NSRect::new(NSPoint::new(20.0, 130.0), NSSize::new(480.0, 20.0));
            let input_toggle: Id = msg_send![class!(NSButton), alloc];
            let input_toggle: Id = msg_send![input_toggle, initWithFrame: input_frame];
            let _: () = msg_send![input_toggle, setButtonType: 3i64];
            let _: () = msg_send![input_toggle, setTitle: nsstring("")];
            let _: () = msg_send![input_toggle, setState: 0i64];
            let _: () = msg_send![input_toggle, setTag: 1i64];
            let _: () = msg_send![input_toggle, setAllowsMixedState: false as BOOL];

            let mic_frame = NSRect::new(NSPoint::new(20.0, 105.0), NSSize::new(480.0, 20.0));
            let mic_toggle: Id = msg_send![class!(NSButton), alloc];
            let mic_toggle: Id = msg_send![mic_toggle, initWithFrame: mic_frame];
            let _: () = msg_send![mic_toggle, setButtonType: 3i64];
            let _: () = msg_send![mic_toggle, setTitle: nsstring("")];
            let _: () = msg_send![mic_toggle, setState: 0i64];
            let _: () = msg_send![mic_toggle, setTag: 2i64];
            let _: () = msg_send![mic_toggle, setAllowsMixedState: false as BOOL];

            let accessibility_frame = NSRect::new(NSPoint::new(20.0, 80.0), NSSize::new(480.0, 20.0));
            let accessibility_toggle: Id = msg_send![class!(NSButton), alloc];
            let accessibility_toggle: Id =
                msg_send![accessibility_toggle, initWithFrame: accessibility_frame];
            let _: () = msg_send![accessibility_toggle, setButtonType: 3i64];
            let _: () = msg_send![accessibility_toggle, setTitle: nsstring("")];
            let _: () = msg_send![accessibility_toggle, setState: 0i64];
            let _: () = msg_send![accessibility_toggle, setTag: 3i64];
            let _: () = msg_send![accessibility_toggle, setAllowsMixedState: false as BOOL];

            let secondary_frame = NSRect::new(NSPoint::new(200.0, 20.0), NSSize::new(140.0, 32.0));
            let secondary: Id = msg_send![class!(NSButton), alloc];
            let secondary: Id = msg_send![secondary, initWithFrame: secondary_frame];
            let _: () = msg_send![secondary, setBezelStyle: 1i64];
            let _: () = msg_send![secondary, setTitle: nsstring("Plus tard")];
            let _: () = msg_send![secondary, setTag: 0i64];

            let primary_frame = NSRect::new(NSPoint::new(360.0, 20.0), NSSize::new(140.0, 32.0));
            let primary: Id = msg_send![class!(NSButton), alloc];
            let primary: Id = msg_send![primary, initWithFrame: primary_frame];
            let _: () = msg_send![primary, setBezelStyle: 1i64];
            let _: () = msg_send![primary, setTitle: nsstring("Continuer")];
            let _: () = msg_send![primary, setTag: 1i64];
            let _: () = msg_send![primary, setKeyEquivalent: nsstring("\r")];

            let target: Id = msg_send![permissions_target_class(), new];
            let target_obj = target as *mut Object;
            (*target_obj).set_ivar("rustState", state_ptr_send.into_ptr());

            let _: () = msg_send![primary, setTarget: target];
            let _: () = msg_send![primary, setAction: sel!(buttonPressed:)];
            let _: () = msg_send![secondary, setTarget: target];
            let _: () = msg_send![secondary, setAction: sel!(buttonPressed:)];
            let _: () = msg_send![input_toggle, setTarget: target];
            let _: () = msg_send![input_toggle, setAction: sel!(togglePressed:)];
            let _: () = msg_send![mic_toggle, setTarget: target];
            let _: () = msg_send![mic_toggle, setAction: sel!(togglePressed:)];
            let _: () = msg_send![accessibility_toggle, setTarget: target];
            let _: () = msg_send![accessibility_toggle, setAction: sel!(togglePressed:)];

            let _: () = msg_send![content_view, addSubview: label];
            let _: () = msg_send![content_view, addSubview: input_toggle];
            let _: () = msg_send![content_view, addSubview: mic_toggle];
            let _: () = msg_send![content_view, addSubview: accessibility_toggle];
            let _: () = msg_send![content_view, addSubview: primary];
            let _: () = msg_send![content_view, addSubview: secondary];

            let _: () = msg_send![window, center];
            let _: () = msg_send![window, makeKeyAndOrderFront: NIL];

            (
                PermissionsWindowHandle {
                    window: SendPtr(window as *mut c_void),
                    message: SendPtr(label as *mut c_void),
                    input_toggle: SendPtr(input_toggle as *mut c_void),
                    mic_toggle: SendPtr(mic_toggle as *mut c_void),
                    accessibility_toggle: SendPtr(accessibility_toggle as *mut c_void),
                    primary_button: SendPtr(primary as *mut c_void),
                    secondary_button: SendPtr(secondary as *mut c_void),
                },
                SendPtr(target as *mut c_void),
                previous_policy,
            )
        });

        Self {
            handle,
            state,
            state_ptr,
            target,
            previous_policy,
        }
    }

    pub fn handle(&self) -> PermissionsWindowHandle {
        self.handle
    }

    pub fn set_title(&self, title: &str) {
        self.handle.set_title(title);
    }

    pub fn set_message(&self, message: &str) {
        self.handle.set_message(message);
    }

    pub fn set_primary_button(&self, title: &str) {
        self.handle.set_primary_button(title);
    }

    pub fn set_secondary_button(&self, title: &str) {
        self.handle.set_secondary_button(title);
    }

    pub fn set_secondary_visible(&self, visible: bool) {
        self.handle.set_secondary_visible(visible);
    }

    pub fn set_toggle(&self, toggle: PermissionToggle, label: &str, checked: bool) {
        self.handle.set_toggle(toggle, label, checked);
    }

    pub fn run_modal(&self) {
        let window = self.handle.window;
        run_on_main_thread(move || unsafe {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let window = window.into_ptr() as Id;
            let _: i64 = msg_send![app, runModalForWindow: window];
        });
    }

    pub fn wait_for_action(&self) -> PermissionsAction {
        self.state.clear();
        self.run_modal();
        self.state
            .take_action()
            .unwrap_or(PermissionsAction::Secondary)
    }

    pub fn close(&self) {
        let window = self.handle.window;
        let target = self.target;
        let previous_policy = self.previous_policy;
        let state_ptr = SendPtr(self.state_ptr as *mut c_void);
        run_on_main_thread(move || unsafe {
            let window = window.into_ptr() as Id;
            let _: () = msg_send![window, orderOut: NIL];
            let _: () = msg_send![window, close];
            let _: () = msg_send![window, release];

            let target = target.into_ptr() as Id;
            let _: () = msg_send![target, release];

            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, setActivationPolicy: previous_policy];

            drop(Arc::from_raw(state_ptr.into_ptr() as *const PermissionsState));
        });
    }
}
