use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, LASTINPUTINFO, MAPVK_VK_TO_VSC,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VkKeyScanW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT, ChildWindowFromPointEx, FindWindowExW, GUITHREADINFO, GetCursorPos,
    GetGUIThreadInfo, IsChild, SetCursorPos,
};
use windows::core::{PCWSTR, w};

use super::combo::{CTRL, Combo, EXTENDED, SHIFT};
use super::win::{class_of, h, post, thread_of};

pub use super::combo::ALT;

const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_MOUSEWHEEL: u32 = 0x020A;
const CHUNK: usize = 200;
pub const DRAG_STEPS: i32 = 12;
const CHROMIUM_HOSTS: &[&str] = &["Chrome_RenderWidgetHostHWND", "Chrome_WidgetWin_1", "Chrome_WidgetWin_0"];
pub const FRONT_HINT: &str =
    "Lighting does not take the screen from the user; lighting config focus away allows it while they are away";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

impl Button {
    fn flags(self) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
        match self {
            Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        }
    }

    fn messages(self) -> (u32, u32, u32, usize) {
        match self {
            Button::Left => (0x201, 0x202, 0x203, 1),
            Button::Right => (0x204, 0x205, 0x206, 2),
        }
    }
}

fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, data: u32) -> INPUT {
    let mi = MOUSEINPUT { dx: 0, dy: 0, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi } }
}

fn key_input(vk: u16, up: bool, scancode: bool) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
    let (mut wvk, mut scan) = (vk, 0u16);
    if scancode {
        scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) } as u16;
        flags |= KEYEVENTF_SCANCODE;
        wvk = 0;
    }
    if EXTENDED.contains(&vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let ki = KEYBDINPUT { wVk: VIRTUAL_KEY(wvk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki } }
}

fn unicode_input(unit: u16, up: bool) -> INPUT {
    let flags = if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE };
    let ki = KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki } }
}

pub fn key(vk: u16, up: bool, scancode: bool) {
    send(&[key_input(vk, up, scancode)]);
}

pub fn scan(ch: char) -> Option<(u16, u8)> {
    let mut buf = [0u16; 2];
    let units = ch.encode_utf16(&mut buf);
    if units.len() != 1 {
        return None;
    }
    let res = unsafe { VkKeyScanW(units[0]) };
    if res == -1 {
        return None;
    }
    Some(((res & 0xFF) as u16, ((res >> 8) & 0xFF) as u8))
}

pub fn press(combo: &Combo, scancode: bool) {
    let mut mods = combo.mods.clone();
    for (bit, m) in [(1, SHIFT), (2, CTRL), (4, ALT)] {
        if combo.shift_state & bit != 0 && !mods.contains(&m) {
            mods.push(m);
        }
    }
    let mut seq: Vec<INPUT> = mods.iter().map(|m| key_input(*m, false, scancode)).collect();
    seq.push(key_input(combo.vk, false, scancode));
    seq.push(key_input(combo.vk, true, scancode));
    seq.extend(mods.iter().rev().map(|m| key_input(*m, true, scancode)));
    send(&seq);
}

pub fn type_text(text: &str) {
    let seq: Vec<INPUT> =
        text.encode_utf16().flat_map(|unit| [unicode_input(unit, false), unicode_input(unit, true)]).collect();
    for part in seq.chunks(CHUNK) {
        send(part);
    }
}

pub fn idle_ms() -> u32 {
    let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    let _ = unsafe { GetLastInputInfo(&raw mut info) };
    unsafe { GetTickCount() }.wrapping_sub(info.dwTime)
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&raw mut p) };
    (p.x, p.y)
}

pub fn set_cursor(x: i32, y: i32) {
    let _ = unsafe { SetCursorPos(x, y) };
}

pub fn click(button: Button, count: u32) {
    let (down, up) = button.flags();
    let seq: Vec<INPUT> = (0..count).flat_map(|_| [mouse_input(down, 0), mouse_input(up, 0)]).collect();
    send(&seq);
}

pub fn mouse_down() {
    send(&[mouse_input(MOUSEEVENTF_LEFTDOWN, 0)]);
}

pub fn mouse_up() {
    send(&[mouse_input(MOUSEEVENTF_LEFTUP, 0)]);
}

pub fn wheel(delta: i32) {
    send(&[mouse_input(MOUSEEVENTF_WHEEL, delta as u32)]);
}

pub fn focus_child(hwnd: isize) -> isize {
    let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    let tid = thread_of(hwnd);
    if tid != 0 && unsafe { GetGUIThreadInfo(tid, &raw mut info) }.is_ok() {
        let focus = info.hwndFocus;
        if !focus.is_invalid() && (focus.0 as isize == hwnd || unsafe { IsChild(h(hwnd), focus) }.as_bool()) {
            return focus.0 as isize;
        }
    }
    unsafe { FindWindowExW(Some(h(hwnd)), None, w!("Chrome_RenderWidgetHostHWND"), PCWSTR::null()) }
        .map_or(hwnd, |child| child.0 as isize)
}

