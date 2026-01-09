use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl, Encode, Encoding};
use std::ffi::CString;

pub type Id = *mut Object;

#[cfg(target_pointer_width = "64")]
pub type CGFloat = f64;
#[cfg(target_pointer_width = "32")]
pub type CGFloat = f32;

pub const NIL: Id = std::ptr::null_mut();

pub type NSWindowStyleMask = usize;
pub const NS_WINDOW_STYLE_MASK_BORDERLESS: NSWindowStyleMask = 0;
pub const NS_WINDOW_STYLE_MASK_TITLED: NSWindowStyleMask = 1;

pub type NSBackingStoreType = usize;
pub const NS_BACKING_STORE_BUFFERED: NSBackingStoreType = 2;

pub type NSWindowCollectionBehavior = usize;
pub const NS_WINDOW_COLLECTION_BEHAVIOR_CAN_JOIN_ALL_SPACES: NSWindowCollectionBehavior = 1 << 0;
pub const NS_WINDOW_COLLECTION_BEHAVIOR_STATIONARY: NSWindowCollectionBehavior = 1 << 4;
pub const NS_WINDOW_COLLECTION_BEHAVIOR_IGNORES_CYCLE: NSWindowCollectionBehavior = 1 << 8;

pub struct AutoreleasePool(Id);

impl AutoreleasePool {
    pub fn new() -> Self {
        unsafe {
            let pool: Id = msg_send![class!(NSAutoreleasePool), new];
            Self(pool)
        }
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                let _: () = msg_send![self.0, drain];
            }
        }
    }
}

pub fn nsstring(text: &str) -> Id {
    let cstr = CString::new(text).unwrap_or_else(|_| {
        CString::new(text.replace('\0', "")).expect("CString replacement failed")
    });
    unsafe { msg_send![class!(NSString), stringWithUTF8String: cstr.as_ptr()] }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

impl NSPoint {
    pub fn new(x: CGFloat, y: CGFloat) -> Self {
        Self { x, y }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSSize {
    pub width: CGFloat,
    pub height: CGFloat,
}

impl NSSize {
    pub fn new(width: CGFloat, height: CGFloat) -> Self {
        Self { width, height }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}

impl NSRect {
    pub fn new(origin: NSPoint, size: NSSize) -> Self {
        Self { origin, size }
    }
}

fn point_encoding() -> Encoding {
    #[cfg(target_pointer_width = "64")]
    {
        unsafe { Encoding::from_str("{CGPoint=dd}") }
    }
    #[cfg(target_pointer_width = "32")]
    {
        unsafe { Encoding::from_str("{CGPoint=ff}") }
    }
}

fn size_encoding() -> Encoding {
    #[cfg(target_pointer_width = "64")]
    {
        unsafe { Encoding::from_str("{CGSize=dd}") }
    }
    #[cfg(target_pointer_width = "32")]
    {
        unsafe { Encoding::from_str("{CGSize=ff}") }
    }
}

fn rect_encoding() -> Encoding {
    #[cfg(target_pointer_width = "64")]
    {
        unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
    #[cfg(target_pointer_width = "32")]
    {
        unsafe { Encoding::from_str("{CGRect={CGPoint=ff}{CGSize=ff}}") }
    }
}

unsafe impl Encode for NSPoint {
    fn encode() -> Encoding {
        point_encoding()
    }
}

unsafe impl Encode for NSSize {
    fn encode() -> Encoding {
        size_encoding()
    }
}

unsafe impl Encode for NSRect {
    fn encode() -> Encoding {
        rect_encoding()
    }
}
