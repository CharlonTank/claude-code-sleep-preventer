use crate::logging;
use crate::objc_utils;
use core_foundation::base::TCFType;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

// Raw FFI bindings to Core Graphics
mod ffi {
    use std::ffi::c_void;

    pub type CGEventRef = *mut c_void;
    pub type CGEventTapProxy = *mut c_void;
    pub type CFMachPortRef = *mut c_void;
    pub type CFRunLoopSourceRef = *mut c_void;

    pub type CGEventType = u32;
    pub const K_CG_EVENT_KEY_DOWN: CGEventType = 10;
    pub const K_CG_EVENT_KEY_UP: CGEventType = 11;
    pub const K_CG_EVENT_FLAGS_CHANGED: CGEventType = 12;
    pub const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFFFFFE;
    pub const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: CGEventType = 0xFFFFFFFF;

    pub type CGEventFlags = u64;
    pub const K_CG_EVENT_FLAG_MASK_SECONDARY_FN: CGEventFlags = 0x00800000;
    pub const K_CG_EVENT_FLAG_MASK_SHIFT: CGEventFlags = 0x00020000;

    pub type CGEventTapLocation = u32;
    pub const K_CG_SESSION_EVENT_TAP: CGEventTapLocation = 1;

    pub type CGEventTapPlacement = u32;
    pub const K_CG_HEAD_INSERT_EVENT_TAP: CGEventTapPlacement = 0;

    pub type CGEventTapOptions = u32;
    pub const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: CGEventTapOptions = 1;

    pub type CGEventMask = u64;
    pub type CGEventField = u32;
    pub const K_CG_KEYBOARD_EVENT_KEYCODE: CGEventField = 9;

    pub type CGEventTapCallBack = extern "C" fn(
        proxy: CGEventTapProxy,
        event_type: CGEventType,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGEventTapCreate(
            tap: CGEventTapLocation,
            place: CGEventTapPlacement,
            options: CGEventTapOptions,
            events_of_interest: CGEventMask,
            callback: CGEventTapCallBack,
            user_info: *mut c_void,
        ) -> CFMachPortRef;

        pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        pub fn CGEventGetFlags(event: CGEventRef) -> CGEventFlags;
        pub fn CGEventGetIntegerValueField(event: CGEventRef, field: CGEventField) -> i64;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFMachPortCreateRunLoopSource(
            allocator: *const c_void,
            port: CFMachPortRef,
            order: i64,
        ) -> CFRunLoopSourceRef;

        pub fn CFRunLoopAddSource(
            rl: *const c_void,
            source: CFRunLoopSourceRef,
            mode: *const c_void,
        );

        pub fn CFRunLoopRemoveSource(
            rl: *const c_void,
            source: CFRunLoopSourceRef,
            mode: *const c_void,
        );

        pub fn CFRunLoopRunInMode(
            mode: *const c_void,
            seconds: f64,
            return_after_source_handled: bool,
        ) -> i32;

        pub fn CFMachPortInvalidate(port: CFMachPortRef);

        pub fn CFRelease(cf: *const c_void);
    }
}

// Minimal IOHIDManager FFI to trigger Input Monitoring prompt on some systems.
mod hid {
    use std::ffi::c_void;

    pub type IOHIDManagerRef = *mut c_void;
    pub type IOOptionBits = u32;
    pub type IOReturn = i32;

    pub const K_IO_RETURN_SUCCESS: IOReturn = 0;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOHIDManagerCreate(
            allocator: *const c_void,
            options: IOOptionBits,
        ) -> IOHIDManagerRef;

        pub fn IOHIDManagerSetDeviceMatching(
            manager: IOHIDManagerRef,
            matching: *const c_void,
        );

        pub fn IOHIDManagerOpen(
            manager: IOHIDManagerRef,
            options: IOOptionBits,
        ) -> IOReturn;

        pub fn IOHIDManagerClose(
            manager: IOHIDManagerRef,
            options: IOOptionBits,
        ) -> IOReturn;

        pub fn IOHIDManagerScheduleWithRunLoop(
            manager: IOHIDManagerRef,
            run_loop: *const c_void,
            run_loop_mode: *const c_void,
        );

