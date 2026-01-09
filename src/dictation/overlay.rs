use objc::{class, msg_send, sel, sel_impl};
use objc::runtime::{BOOL, YES};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::logging;
use crate::objc_utils::{
    CGFloat, Id, NSPoint, NSRect, NSSize, NIL, NS_BACKING_STORE_BUFFERED,
    NS_WINDOW_COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES,
    NS_WINDOW_COLLECTION_BEHAVIOR_IGNORES_CYCLE,
    NS_WINDOW_COLLECTION_BEHAVIOR_STATIONARY, NS_WINDOW_STYLE_MASK_BORDERLESS,
};

static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayMode {
    Recording,
    Transcribing,
}

pub struct RecordingOverlay {
    window: Option<Id>,
    mode: OverlayMode,
}

impl RecordingOverlay {
    pub fn new() -> Self {
        Self {
            window: None,
            mode: OverlayMode::Recording,
        }
    }

    pub fn show(&mut self) {
        self.show_with_mode(OverlayMode::Recording);
    }

    pub fn show_with_mode(&mut self, mode: OverlayMode) {
        self.mode = mode;

        if let Some(window) = self.window {
            unsafe {
                let color = self.color_for_mode(mode);
                let _: () = msg_send![window, setBackgroundColor: color];
            }
            logging::log(&format!("[overlay] Updated color: {:?}", mode));
            return;
        }

        unsafe {
            let screen: Id = msg_send![class!(NSScreen), mainScreen];
            if screen.is_null() {
                logging::log("[overlay] ERROR: NSScreen::mainScreen returned nil");
                return;
            }
            let screen_frame: NSRect = msg_send![screen, frame];

            let bar_height: CGFloat = 6.0;
            let frame = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(screen_frame.size.width, bar_height),
            );

            let window: Id = msg_send![class!(NSWindow), alloc];
            let window: Id = msg_send![
                window,
                initWithContentRect: frame
                styleMask: NS_WINDOW_STYLE_MASK_BORDERLESS
                backing: NS_BACKING_STORE_BUFFERED
                defer: false as BOOL
            ];

            if window.is_null() {
                logging::log("[overlay] ERROR: Failed to create NSWindow");
                return;
            }

            let _: () = msg_send![window, setLevel: 25i64];
            let _: () = msg_send![window, setOpaque: false as BOOL];
            let _: () = msg_send![window, setHasShadow: false as BOOL];
            let _: () = msg_send![window, setIgnoresMouseEvents: YES];

            let behavior = NS_WINDOW_COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES
                | NS_WINDOW_COLLECTION_BEHAVIOR_STATIONARY
                | NS_WINDOW_COLLECTION_BEHAVIOR_IGNORES_CYCLE;
            let _: () = msg_send![window, setCollectionBehavior: behavior];

            let color = self.color_for_mode(mode);
            let _: () = msg_send![window, setBackgroundColor: color];

            let _: () = msg_send![window, makeKeyAndOrderFront: NIL];

            self.window = Some(window);
            OVERLAY_VISIBLE.store(true, Ordering::SeqCst);
            logging::log(&format!("[overlay] Shown: {:?}", mode));
        }
    }

    fn color_for_mode(&self, mode: OverlayMode) -> Id {
        unsafe {
            match mode {
                OverlayMode::Recording => msg_send![
                    class!(NSColor),
                    colorWithRed: 0.9
                    green: 0.2
                    blue: 0.2
                    alpha: 0.95
                ],
                OverlayMode::Transcribing => msg_send![
                    class!(NSColor),
                    colorWithRed: 1.0
                    green: 0.6
                    blue: 0.0
                    alpha: 0.95
                ],
            }
        }
    }

    pub fn set_mode(&mut self, mode: OverlayMode) {
        if self.window.is_some() {
            self.show_with_mode(mode);
        }
    }

    pub fn hide(&mut self) {
        if let Some(window) = self.window.take() {
            unsafe {
                let _: () = msg_send![window, orderOut: NIL];
                let _: () = msg_send![window, close];
            }
            logging::log("[overlay] Hidden");
        }
        OVERLAY_VISIBLE.store(false, Ordering::SeqCst);
    }
}

impl Drop for RecordingOverlay {
    fn drop(&mut self) {
        self.hide();
    }
}