fn xy(x: i32, y: i32) -> isize {
    (((y & 0xFFFF) << 16) | (x & 0xFFFF)) as isize
}

fn child_at(hwnd: isize, x: i32, y: i32) -> (isize, i32, i32) {
    let mut cur = hwnd;
    loop {
        let mut pt = POINT { x, y };
        let _ = unsafe { ScreenToClient(h(cur), &raw mut pt) };
        let next = unsafe { ChildWindowFromPointEx(h(cur), pt, CWP_SKIPINVISIBLE | CWP_SKIPTRANSPARENT) };
        if next.is_invalid() || next.0 as isize == cur {
            return (cur, pt.x, pt.y);
        }
        cur = next.0 as isize;
    }
}

fn bg_key(target: isize, vk: u16, up: bool) {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) };
    let mut lp = 1u32 | (scan << 16) | if EXTENDED.contains(&vk) { 1 << 24 } else { 0 };
    if up {
        lp |= 3 << 30;
    }
    post(target, if up { WM_KEYUP } else { WM_KEYDOWN }, usize::from(vk), lp as isize);
}

fn edit_key(main: &str) -> Option<(u32, usize, isize)> {
    Some(match main.to_lowercase().as_str() {
        "a" => (0x00B1, 0, -1),
        "c" => (0x0301, 0, 0),
        "v" => (0x0302, 0, 0),
        "x" => (0x0300, 0, 0),
        "z" => (0x0304, 0, 0),
        _ => return None,
    })
}

fn char_key(vk: u16) -> Option<char> {
    Some(match vk {
        0x0D => '\r',
        0x09 => '\t',
        0x08 => '\u{8}',
        0x1B => '\u{1b}',
        0x20 => ' ',
        _ => return None,
    })
}

pub fn bg_press(hwnd: isize, raw: &str, combo: &Combo) -> Result<(), String> {
    let target = focus_child(hwnd);
    if combo.mods == [CTRL]
        && !CHROMIUM_HOSTS.contains(&class_of(target).as_str())
        && let Some((msg, wparam, lparam)) = edit_key(&combo.main)
    {
        post(target, msg, wparam, lparam);
        return Ok(());
    }
    let single = combo.main.chars().count() == 1;
    if combo.mods.iter().any(|m| ![SHIFT, 0xA0, 0xA1].contains(m)) || (!combo.mods.is_empty() && !single) {
        return Err(format!("{raw} needs the window in front, and {FRONT_HINT}"));
    }
    let ch = if single {
        let c = combo.main.chars().next();
        if combo.mods.is_empty() { c } else { c.map(|c| c.to_uppercase().next().unwrap_or(c)) }
    } else {
        char_key(combo.vk)
    };
    bg_key(target, combo.vk, false);
    if let Some(ch) = ch {
        post(target, WM_CHAR, ch as usize, 1);
    }
    bg_key(target, combo.vk, true);
    Ok(())
}

pub fn bg_type(hwnd: isize, text: &str) {
    let target = focus_child(hwnd);
    for unit in text.replace("\r\n", "\n").replace('\n', "\r").encode_utf16() {
        post(target, WM_CHAR, usize::from(unit), 1);
    }
}

pub fn bg_click(hwnd: isize, x: i32, y: i32, button: Button, count: u32) {
    let (target, cx, cy) = child_at(hwnd, x, y);
    let (down, up, double, mk) = button.messages();
    let lp = xy(cx, cy);
    post(target, WM_MOUSEMOVE, 0, lp);
    for i in 0..count {
        post(target, if i == 0 { down } else { double }, mk, lp);
        post(target, up, 0, lp);
    }
}

pub fn bg_drag(hwnd: isize, a: (i32, i32), b: (i32, i32)) {
    let (target, ax, ay) = child_at(hwnd, a.0, a.1);
    let (bx, by) = (ax + b.0 - a.0, ay + b.1 - a.1);
    post(target, 0x0201, 1, xy(ax, ay));
    for i in 1..=DRAG_STEPS {
        post(target, WM_MOUSEMOVE, 1, xy(ax + (bx - ax) * i / DRAG_STEPS, ay + (by - ay) * i / DRAG_STEPS));
        thread::sleep(Duration::from_millis(12));
    }
    post(target, 0x0202, 0, xy(bx, by));
}

pub fn bg_wheel(hwnd: isize, x: i32, y: i32, delta: i32) {
    let target = child_at(hwnd, x, y).0;
    post(target, WM_MOUSEWHEEL, usize::from(delta as u16) << 16, xy(x, y));
}