        pub fn IOHIDManagerUnscheduleFromRunLoop(
            manager: IOHIDManagerRef,
            run_loop: *const c_void,
            run_loop_mode: *const c_void,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlobeKeyEvent {
    Ready,
    DictateStart,
    DictateStop,
}

pub struct GlobeKeyManager {
    event_rx: Option<Receiver<GlobeKeyEvent>>,
    stop_flag: Option<Arc<AtomicBool>>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl GlobeKeyManager {
    pub fn new() -> Self {
        Self {
            event_rx: None,
            stop_flag: None,
            thread_handle: None,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.event_rx.is_some() {
            return Ok(());
        }

        let (tx, rx) = mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = stop_flag.clone();

        let handle = thread::spawn(move || {
            run_event_tap(tx, stop_flag_clone);
        });

        self.event_rx = Some(rx);
        self.stop_flag = Some(stop_flag);
        self.thread_handle = Some(handle);

        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(flag) = self.stop_flag.take() {
            flag.store(true, Ordering::SeqCst);
        }
        self.thread_handle.take();
        self.event_rx = None;
    }

    pub fn try_recv(&self) -> Option<GlobeKeyEvent> {
        self.event_rx.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        })
    }
}

impl Drop for GlobeKeyManager {
    fn drop(&mut self) {
        self.stop();
    }
}

// Global state for callback (necessary because C callbacks can't capture Rust closures)
static CALLBACK_STATE: OnceLock<Mutex<Option<CallbackState>>> = OnceLock::new();
static EVENT_TAP: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());
static FLAGS_EVENT_COUNT: AtomicUsize = AtomicUsize::new(0);
static DISABLED_TIMEOUT_COUNT: AtomicUsize = AtomicUsize::new(0);
static DISABLED_USER_INPUT_COUNT: AtomicUsize = AtomicUsize::new(0);
static LAST_FLAGS_RAW: AtomicU64 = AtomicU64::new(0);
static LAST_KEYCODE: AtomicU64 = AtomicU64::new(u64::MAX);

fn callback_state() -> &'static Mutex<Option<CallbackState>> {
    CALLBACK_STATE.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone, Copy)]
pub struct GlobeKeyDiagnostics {
    pub flags_events: usize,
    pub disabled_timeout: usize,
    pub disabled_user_input: usize,
    pub last_flags_raw: u64,
    pub last_keycode: Option<u64>,
}

pub fn take_diagnostics() -> GlobeKeyDiagnostics {
    let flags_events = FLAGS_EVENT_COUNT.swap(0, Ordering::Relaxed);
    let disabled_timeout = DISABLED_TIMEOUT_COUNT.swap(0, Ordering::Relaxed);
    let disabled_user_input = DISABLED_USER_INPUT_COUNT.swap(0, Ordering::Relaxed);
    let last_flags_raw = LAST_FLAGS_RAW.load(Ordering::Relaxed);
    let last_keycode_raw = LAST_KEYCODE.load(Ordering::Relaxed);

    GlobeKeyDiagnostics {
        flags_events,
        disabled_timeout,
        disabled_user_input,
        last_flags_raw,
        last_keycode: if last_keycode_raw == u64::MAX {
            None
        } else {
            Some(last_keycode_raw)
        },
    }
}

pub fn check_input_monitoring_permission() -> bool {
    if let Some(granted) = cg_preflight_listen_event_access() {
        return granted;
    }

    match iohid_check_access(k_iohid_request_type_listen_event()) {
        Some(access) => access == k_iohid_access_type_granted(),
        None => true,
    }
}

