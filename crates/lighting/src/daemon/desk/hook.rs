use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, PoisonError};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
    FW_BOLD, FillRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState, GetKeyboardLayout};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow, GetMessageW,
    GetSystemMetrics, GetWindowThreadProcessId, KBDLLHOOKSTRUCT, KillTimer, LWA_ALPHA, MSG, MSLLHOOKSTRUCT,
    PostThreadMessageW, RegisterClassW, SM_CXSCREEN, SW_SHOWNOACTIVATE, SetLayeredWindowAttributes, SetTimer,
    SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_PAINT, WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::super::ctx::now;

const MOUSE_INJECTED: u32 = 0x01;
const KEY_INJECTED: u32 = 0x10;
const BADGE_W: i32 = 132;
const BADGE_H: i32 = 26;
const BADGE_MARGIN: i32 = 18;
const BADGE_TOP: i32 = 14;
const BADGE_ALPHA: u8 = 235;
const BADGE_RED: u32 = 0x0000_00D0;
const WHITE: u32 = 0x00FF_FFFF;
const FONT_PX: i32 = 15;

pub enum Ev {
    Click { x: i32, y: i32, right: bool, t: f64 },
    Key { vk: u32, scan: u32, mods: [bool; 5], fg: isize, hkl: isize, t: f64 },
}

static TX: Mutex<Option<Sender<Ev>>> = Mutex::new(None);
static INJECTED: AtomicBool = AtomicBool::new(false);

fn send(ev: Ev) {
    if let Some(tx) = TX.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        let _ = tx.send(ev);
    }
}

fn down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

unsafe extern "system" fn on_mouse(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let msg = wp.0 as u32;
    if code == 0 && (msg == WM_LBUTTONDOWN || msg == WM_RBUTTONDOWN) {
        let info = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        if INJECTED.load(Ordering::Relaxed) || info.flags & MOUSE_INJECTED == 0 {
            send(Ev::Click { x: info.pt.x, y: info.pt.y, right: msg == WM_RBUTTONDOWN, t: now() });
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

unsafe extern "system" fn on_key(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let msg = wp.0 as u32;
    if code == 0 && (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) {
        let info = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        if INJECTED.load(Ordering::Relaxed) || info.flags.0 & KEY_INJECTED == 0 {
            let caps = unsafe { GetKeyState(0x14) } & 1 != 0;
            let mods = [down(0x10), down(0x11), down(0x12), down(0x5B) || down(0x5C), caps];
            let fg = unsafe { GetForegroundWindow() };
            let hkl = unsafe { GetKeyboardLayout(GetWindowThreadProcessId(fg, None)) };
            send(Ev::Key {
                vk: info.vkCode,
                scan: info.scanCode,
                mods,
                fg: fg.0 as isize,
                hkl: hkl.0 as isize,
                t: now(),
            });
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

unsafe extern "system" fn badge_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg != WM_PAINT {
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    }
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &raw mut ps);
        let brush = CreateSolidBrush(COLORREF(BADGE_RED));
        let mut r = RECT { left: 0, top: 0, right: BADGE_W, bottom: BADGE_H };
        FillRect(dc, &raw const r, brush);
        let font = CreateFontW(
            FONT_PX,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            0,
            w!("Segoe UI"),
        );
        let old = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(WHITE));
        let mut text: Vec<u16> = "\u{25CF} REC Lighting".encode_utf16().collect();
        DrawTextW(dc, &mut text, &raw mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(brush.into());
        let _ = EndPaint(hwnd, &raw const ps);
    }
    LRESULT(0)
}

fn badge() -> Option<HWND> {
    unsafe {
        let inst = GetModuleHandleW(None).ok()?;
        let class = w!("LightingRecBadge");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(badge_proc),
            hInstance: inst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&raw const wc);
        let x = GetSystemMetrics(SM_CXSCREEN) - BADGE_W - BADGE_MARGIN;
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED;
        let hwnd = CreateWindowExW(
            ex,
            class,
            PCWSTR::null(),
            WS_POPUP,
            x,
            BADGE_TOP,
            BADGE_W,
            BADGE_H,
            None,
            None,
            Some(inst.into()),
            None,
        )
        .ok()?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), BADGE_ALPHA, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        Some(hwnd)
    }
}

pub fn run(tx: Sender<Ev>, injected: bool, minutes: u64, ready: &Sender<u32>) {
    INJECTED.store(injected, Ordering::Relaxed);
    *TX.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
    unsafe {
        let inst = GetModuleHandleW(None).ok();
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(on_mouse), inst.map(Into::into), 0).ok();
        let keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), inst.map(Into::into), 0).ok();
        let shown = badge();
        let timer = SetTimer(None, 0, u32::try_from(minutes * 60_000).unwrap_or(u32::MAX), None);
        let _ = ready.send(GetCurrentThreadId());
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, None, 0, 0).0 > 0 {
            if msg.message == WM_TIMER && msg.wParam.0 == timer {
                break;
            }
            DispatchMessageW(&raw const msg);
        }
        let _ = KillTimer(None, timer);
        for h in [mouse, keys].into_iter().flatten() {
            let _ = UnhookWindowsHookEx(h);
        }
        if let Some(b) = shown {
            let _ = DestroyWindow(b);
        }
    }
    *TX.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

pub fn quit(tid: u32) {
    let _ = unsafe { PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) };
}
