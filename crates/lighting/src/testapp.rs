use std::sync::Mutex;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowTextW,
    GetWindowThreadProcessId, HMENU, HWND_BOTTOM, IDC_ARROW, IsDialogMessageW, KillTimer, LoadCursorW, MSG,
    PostQuitMessage, RegisterClassW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_COMMAND, WM_DESTROY, WM_SETFONT, WM_TIMER, WNDCLASSW,
};
use windows::core::{HSTRING, PCWSTR};

pub const APP_TITLE: &str = "Lighting Test App";
pub const CHAT_TITLE: &str = "Lighting Test Chat";
const EX_APP: u32 = 0x0804_0000;
const STYLE_APP: u32 = 0x00CF_0000;
const CHILD: u32 = 0x5000_0000;
const EX_EDGE: u32 = 0x200;
const TAB_STOP: u32 = 0x0001_0000;
const ES_AUTOHSCROLL: u32 = 0x80;
const BS_AUTOCHECKBOX: u32 = 0x3;
const LBS_NOTIFY_VSCROLL: u32 = 0x0020_0001;
const LB_ADDSTRING: u32 = 0x0180;
const ID_GO: usize = 101;
const ID_FRONT: usize = 105;
const ID_SEND: usize = 1;
const PEER_TIMER: usize = 7;
const PEER_MS: u32 = 300;
const ROOMS: &[&str] = &["Friends, 3 unread messages", "2 mentions General", "Unread messages, News", "Quiet room"];
const TABS: &[&str] = &["One", "Two", "Three"];
const TCM_INSERTITEMW: u32 = 0x133E;
const TCIF_TEXT: u32 = 0x1;

#[repr(C)]
struct TabItem {
    mask: u32,
    state: u32,
    state_mask: u32,
    text: *mut u16,
    max: i32,
    image: i32,
    param: isize,
}

struct State {
    clicks: u32,
    edit: isize,
    status: isize,
    list: isize,
    sent: usize,
}

static STATE: Mutex<State> = Mutex::new(State { clicks: 0, edit: 0, status: 0, list: 0, sent: 0 });

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn hwnd(h: isize) -> HWND {
    HWND(h as *mut _)
}

fn text_of(h: isize) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd(h), &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0)])
}

fn set_text(h: isize, text: &str) {
    let _ = unsafe { SetWindowTextW(hwnd(h), &HSTRING::from(text)) };
}

fn add_line(list: isize, text: &str) {
    let wide = HSTRING::from(text);
    unsafe { SendMessageW(hwnd(list), LB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(wide.as_ptr() as isize))) };
}

unsafe extern "system" fn app_proc(w: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COMMAND && wp.0 & 0xFFFF == ID_GO {
        let mut s = state();
        s.clicks += 1;
        let name = text_of(s.edit);
        let shown = if name.is_empty() { "nothing".to_string() } else { name };
        set_text(s.status, &format!("Status: clicked {} with {shown}", s.clicks));
        return LRESULT(0);
    }
    if msg == WM_COMMAND && wp.0 & 0xFFFF == ID_FRONT {
        jump(w);
        return LRESULT(0);
    }
    if msg == WM_DESTROY {
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(w, msg, wp, lp) }
}

fn jump(w: HWND) {
    let me = unsafe { GetCurrentThreadId() };
    let them = unsafe { GetWindowThreadProcessId(GetForegroundWindow(), None) };
    let attach = them != 0 && them != me;
    if attach {
        let _ = unsafe { AttachThreadInput(me, them, true) };
    }
    let _ = unsafe { SetForegroundWindow(w) };
    if attach {
        let _ = unsafe { AttachThreadInput(me, them, false) };
    }
}

unsafe extern "system" fn chat_proc(w: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COMMAND && wp.0 & 0xFFFF == ID_SEND {
        let mut s = state();
        let text = text_of(s.edit);
        if !text.trim().is_empty() {
            add_line(s.list, &format!("me: {text}"));
            s.sent = text.chars().count();
            set_text(s.edit, "");
            unsafe { SetTimer(Some(w), PEER_TIMER, PEER_MS, None) };
        }
        return LRESULT(0);
    }
    if msg == WM_TIMER {
        let _ = unsafe { KillTimer(Some(w), PEER_TIMER) };
        let s = state();
        add_line(s.list, &format!("peer: ok, {} chars", s.sent));
        return LRESULT(0);
    }
    if msg == WM_DESTROY {
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(w, msg, wp, lp) }
}

struct Maker {
    inst: HINSTANCE,
    parent: HWND,
}