pub fn request_input_monitoring_permission() -> bool {
    if check_input_monitoring_permission() {
        return true;
    }

    let bundle_id = objc_utils::main_bundle_identifier().unwrap_or_else(|| "unknown".to_string());
    let bundle_path = objc_utils::main_bundle_path().unwrap_or_else(|| "unknown".to_string());
    logging::log(&format!(
        "[input_monitoring] bundle id={}, path={}",
        bundle_id, bundle_path
    ));

    logging::log("[input_monitoring] requesting listen access");

    if let Some(granted) = cg_request_listen_event_access() {
        logging::log(&format!(
            "[input_monitoring] CGRequestListenEventAccess -> {}",
            granted
        ));
        if granted {
            return true;
        }
    } else {
        logging::log("[input_monitoring] CGRequestListenEventAccess unavailable");
    }

    if let Some(granted) = iohid_request_access(k_iohid_request_type_listen_event()) {
        logging::log(&format!(
            "[input_monitoring] IOHIDRequestAccess -> {}",
            granted
        ));
        if granted {
            return true;
        }
    } else {
        logging::log("[input_monitoring] IOHIDRequestAccess unavailable");
    }

    std::thread::spawn(|| {
        let probe_ok = probe_input_monitoring_event_tap();
        logging::log(&format!("[input_monitoring] probe event tap -> {}", probe_ok));

        let hid_ok = probe_input_monitoring_iohid_manager();
        logging::log(&format!("[input_monitoring] probe IOHIDManager -> {}", hid_ok));

        let granted = check_input_monitoring_permission();
        logging::log(&format!(
            "[input_monitoring] granted after probe -> {}",
            granted
        ));
    });

    false
}

fn probe_input_monitoring_event_tap() -> bool {
    let event_mask: ffi::CGEventMask =
        (1u64 << ffi::K_CG_EVENT_KEY_DOWN)
        | (1u64 << ffi::K_CG_EVENT_KEY_UP)
        | (1u64 << ffi::K_CG_EVENT_FLAGS_CHANGED);
    let tap = unsafe {
        ffi::CGEventTapCreate(
            ffi::K_CG_SESSION_EVENT_TAP,
            ffi::K_CG_HEAD_INSERT_EVENT_TAP,
            ffi::K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            event_mask,
            event_tap_probe_callback,
            std::ptr::null_mut(),
        )
    };

    if tap.is_null() {
        return false;
    }

    let source = unsafe { ffi::CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0) };
    if source.is_null() {
        unsafe {
            ffi::CFMachPortInvalidate(tap);
            ffi::CFRelease(tap as *const std::ffi::c_void);
        }
        return false;
    }

    let run_loop = CFRunLoop::get_current();
    unsafe {
        ffi::CFRunLoopAddSource(
            run_loop.as_concrete_TypeRef() as *const _,
            source,
            kCFRunLoopDefaultMode as *const _,
        );
        ffi::CGEventTapEnable(tap, true);
    }

    let mut granted = check_input_monitoring_permission();
    for _ in 0..100 {
        unsafe {
            ffi::CFRunLoopRunInMode(kCFRunLoopDefaultMode as *const _, 0.1, true);
        }
        granted = check_input_monitoring_permission();
        if granted {
            break;
        }
    }

    unsafe {
        ffi::CGEventTapEnable(tap, false);
        ffi::CFRunLoopRemoveSource(
            run_loop.as_concrete_TypeRef() as *const _,
            source,
            kCFRunLoopDefaultMode as *const _,
        );
        ffi::CFRelease(source as *const std::ffi::c_void);
        ffi::CFMachPortInvalidate(tap);
        ffi::CFRelease(tap as *const std::ffi::c_void);
    }

    granted
}

