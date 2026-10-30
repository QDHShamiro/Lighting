use std::ffi::c_void;
use std::ptr;
use std::slice;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GW_OWNER, GetClassNameW, GetForegroundWindow, GetSystemMetrics, GetWindow,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HWND_BOTTOM, IsIconic, IsWindow,
    IsWindowVisible, PostMessageW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_MINIMIZE, SW_RESTORE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow,
    SetWindowPos, ShowWindow, WM_CLOSE,
};
use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR, w};

use super::input;

const CF_UNICODETEXT: u32 = 13;
const BM_CLICK: u32 = 0x00F5;
const EM_SETSEL: u32 = 0x00B1;
const SHELL: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

pub type Rect = (i32, i32, i32, i32);

pub struct Win {
    pub hwnd: isize,
    pub title: String,
    pub cls: String,
    pub exe: String,
    pub min: bool,
    pub fg: bool,
}

pub fn h(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

fn raw(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

pub fn text_of(hwnd: isize) -> String {
    let n = unsafe { GetWindowTextLengthW(h(hwnd)) };
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = unsafe { GetWindowTextW(h(hwnd), &mut buf) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

pub fn class_of(hwnd: isize) -> String {
    let mut buf = [0u16; 256];
    let got = unsafe { GetClassNameW(h(hwnd), &mut buf) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

pub fn pid_of(hwnd: isize) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h(hwnd), Some(&raw mut pid)) };
    pid
}

pub fn thread_of(hwnd: isize) -> u32 {
    unsafe { GetWindowThreadProcessId(h(hwnd), None) }
}

pub fn exe_of(pid: u32) -> String {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return String::new();
    };
    let mut buf = [0u16; 1024];
    let mut size = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &raw mut size) }
        .is_ok();
    let _ = unsafe { CloseHandle(process) };
    if !ok {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..size as usize]);
    path.rsplit(['\\', '/']).next().unwrap_or_default().to_string()
}

pub fn exe_of_window(hwnd: isize) -> String {
    exe_of(pid_of(hwnd))
}

pub fn elevated(pid: u32) -> bool {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return true };
    let mut token = HANDLE::default();
    let mut out = true;
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) }.is_ok() {
        let mut val = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let got = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                Some((&raw mut val).cast()),
                size_of::<TOKEN_ELEVATION>() as u32,
                &raw mut size,
            )
        };
        out = got.is_ok() && val.TokenIsElevated != 0;
        let _ = unsafe { CloseHandle(token) };
    }
    let _ = unsafe { CloseHandle(process) };
    out
}

fn cloaked(hwnd: isize) -> bool {
    let mut val = 0i32;
    unsafe { DwmGetWindowAttribute(h(hwnd), DWMWA_CLOAKED, (&raw mut val).cast(), 4) }.is_ok() && val != 0
}

pub fn rect(hwnd: isize) -> Rect {
    let mut r = RECT::default();
    let size = size_of::<RECT>() as u32;
    if unsafe { DwmGetWindowAttribute(h(hwnd), DWMWA_EXTENDED_FRAME_BOUNDS, (&raw mut r).cast(), size) }.is_err() {
        let _ = unsafe { GetWindowRect(h(hwnd), &raw mut r) };
    }
    (r.left, r.top, r.right, r.bottom)
}

pub fn outer_rect(hwnd: isize) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h(hwnd), &raw mut r) }.ok()?;
    Some((r.left, r.top, r.right, r.bottom))
}

unsafe extern "system" fn gather(hwnd: HWND, list: LPARAM) -> BOOL {
    let list = unsafe { &mut *(list.0 as *mut Vec<isize>) };
    list.push(raw(hwnd));
    true.into()
}

pub fn all() -> Vec<isize> {
    let mut list: Vec<isize> = Vec::new();
    let _ = unsafe { EnumWindows(Some(gather), LPARAM((&raw mut list) as isize)) };
    list
}

pub fn windows() -> Vec<Win> {
    let fg = foreground();
    all()
        .into_iter()
        .filter_map(|hwnd| {
            if !visible(hwnd) || unsafe { GetWindow(h(hwnd), GW_OWNER) }.is_ok() {
                return None;
            }
            let title = text_of(hwnd);
            if title.is_empty() || cloaked(hwnd) {
                return None;
            }
            let cls = class_of(hwnd);
            if SHELL.contains(&cls.as_str()) {
                return None;
            }
            let (l, t, r, b) = rect(hwnd);
            if r - l < 40 || b - t < 30 {
                return None;
            }
            let pid = pid_of(hwnd);
            Some(Win { hwnd, title, cls, exe: exe_of(pid), min: minimized(hwnd), fg: hwnd == fg })
        })
        .collect()
}

