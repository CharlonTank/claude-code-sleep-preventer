use objc::{class, msg_send, sel, sel_impl};
use objc::runtime::NO;

use crate::objc_utils::{nsstring, CGFloat, Id, NSPoint, NSRect, NSSize};

pub unsafe fn create_toggle_switch(
    enabled: bool,
    title: &str,
    x: CGFloat,
    y: CGFloat,
    width: CGFloat,
    height: CGFloat,
) -> Id {
    let toggle: Id = msg_send![class!(NSButton), alloc];
    let toggle: Id = msg_send![
        toggle,
        initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    ];

    let _: () = msg_send![toggle, setButtonType: 3i64]; // NSButtonTypeSwitch
    let _: () = msg_send![toggle, setState: if enabled { 1i64 } else { 0i64 }];
    let _: () = msg_send![toggle, setTitle: nsstring(title)];

    toggle
}

pub unsafe fn create_label(
    text: &str,
    x: CGFloat,
    y: CGFloat,
    width: CGFloat,
    height: CGFloat,
    bold: bool,
) -> Id {
    let label: Id = msg_send![class!(NSTextField), alloc];
    let label: Id = msg_send![
        label,
        initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    ];

    let _: () = msg_send![label, setStringValue: nsstring(text)];
    let _: () = msg_send![label, setBezeled: NO];
    let _: () = msg_send![label, setDrawsBackground: NO];
    let _: () = msg_send![label, setEditable: NO];
    let _: () = msg_send![label, setSelectable: NO];

    if bold {
        let font: Id = msg_send![class!(NSFont), boldSystemFontOfSize: 14.0 as CGFloat];
        let _: () = msg_send![label, setFont: font];
    }

    label
}

pub unsafe fn create_button(
    title: &str,
    x: CGFloat,
    y: CGFloat,
    width: CGFloat,
    height: CGFloat,
) -> Id {
    let button: Id = msg_send![class!(NSButton), alloc];
    let button: Id = msg_send![
        button,
        initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    ];

    let _: () = msg_send![button, setTitle: nsstring(title)];
    let _: () = msg_send![button, setBezelStyle: 1i64]; // NSBezelStyleRounded

    button
}

pub unsafe fn create_separator(x: CGFloat, y: CGFloat, width: CGFloat) -> Id {
    let separator: Id = msg_send![class!(NSBox), alloc];
    let separator: Id = msg_send![
        separator,
        initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(width, 1.0))
    ];

    let _: () = msg_send![separator, setBoxType: 1i64]; // NSBoxSeparator

    separator
}
