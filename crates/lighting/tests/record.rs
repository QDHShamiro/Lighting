#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetForegroundWindow, SetForegroundWindow};
use windows::core::w;

const EXE: &str = env!("CARGO_BIN_EXE_lighting");

struct Home(PathBuf);

impl Home {
    fn new() -> Home {
        let path = std::env::temp_dir().join(format!("ltd-record-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("v3"), "").unwrap();
        Home(path)
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(EXE)
            .args(args)
            .env("LIGHTING_HOME", &self.0)
            .env("USERPROFILE", &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-record")
            .env_remove("MSYSTEM")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["stop"]);
        for _ in 0..30 {
            if fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

fn key(vk: u16, up: bool) -> INPUT {
    let flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn tap(vk: u16) {
    let seq = [key(vk, false), key(vk, true)];
    unsafe { SendInput(&seq, size_of::<INPUT>() as i32) };
    thread::sleep(Duration::from_millis(80));
}

#[test]
fn typing_in_an_app_is_recorded_as_a_routine() {
    if std::env::var_os("CI").is_none() && std::env::var_os("LT_RECORD_TEST").is_none() {
        return;
    }
    let home = Home::new();
    let (code, out) = home.run(&["record", "start", "rectest", "--desktop", "--injected"]);
    assert!(code == 0 && out.contains("desktop apps"), "{out}");
    let mut pad = Command::new("notepad.exe").spawn().unwrap();
    let t0 = Instant::now();
    let hwnd = loop {
        let found = unsafe { FindWindowW(w!("Notepad"), None) };
        if let Ok(h) = found
            && !h.is_invalid()
        {
            break h;
        }
        assert!(t0.elapsed() < Duration::from_secs(15), "notepad did not open");
        thread::sleep(Duration::from_millis(200));
    };
    tap(0x12);
    let _ = unsafe { SetForegroundWindow(hwnd) };
    thread::sleep(Duration::from_millis(600));
    let front = unsafe { GetForegroundWindow() };
    for vk in [0x48, 0x49, 0x0D] {
        tap(vk);
    }
    thread::sleep(Duration::from_millis(800));
    let (code, out) = home.run(&["record", "stop"]);
    let _ = pad.kill();
    let _ = pad.wait();
    assert_eq!(code, 0, "{out}");
    if front != hwnd {
        eprintln!("notepad was not in front, recorder output: {out}");
        return;
    }
    assert!(out.starts_with("saved routine rectest ("), "{out}");
    assert!(out.contains("hi"), "{out}");
    assert!(out.contains("press enter"), "{out}");
    assert!(home.0.join("routines").join("rectest.json").is_file());
}
