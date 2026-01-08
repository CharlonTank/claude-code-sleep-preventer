#[allow(deprecated)]
use cocoa::appkit::{
    NSBackingStoreType, NSColor, NSScreen, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
#[allow(deprecated)]
use cocoa::base::{id, nil, YES};
#[allow(deprecated)]
use cocoa::foundation::{NSPoint, NSRect, NSSize};
use objc::msg_send;
use objc::runtime::BOOL;
use objc::sel;
use objc::sel_impl;
use std::sync::atomic::{AtomicBool, Ordering};

static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq)]
pub enum OverlayMode {
    Recording,    // Red - recording audio
    Transcribing, // Orange - processing
}

pub struct RecordingOverlay {
    window: Option<id>,
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

        // If window exists, just update color
        if let Some(window) = self.window {
            unsafe {
                let color = self.color_for_mode(mode);
                let _: () = msg_send![window, setBackgroundColor: color];
            }
            return;
        }

        unsafe {
            // Get screen dimensions
            let screen: id = NSScreen::mainScreen(nil);
            if screen == nil {
                return;
            }
            let screen_frame = NSScreen::frame(screen);

            // Bar dimensions: full width, 6 pixels high at bottom
            let bar_height = 6.0;
            let frame = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(screen_frame.size.width, bar_height),
            );

            // Create borderless window
            let window: id = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
                frame,
                NSWindowStyleMask::NSBorderlessWindowMask,
                NSBackingStoreType::NSBackingStoreBuffered,
                false as BOOL,
            );

            if window == nil {
                return;
            }

            // Configure window behavior
            let _: () = msg_send![window, setLevel: 25i64]; // NSStatusWindowLevel + 1
            let _: () = msg_send![window, setOpaque: false as BOOL];
            let _: () = msg_send![window, setHasShadow: false as BOOL];
            let _: () = msg_send![window, setIgnoresMouseEvents: YES];

            // Appear on all spaces
            window.setCollectionBehavior_(
                NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                    | NSWindowCollectionBehavior::NSWindowCollectionBehaviorStationary
                    | NSWindowCollectionBehavior::NSWindowCollectionBehaviorIgnoresCycle,
            );

            // Set background color based on mode
            let color = self.color_for_mode(mode);
            window.setBackgroundColor_(color);

            // Show window
            let _: () = msg_send![window, makeKeyAndOrderFront: nil];

            self.window = Some(window);
            OVERLAY_VISIBLE.store(true, Ordering::SeqCst);
        }
    }

    fn color_for_mode(&self, mode: OverlayMode) -> id {
        unsafe {
            match mode {
                OverlayMode::Recording => {
                    // Red for recording
                    NSColor::colorWithRed_green_blue_alpha_(nil, 0.9, 0.2, 0.2, 0.95)
                }
                OverlayMode::Transcribing => {
                    // Orange for transcribing
                    NSColor::colorWithRed_green_blue_alpha_(nil, 1.0, 0.6, 0.0, 0.95)
                }
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
                let _: () = msg_send![window, orderOut: nil];
                let _: () = msg_send![window, close];
            }
        }
        OVERLAY_VISIBLE.store(false, Ordering::SeqCst);
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_some()
    }
}

impl Drop for RecordingOverlay {
    fn drop(&mut self) {
        self.hide();
    }
}

pub fn is_overlay_visible() -> bool {
    OVERLAY_VISIBLE.load(Ordering::SeqCst)
}