fn probe_input_monitoring_iohid_manager() -> bool {
    let manager = unsafe { hid::IOHIDManagerCreate(std::ptr::null(), 0) };
    if manager.is_null() {
        return false;
    }

    unsafe {
        hid::IOHIDManagerSetDeviceMatching(manager, std::ptr::null());
    }

    let run_loop = CFRunLoop::get_current();
    unsafe {
        hid::IOHIDManagerScheduleWithRunLoop(
            manager,
            run_loop.as_concrete_TypeRef() as *const _,
            kCFRunLoopDefaultMode as *const _,
        );
    }

    let open_result = unsafe { hid::IOHIDManagerOpen(manager, 0) };
    if open_result == hid::K_IO_RETURN_SUCCESS {
        for _ in 0..30 {
            unsafe {
                ffi::CFRunLoopRunInMode(kCFRunLoopDefaultMode as *const _, 0.1, true);
            }
            if check_input_monitoring_permission() {
                break;
            }
        }
    }

    unsafe {
        hid::IOHIDManagerUnscheduleFromRunLoop(
            manager,
            run_loop.as_concrete_TypeRef() as *const _,
            kCFRunLoopDefaultMode as *const _,
        );
    }

    let close_result = unsafe { hid::IOHIDManagerClose(manager, 0) };
    unsafe {
        ffi::CFRelease(manager as *const std::ffi::c_void);
    }

    logging::log(&format!(
        "[input_monitoring] IOHIDManagerOpen -> {}",
        open_result
    ));
    logging::log(&format!(
        "[input_monitoring] IOHIDManagerClose -> {}",
        close_result
    ));

    open_result == hid::K_IO_RETURN_SUCCESS
}

type CGPreflightListenEventAccessFn = unsafe extern "C" fn() -> bool;
type CGRequestListenEventAccessFn = unsafe extern "C" fn() -> bool;

type IOHIDRequestType = i32;
type IOHIDAccessType = i32;

type IOHIDCheckAccessFn = unsafe extern "C" fn(IOHIDRequestType) -> IOHIDAccessType;
type IOHIDRequestAccessFn = unsafe extern "C" fn(IOHIDRequestType) -> bool;

fn cg_preflight_listen_event_access() -> Option<bool> {
    let symbol = resolve_symbol("CGPreflightListenEventAccess")?;
    let func: CGPreflightListenEventAccessFn = unsafe { std::mem::transmute(symbol) };
    Some(unsafe { func() })
}

fn cg_request_listen_event_access() -> Option<bool> {
    let symbol = resolve_symbol("CGRequestListenEventAccess")?;
    let func: CGRequestListenEventAccessFn = unsafe { std::mem::transmute(symbol) };
    Some(unsafe { func() })
}

fn k_iohid_request_type_listen_event() -> IOHIDRequestType {
    1
}

fn k_iohid_access_type_granted() -> IOHIDAccessType {
    0
}

fn iohid_check_access(request_type: IOHIDRequestType) -> Option<IOHIDAccessType> {
    let symbol = resolve_symbol("IOHIDCheckAccess")?;
    let func: IOHIDCheckAccessFn = unsafe { std::mem::transmute(symbol) };
    Some(unsafe { func(request_type) })
}

fn iohid_request_access(request_type: IOHIDRequestType) -> Option<bool> {
    let symbol = resolve_symbol("IOHIDRequestAccess")?;
    let func: IOHIDRequestAccessFn = unsafe { std::mem::transmute(symbol) };
    Some(unsafe { func(request_type) })
}

fn resolve_symbol(name: &str) -> Option<*mut std::ffi::c_void> {
    let c_name = CString::new(name).ok()?;
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr() as *const libc::c_char) };
    if symbol.is_null() {
        None
    } else {
        Some(symbol)
    }
}

struct CallbackState {
    fn_down: bool,
    shift_down: bool,
    is_dictating: bool,
    tx: Sender<GlobeKeyEvent>,
}

