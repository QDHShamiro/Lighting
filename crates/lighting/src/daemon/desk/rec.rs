use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lt_core::Fail;
use serde_json::{Map, Value, json};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Input::KeyboardAndMouse::{HKL, ToUnicodeEx};
use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};

use super::hook::{self, Ev};
use super::uia::{self, role_name};
use super::win;

const SKIP_EXES: &[&str] = &[
    "brave.exe",
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "opera.exe",
    "vivaldi.exe",
    "chromium.exe",
    "windowsterminal.exe",
    "openconsole.exe",
    "conhost.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "mintty.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "code.exe",
    "cursor.exe",
    "windsurf.exe",
    "claude.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
];
const SKIP_CLASSES: &[&str] =
    &["Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Progman", "WorkerW", "NotifyIconOverflowWindow"];
const ACTIONABLE: &[&str] = &[
    "button",
    "checkbox",
    "combobox",
    "link",
    "listitem",
    "menuitem",
    "radio",
    "tab",
    "treeitem",
    "dataitem",
    "splitbutton",
    "slider",
    "header",
    "edit",
];
const MOD_VKS: &[u32] = &[0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x14, 0x90, 0x91];
const VK_NAMES: &[(u32, &str)] = &[
    (0x0D, "enter"),
    (0x1B, "escape"),
    (0x20, "space"),
    (0x2E, "delete"),
    (0x2D, "insert"),
    (0x24, "home"),
    (0x23, "end"),
    (0x21, "pageup"),
    (0x22, "pagedown"),
    (0x25, "left"),
    (0x26, "up"),
    (0x27, "right"),
    (0x28, "down"),
    (0x08, "backspace"),
    (0x09, "tab"),
];
const CHAIN: usize = 5;
const CLICK_CHARS: usize = 80;
const FIELD_CHARS: usize = 60;
const READY: Duration = Duration::from_secs(3);

pub struct Recorder {
    tid: u32,
    hooks: Option<JoinHandle<()>>,
    work: Option<JoinHandle<()>>,
    steps: Arc<Mutex<Vec<Value>>>,
}

pub fn vk_name(vk: u32) -> Option<String> {
    if let Some((_, n)) = VK_NAMES.iter().find(|(v, _)| *v == vk) {
        return Some((*n).to_string());
    }
    if (0x70..=0x87).contains(&vk) {
        return Some(format!("f{}", vk - 0x6F));
    }
    char::from_u32(vk)
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .map(|c| c.to_ascii_lowercase().to_string())
}

pub fn app_key(exe: &str, title: &str) -> String {
    let stem = if exe.to_lowercase().ends_with(".exe") { &exe[..exe.len() - 4] } else { exe };
    if stem.eq_ignore_ascii_case("applicationframehost") && !title.is_empty() {
        title.to_string()
    } else {
        stem.to_string()
    }
}

fn root_of(hwnd: isize) -> isize {
    if hwnd == 0 {
        return 0;
    }
    unsafe { GetAncestor(HWND(hwnd as *mut _), GA_ROOT) }.0 as isize
}

fn chain_at(x: i32, y: i32) -> Vec<(&'static str, String)> {
    uia::api(|u| unsafe {
        let walker = u.ControlViewWalker()?;
        let mut cur = Some(u.ElementFromPoint(POINT { x, y })?);
        let mut out = Vec::new();
        for _ in 0..CHAIN {
            let Some(el) = cur else { break };
            let name = el.CurrentName().map(|n| n.to_string().trim().to_string()).unwrap_or_default();
            out.push((role_name(el.CurrentControlType().map_or(0, |t| t.0)), name));
            cur = walker.GetParentElement(&el).ok();
        }
        Ok(out)
    })
    .unwrap_or_default()
}