impl Maker {
    fn make(&self, class: &str, text: &str, style: u32, (x, y, w, h): (i32, i32, i32, i32), id: usize) -> isize {
        let ex = if class == "EDIT" || class == "LISTBOX" { EX_EDGE } else { 0 };
        let made = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(ex),
                &HSTRING::from(class),
                &HSTRING::from(text),
                WINDOW_STYLE(CHILD | style),
                x,
                y,
                w,
                h,
                Some(self.parent),
                Some(HMENU(id as *mut _)),
                Some(self.inst),
                None,
            )
        };
        let Ok(child) = made else { return 0 };
        let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        unsafe { SendMessageW(child, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1))) };
        child.0 as isize
    }
}

fn window(
    class: &str,
    title: &str,
    proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC,
    rect: (i32, i32, i32, i32),
) -> Option<Maker> {
    let inst: HINSTANCE = unsafe { GetModuleHandleW(None) }.ok()?.into();
    let name = HSTRING::from(class);
    let wc = WNDCLASSW {
        lpfnWndProc: proc,
        hInstance: inst,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.ok()?,
        hbrBackground: HBRUSH(16 as *mut _),
        lpszClassName: PCWSTR(name.as_ptr()),
        ..Default::default()
    };
    unsafe { RegisterClassW(&raw const wc) };
    let parent = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(EX_APP),
            &name,
            &HSTRING::from(title),
            WINDOW_STYLE(STYLE_APP),
            rect.0,
            rect.1,
            rect.2,
            rect.3,
            None,
            None,
            Some(inst),
            None,
        )
    }
    .ok()?;
    Some(Maker { inst, parent })
}

fn show_and_loop(parent: HWND) {
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW;
    let _ = unsafe { SetWindowPos(parent, Some(HWND_BOTTOM), 0, 0, 0, 0, flags) };
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.0 > 0 {
        if !unsafe { IsDialogMessageW(parent, &raw const msg) }.as_bool() {
            let _ = unsafe { TranslateMessage(&raw const msg) };
            unsafe { DispatchMessageW(&raw const msg) };
        }
    }
}

fn app() {
    let Some(m) = window("LightingTestApp", APP_TITLE, Some(app_proc), (200, 200, 460, 260)) else { return };
    m.make("STATIC", "Name:", 0, (20, 24, 60, 22), 0);
    let edit = m.make("EDIT", "", TAB_STOP | ES_AUTOHSCROLL, (90, 20, 320, 26), 100);
    m.make("BUTTON", "Go", TAB_STOP, (90, 64, 100, 32), ID_GO);
    m.make("BUTTON", "Remember me", TAB_STOP | BS_AUTOCHECKBOX, (210, 64, 120, 32), 102);
    m.make("BUTTON", "Front", TAB_STOP, (340, 64, 80, 32), ID_FRONT);
    let status = m.make("STATIC", "Status: ready", 0, (20, 120, 400, 24), 103);
    let _ = unsafe { LoadLibraryW(&HSTRING::from("comctl32.dll")) };
    let tabs = m.make("SysTabControl32", "", TAB_STOP, (20, 150, 400, 30), 104);
    for (i, name) in TABS.iter().enumerate() {
        let text = HSTRING::from(*name);
        let item = TabItem {
            mask: TCIF_TEXT,
            state: 0,
            state_mask: 0,
            text: text.as_ptr().cast_mut(),
            max: 0,
            image: -1,
            param: 0,
        };
        unsafe { SendMessageW(hwnd(tabs), TCM_INSERTITEMW, Some(WPARAM(i)), Some(LPARAM(&raw const item as isize))) };
    }
    {
        let mut s = state();
        s.edit = edit;
        s.status = status;
    }
    show_and_loop(m.parent);
}

fn chat() {
    let Some(m) = window("LightingTestChat", CHAT_TITLE, Some(chat_proc), (260, 240, 640, 360)) else { return };
    m.make("STATIC", "Nachrichten in Test", 0, (16, 10, 300, 20), 0);
    let list = m.make("LISTBOX", "", LBS_NOTIFY_VSCROLL | TAB_STOP, (16, 32, 370, 200), 200);
    m.make("STATIC", "Nachricht an Test:", 0, (16, 244, 300, 20), 0);
    let edit = m.make("EDIT", "", TAB_STOP | ES_AUTOHSCROLL, (16, 266, 370, 26), 201);
    add_line(list, "peer: hi, welcome");
    m.make("STATIC", "Kanaele", 0, (400, 10, 200, 20), 0);
    let rooms = m.make("LISTBOX", "", LBS_NOTIFY_VSCROLL, (400, 32, 200, 200), 210);
    for room in ROOMS {
        add_line(rooms, room);
    }
    {
        let mut s = state();
        s.edit = edit;
        s.list = list;
    }
    show_and_loop(m.parent);
}

pub fn main(args: &[String]) {
    if args.get(1).is_some_and(|a| a == "chat") { chat() } else { app() }
}
