use crate::logging;
use core_foundation::base::TCFType;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;

// Raw FFI bindings to Core Graphics
mod ffi {
    use std::ffi::c_void;

    pub type CGEventRef = *mut c_void;
    pub type CGEventTapProxy = *mut c_void;
    pub type CFMachPortRef = *mut c_void;
    pub type CFRunLoopSourceRef = *mut c_void;

    pub type CGEventType = u32;
    pub const kCGEventFlagsChanged: CGEventType = 12;
    pub const kCGEventTapDisabledByTimeout: CGEventType = 0xFFFFFFFE;
    pub const kCGEventTapDisabledByUserInput: CGEventType = 0xFFFFFFFF;

    pub type CGEventFlags = u64;
    pub const kCGEventFlagMaskSecondaryFn: CGEventFlags = 0x00800000;
    pub const kCGEventFlagMaskShift: CGEventFlags = 0x00020000;

    pub type CGEventTapLocation = u32;
    pub const kCGSessionEventTap: CGEventTapLocation = 1;

    pub type CGEventTapPlacement = u32;
    pub const kCGHeadInsertEventTap: CGEventTapPlacement = 0;

    pub type CGEventTapOptions = u32;
    pub const kCGEventTapOptionListenOnly: CGEventTapOptions = 1;

    pub type CGEventMask = u64;

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

        pub fn CFRunLoopRunInMode(
            mode: *const c_void,
            seconds: f64,
            return_after_source_handled: bool,
        ) -> i32;
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
static mut CALLBACK_STATE: Option<CallbackState> = None;

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
    if event_type == ffi::kCGEventTapDisabledByTimeout
        || event_type == ffi::kCGEventTapDisabledByUserInput
    {
        return event;
    }

    if event_type != ffi::kCGEventFlagsChanged {
        return event;
    }

    unsafe {
        if let Some(state) = CALLBACK_STATE.as_mut() {
            let flags = ffi::CGEventGetFlags(event);

            let fn_down = (flags & ffi::kCGEventFlagMaskSecondaryFn) != 0;
            let shift_down = (flags & ffi::kCGEventFlagMaskShift) != 0;

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

fn run_event_tap(tx: Sender<GlobeKeyEvent>, stop_flag: Arc<AtomicBool>) {
    logging::log("[globe_key] Starting native CGEventTap...");

    // Initialize global callback state
    unsafe {
        CALLBACK_STATE = Some(CallbackState {
            fn_down: false,
            shift_down: false,
            is_dictating: false,
            tx: tx.clone(),
        });
    }

    // Event mask for flags changed
    let event_mask: ffi::CGEventMask = 1 << ffi::kCGEventFlagsChanged;

    // Create event tap
    let tap = unsafe {
        ffi::CGEventTapCreate(
            ffi::kCGSessionEventTap,
            ffi::kCGHeadInsertEventTap,
            ffi::kCGEventTapOptionListenOnly,
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
    unsafe {
        CALLBACK_STATE = None;
    }

    logging::log("[globe_key] CGEventTap stopped");
}