pub fn alive(hwnd: isize) -> bool {
    hwnd != 0 && unsafe { IsWindow(Some(h(hwnd))) }.as_bool()
}

pub fn visible(hwnd: isize) -> bool {
    unsafe { IsWindowVisible(h(hwnd)) }.as_bool()
}

pub fn minimized(hwnd: isize) -> bool {
    unsafe { IsIconic(h(hwnd)) }.as_bool()
}

pub fn post(target: isize, msg: u32, wparam: usize, lparam: isize) {
    let _ = unsafe { PostMessageW(Some(h(target)), msg, WPARAM(wparam), LPARAM(lparam)) };
}

pub fn close(hwnd: isize) {
    post(hwnd, WM_CLOSE, 0, 0);
}

pub fn post_click(hwnd: isize) {
    post(hwnd, BM_CLICK, 0, 0);
}

pub fn select_all_in(hwnd: isize) {
    post(hwnd, EM_SETSEL, 0, -1);
}

pub fn foreground() -> isize {
    raw(unsafe { GetForegroundWindow() })
}

pub fn set_foreground(hwnd: isize) -> bool {
    if minimized(hwnd) {
        let _ = unsafe { ShowWindow(h(hwnd), SW_RESTORE) };
    }
    if foreground() == hwnd {
        return true;
    }
    let fg = foreground();
    let me = unsafe { GetCurrentThreadId() };
    let them = if fg == 0 { 0 } else { thread_of(fg) };
    let attach = them != 0 && them != me;
    if attach {
        let _ = unsafe { AttachThreadInput(me, them, true) };
    }
    input::key(input::ALT, false, false);
    input::key(input::ALT, true, false);
    let _ = unsafe { BringWindowToTop(h(hwnd)) };
    let _ = unsafe { SetForegroundWindow(h(hwnd)) };
    if attach {
        let _ = unsafe { AttachThreadInput(me, them, false) };
    }
    thread::sleep(Duration::from_millis(50));
    foreground() == hwnd
}

pub fn restore_cycle(hwnd: isize) {
    let _ = unsafe { ShowWindow(h(hwnd), SW_MINIMIZE) };
    thread::sleep(Duration::from_millis(50));
    let _ = unsafe { ShowWindow(h(hwnd), SW_RESTORE) };
}

pub fn put_behind(hwnd: isize, front: isize) {
    let after = if front == 0 { HWND_BOTTOM } else { h(front) };
    let _ = unsafe { SetWindowPos(h(hwnd), Some(after), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE) };
}

pub fn virtual_screen() -> Rect {
    let m = |i| unsafe { GetSystemMetrics(i) };
    let (x, y) = (m(SM_XVIRTUALSCREEN), m(SM_YVIRTUALSCREEN));
    (x, y, x + m(SM_CXVIRTUALSCREEN), y + m(SM_CYVIRTUALSCREEN))
}

fn open_clip() -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(30));
    }
    false
}

pub fn clip_get() -> Option<String> {
    if !open_clip() {
        return None;
    }
    let text = unsafe { GetClipboardData(CF_UNICODETEXT) }.ok().map_or_else(String::new, |handle| {
        let global = HGLOBAL(handle.0);
        let p = unsafe { GlobalLock(global) }.cast::<u16>();
        if p.is_null() {
            return String::new();
        }
        let len = (0..).take_while(|i| unsafe { *p.add(*i) } != 0).count();
        let text = String::from_utf16_lossy(unsafe { slice::from_raw_parts(p, len) });
        let _ = unsafe { GlobalUnlock(global) };
        text
    });
    let _ = unsafe { CloseClipboard() };
    Some(text)
}

pub fn clip_set(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    if !open_clip() {
        return false;
    }
    let _ = unsafe { EmptyClipboard() };
    if let Ok(global) = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, wide.len() * 2) } {
        let p = unsafe { GlobalLock(global) }.cast::<u16>();
        if !p.is_null() {
            unsafe { ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len()) };
            let _ = unsafe { GlobalUnlock(global) };
            let _ = unsafe { SetClipboardData(CF_UNICODETEXT, Some(HANDLE(global.0))) };
        }
    }
    let _ = unsafe { CloseClipboard() };
    true
}

pub fn open_target(target: &str) -> bool {
    let file = HSTRING::from(target);
    let done = unsafe { ShellExecuteW(None, w!("open"), &file, PCWSTR::null(), PCWSTR::null(), SW_SHOWNOACTIVATE) };
    done.0 as isize > 32
}