fn focused_field(root: isize) -> (String, bool) {
    uia::api(|u| unsafe {
        let el = u.GetFocusedElement()?;
        if el.CurrentProcessId()? as u32 != win::pid_of(root) {
            return Ok((String::new(), false));
        }
        let name: String = el.CurrentName()?.to_string().trim().chars().take(FIELD_CHARS).collect();
        Ok((name, el.CurrentIsPassword()?.as_bool()))
    })
    .unwrap_or_default()
}

fn typed(vk: u32, scan: u32, mods: [bool; 5], hkl: isize) -> String {
    let [shift, ctrl, alt, _, caps] = mods;
    let mut state = [0u8; 256];
    for (on, key, val) in [(shift, 0x10, 0x80), (ctrl, 0x11, 0x80), (alt, 0x12, 0x80), (caps, 0x14, 0x01)] {
        if on {
            state[key] = val;
        }
    }
    let mut buf = [0u16; 8];
    let n = unsafe { ToUnicodeEx(vk, scan, &state, &mut buf, 0x4, Some(HKL(hkl as *mut _))) };
    if n > 0 { String::from_utf16_lossy(&buf[..n as usize]) } else { String::new() }
}

struct Work {
    steps: Arc<Mutex<Vec<Value>>>,
    pids0: HashSet<u32>,
    started: HashSet<String>,
    me: u32,
    last_root: isize,
    buf: String,
    buf_root: isize,
    buf_target: (String, bool),
    buf_t: f64,
}

impl Work {
    fn push(&self, step: Value) {
        self.steps.lock().unwrap_or_else(PoisonError::into_inner).push(step);
    }

    fn emit(&self, cmd: &str, args: Value, t: f64, mut flags: Map<String, Value>) {
        flags.insert("on".into(), json!("app"));
        self.push(json!({"t": t, "cmd": cmd, "args": args, "flags": flags}));
    }

    fn skip(&self, root: isize) -> bool {
        if root == 0 {
            return true;
        }
        let pid = win::pid_of(root);
        pid == self.me
            || SKIP_CLASSES.contains(&win::class_of(root).as_str())
            || SKIP_EXES.contains(&win::exe_of(pid).to_lowercase().as_str())
    }

    fn context(&mut self, root: isize, t: f64) {
        if root == self.last_root {
            return;
        }
        self.last_root = root;
        let pid = win::pid_of(root);
        let key = app_key(&win::exe_of(pid), &win::text_of(root));
        if !self.pids0.contains(&pid) && self.started.insert(key.clone()) {
            self.emit("launch", json!([key]), t, Map::new());
        } else {
            self.emit("focus", json!([format!("app:{key}")]), t, Map::new());
        }
    }

    fn click(&mut self, x: i32, y: i32, right: bool, t: f64) {
        let root = root_of(unsafe { WindowFromPoint(POINT { x, y }) }.0 as isize);
        if self.skip(root) {
            return;
        }
        let (mut pick, mut text_name) = (None, String::new());
        for (role, name) in chain_at(x, y) {
            if role == "text" && !name.is_empty() && text_name.is_empty() {
                text_name.clone_from(&name);
            }
            if ACTIONABLE.contains(&role) && (!name.is_empty() || role == "edit") {
                pick = Some((role, name));
                break;
            }
        }
        if pick.as_ref().is_some_and(|(r, _)| *r == "edit") {
            return;
        }
        let name = pick.map_or(text_name, |(_, n)| n);
        self.context(root, t);
        if name.is_empty() {
            let note = format!("click at {x},{y} had no name, not recorded");
            self.push(json!({"t": t, "cmd": "#", "args": [note], "flags": {}}));
            return;
        }
        let mut flags = Map::new();
        if right {
            flags.insert("right".into(), json!(true));
        }
        self.emit("click", json!([name.chars().take(CLICK_CHARS).collect::<String>()]), t, flags);
    }

    fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let (name, secret) = std::mem::take(&mut self.buf_target);
        self.context(self.buf_root, self.buf_t);
        let text = if secret { "@secret".to_string() } else { std::mem::take(&mut self.buf) };
        let field = if name.is_empty() { "focused".to_string() } else { name };
        self.emit("type", json!([field, text]), self.buf_t, Map::new());
        self.buf.clear();
        self.buf_root = 0;
    }

    fn key(&mut self, vk: u32, scan: u32, mods: [bool; 5], fg: isize, hkl: isize, t: f64) {
        let [shift, ctrl, alt, winkey, _] = mods;
        let root = root_of(fg);
        if self.skip(root) || MOD_VKS.contains(&vk) {
            return;
        }
        let ch = if winkey { String::new() } else { typed(vk, scan, mods, hkl) };
        if !ch.is_empty() && !ch.chars().any(char::is_control) && (!(ctrl || alt) || (ctrl && alt)) {
            if self.buf_root != root {
                self.flush();
                (self.buf_root, self.buf_target, self.buf_t) = (root, focused_field(root), t);
            }
            self.buf += &ch;
            return;
        }
        if vk == 0x08 && !self.buf.is_empty() {
            self.buf.pop();
            return;
        }
        let Some(name) = vk_name(vk) else { return };
        if vk == 0x09 && !(ctrl || alt) {
            self.flush();
            return;
        }
        let mut keys: Vec<String> = [("ctrl", ctrl), ("alt", alt), ("shift", shift), ("win", winkey)]
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(m, _)| m.to_string())
            .collect();
        keys.push(name);
        self.flush();
        self.context(root, t);
        self.emit("press", json!([keys.join("+")]), t, Map::new());
    }

    fn run(mut self, rx: &Receiver<Ev>) {
        while let Ok(ev) = rx.recv() {
            match ev {
                Ev::Click { x, y, right, t } => {
                    self.flush();
                    self.click(x, y, right, t);
                }
                Ev::Key { vk, scan, mods, fg, hkl, t } => self.key(vk, scan, mods, fg, hkl, t),
            }
        }
        self.flush();
    }
}

impl Recorder {
    pub fn start(minutes: u64, injected: bool) -> Result<Recorder, Fail> {
        let steps = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let hooks = thread::spawn(move || hook::run(tx, injected, minutes, &ready_tx));
        let tid = ready_rx.recv_timeout(READY).map_err(|_| Fail::new("could not start the desktop recorder"))?;
        let work = Work {
            steps: Arc::clone(&steps),
            pids0: win::windows().iter().map(|w| win::pid_of(w.hwnd)).collect(),
            started: HashSet::new(),
            me: std::process::id(),
            last_root: 0,
            buf: String::new(),
            buf_root: 0,
            buf_target: (String::new(), false),
            buf_t: 0.0,
        };
        let work = thread::spawn(move || work.run(&rx));
        Ok(Recorder { tid, hooks: Some(hooks), work: Some(work), steps })
    }

    pub fn count(&self) -> usize {
        self.steps.lock().unwrap_or_else(PoisonError::into_inner).iter().filter(|s| s["cmd"] != "#").count()
    }

    pub fn stop(mut self) -> Vec<Value> {
        hook::quit(self.tid);
        if let Some(h) = self.hooks.take() {
            let _ = h.join();
        }
        if let Some(w) = self.work.take() {
            let _ = w.join();
        }
        std::mem::take(&mut *self.steps.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_app_names_read_like_2x() {
        assert_eq!(vk_name(0x0D).as_deref(), Some("enter"));
        assert_eq!(vk_name(0x70).as_deref(), Some("f1"));
        assert_eq!(vk_name(0x87).as_deref(), Some("f24"));
        assert_eq!(vk_name(0x41).as_deref(), Some("a"));
        assert_eq!(vk_name(0x35).as_deref(), Some("5"));
        assert_eq!(vk_name(0xBA), None);
        assert_eq!(app_key("Notepad.EXE", "x"), "Notepad");
        assert_eq!(app_key("ApplicationFrameHost.exe", "Rechner"), "Rechner");
        assert_eq!(app_key("ApplicationFrameHost.exe", ""), "ApplicationFrameHost");
    }
}
