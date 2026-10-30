use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

type Action = Box<dyn Fn() + Send + Sync>;

static ON: OnceLock<Action> = OnceLock::new();
static READY: AtomicBool = AtomicBool::new(false);

pub const NAME: &str = "Ctrl+Alt+End";
#[cfg(windows)]
const RETRY: std::time::Duration = std::time::Duration::from_secs(5);

pub fn start(action: impl Fn() + Send + Sync + 'static) {
    if ON.set(Box::new(action)).is_ok() {
        let _ = thread::Builder::new().name("hotkey".into()).spawn(listen);
    }
}

pub fn label() -> &'static str {
    if READY.load(Ordering::SeqCst) { NAME } else { "unavailable" }
}

fn fire() {
    if let Some(action) = ON.get() {
        action();
    }
}

#[cfg(windows)]
fn listen() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, VK_END};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};
    let mut told = false;
    while let Err(e) = unsafe { RegisterHotKey(None, 1, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, u32::from(VK_END.0)) } {
        if !told {
            eprintln!("lighting daemon: {NAME} not free yet ({e}), trying again every {RETRY:?}");
            told = true;
        }
        thread::sleep(RETRY);
    }
    if told {
        eprintln!("lighting daemon: {NAME} registered");
    }
    READY.store(true, Ordering::SeqCst);
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if msg.message == WM_HOTKEY {
            fire();
        }
    }
    READY.store(false, Ordering::SeqCst);
}

#[cfg(target_os = "linux")]
fn listen() {
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
    const END: u32 = 0xff57;
    let Ok((conn, screen)) = x11rb::connect(None) else { return };
    let (root, min, max) = {
        let setup = conn.setup();
        (setup.roots[screen].root, setup.min_keycode, setup.max_keycode)
    };
    let Some(map) = conn.get_keyboard_mapping(min, max - min + 1).ok().and_then(|c| c.reply().ok()) else { return };
    let per = usize::from(map.keysyms_per_keycode.max(1));
    let Some(code) = map.keysyms.chunks(per).position(|k| k.contains(&END)).and_then(|i| u8::try_from(i).ok()) else {
        return;
    };
    let code = min + code;
    let mods = ModMask::CONTROL | ModMask::M1;
    for extra in [ModMask::from(0u16), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2] {
        let grabbed = conn.grab_key(true, root, mods | extra, code, GrabMode::ASYNC, GrabMode::ASYNC);
        if grabbed.ok().and_then(|c| c.check().ok()).is_none() {
            return;
        }
    }
    if conn.flush().is_err() {
        return;
    }
    READY.store(true, Ordering::SeqCst);
    loop {
        match conn.wait_for_event() {
            Ok(Event::KeyPress(e)) if e.detail == code => fire(),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    READY.store(false, Ordering::SeqCst);
}

#[cfg(target_os = "macos")]
fn listen() {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::atomic::AtomicPtr;
    type Ref = *mut c_void;
    type Callback = extern "C" fn(Ref, u32, Ref, Ref) -> Ref;
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: Callback, info: Ref) -> Ref;
        fn CGEventTapEnable(tap: Ref, enable: bool);
        fn CGEventGetIntegerValueField(event: Ref, field: u32) -> i64;
        fn CGEventGetFlags(event: Ref) -> u64;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopCommonModes: Ref;
        fn CFMachPortCreateRunLoopSource(alloc: Ref, port: Ref, order: isize) -> Ref;
        fn CFRunLoopGetCurrent() -> Ref;
        fn CFRunLoopAddSource(run_loop: Ref, source: Ref, mode: Ref);
        fn CFRunLoopRun();
    }
    const SESSION_TAP: u32 = 1;
    const HEAD_INSERT: u32 = 0;
    const LISTEN_ONLY: u32 = 1;
    const KEY_DOWN: u32 = 10;
    const KEYCODE_FIELD: u32 = 9;
    const END_KEY: i64 = 0x77;
    const CTRL_ALT: u64 = (1 << 18) | (1 << 19);
    const DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    const DISABLED_BY_INPUT: u32 = 0xFFFF_FFFF;
    static TAP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    extern "C" fn on_event(_: Ref, kind: u32, event: Ref, _: Ref) -> Ref {
        if kind == DISABLED_BY_TIMEOUT || kind == DISABLED_BY_INPUT {
            unsafe { CGEventTapEnable(TAP.load(Ordering::SeqCst), true) };
        } else if kind == KEY_DOWN
            && unsafe { CGEventGetIntegerValueField(event, KEYCODE_FIELD) } == END_KEY
            && unsafe { CGEventGetFlags(event) } & CTRL_ALT == CTRL_ALT
        {
            fire();
        }
        event
    }
    let tap =
        unsafe { CGEventTapCreate(SESSION_TAP, HEAD_INSERT, LISTEN_ONLY, 1 << KEY_DOWN, on_event, ptr::null_mut()) };
    if tap.is_null() {
        return;
    }
    TAP.store(tap, Ordering::SeqCst);
    unsafe {
        let source = CFMachPortCreateRunLoopSource(ptr::null_mut(), tap, 0);
        if source.is_null() {
            return;
        }
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    READY.store(true, Ordering::SeqCst);
    unsafe { CFRunLoopRun() };
    READY.store(false, Ordering::SeqCst);
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn listen() {}