extern "C" fn event_tap_callback(
    _proxy: ffi::CGEventTapProxy,
    event_type: ffi::CGEventType,
    event: ffi::CGEventRef,
    _user_info: *mut std::ffi::c_void,
) -> ffi::CGEventRef {
    // Re-enable tap if disabled
    if event_type == ffi::K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || event_type == ffi::K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        if event_type == ffi::K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT {
            DISABLED_TIMEOUT_COUNT.fetch_add(1, Ordering::Relaxed);
        } else {
            DISABLED_USER_INPUT_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        let tap = EVENT_TAP.load(Ordering::SeqCst);
        if !tap.is_null() {
            unsafe { ffi::CGEventTapEnable(tap as ffi::CFMachPortRef, true); }
        }
        return event;
    }

    if event_type != ffi::K_CG_EVENT_FLAGS_CHANGED {
        return event;
    }

    unsafe {
        let mut state_guard = callback_state().lock().unwrap();
        if let Some(state) = state_guard.as_mut() {
            let flags = ffi::CGEventGetFlags(event);
            let keycode = ffi::CGEventGetIntegerValueField(event, ffi::K_CG_KEYBOARD_EVENT_KEYCODE);
            FLAGS_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
            LAST_FLAGS_RAW.store(flags, Ordering::Relaxed);
            LAST_KEYCODE.store(keycode as u64, Ordering::Relaxed);

            let fn_down = (flags & ffi::K_CG_EVENT_FLAG_MASK_SECONDARY_FN) != 0;
            let shift_down = (flags & ffi::K_CG_EVENT_FLAG_MASK_SHIFT) != 0;

            state.fn_down = fn_down;
            state.shift_down = shift_down;

            let should_dictate = fn_down && shift_down;
            let was_dictating = state.is_dictating;

            if should_dictate && !was_dictating {
                state.is_dictating = true;
                let _ = state.tx.send(GlobeKeyEvent::DictateStart);
            } else if !should_dictate && was_dictating {
                state.is_dictating = false;
                let _ = state.tx.send(GlobeKeyEvent::DictateStop);
            }
        }
    }

    event
}

extern "C" fn event_tap_probe_callback(
    _proxy: ffi::CGEventTapProxy,
    _event_type: ffi::CGEventType,
    event: ffi::CGEventRef,
    _user_info: *mut std::ffi::c_void,
) -> ffi::CGEventRef {
    event
}

fn run_event_tap(tx: Sender<GlobeKeyEvent>, stop_flag: Arc<AtomicBool>) {
    logging::log("[globe_key] Starting native CGEventTap...");

    // Initialize global callback state
    {
        let mut state_guard = callback_state().lock().unwrap();
        *state_guard = Some(CallbackState {
            fn_down: false,
            shift_down: false,
            is_dictating: false,
            tx: tx.clone(),
        });
    }

    // Event mask for flags changed
    let event_mask: ffi::CGEventMask =
        (1u64 << ffi::K_CG_EVENT_FLAGS_CHANGED)
        | (1u64 << ffi::K_CG_EVENT_KEY_DOWN)
        | (1u64 << ffi::K_CG_EVENT_KEY_UP);

    // Create event tap
    let tap = unsafe {
        ffi::CGEventTapCreate(
            ffi::K_CG_SESSION_EVENT_TAP,
            ffi::K_CG_HEAD_INSERT_EVENT_TAP,
            ffi::K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            event_mask,
            event_tap_callback,
            std::ptr::null_mut(),
        )
    };

    if tap.is_null() {
        logging::log(
            "[globe_key] ERROR: Failed to create CGEventTap - Input Monitoring permission required",
        );
        return;
    }

    EVENT_TAP.store(tap as *mut std::ffi::c_void, Ordering::SeqCst);
    logging::log(&format!("[globe_key] CGEventTap created: {:p}", tap));

    // Enable the tap
    unsafe {
        ffi::CGEventTapEnable(tap, true);
    }

    // Create run loop source
    let source = unsafe { ffi::CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0) };

    if source.is_null() {
        logging::log("[globe_key] ERROR: Failed to create run loop source");
        return;
    }

    // Add to current run loop
    let run_loop = CFRunLoop::get_current();
    unsafe {
        ffi::CFRunLoopAddSource(
            run_loop.as_concrete_TypeRef() as *const _,
            source,
            kCFRunLoopDefaultMode as *const _,
        );
    }

    // Signal ready
    let _ = tx.send(GlobeKeyEvent::Ready);
    logging::log("[globe_key] Native CGEventTap ready, listening for Fn+Shift...");

    // Run the event loop
    while !stop_flag.load(Ordering::SeqCst) {
        unsafe {
            ffi::CFRunLoopRunInMode(kCFRunLoopDefaultMode as *const _, 0.1, true);
        }
    }

    // Cleanup
    {
        let mut state_guard = callback_state().lock().unwrap();
        *state_guard = None;
    }
    EVENT_TAP.store(std::ptr::null_mut(), Ordering::SeqCst);

    logging::log("[globe_key] CGEventTap stopped");
}
