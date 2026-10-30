use std::thread;
use std::time::{Duration, Instant};

use lt_core::Fail;

use super::super::config;
use super::super::game;
use super::input::{self, Button, DRAG_STEPS};
use super::uia;
use super::win;
use super::{Halt, Res};

const IDLE_MS: u32 = 300;
const BLITZ_WAIT: Duration = Duration::from_millis(800);
const AWAY_MS: u32 = 60_000;

pub fn front_ok(home: &std::path::Path) -> bool {
    let mode = config::text(&config::load(home), "focus");
    if mode.is_empty() || mode == "never" || !game::running().is_empty() {
        return false;
    }
    mode == "idle" || input::idle_ms() >= AWAY_MS
}

pub fn wait_idle() -> bool {
    let deadline = Instant::now() + BLITZ_WAIT;
    while input::idle_ms() < IDLE_MS {
        if Instant::now() > deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(30));
    }
    true
}

pub fn idle_now() -> bool {
    input::idle_ms() >= IDLE_MS
}

pub fn calm(hwnd: isize) -> Res<()> {
    if win::foreground() == hwnd && !wait_idle() {
        return Err(Halt::Busy);
    }
    Ok(())
}

pub fn front(hwnd: isize) -> bool {
    if win::set_foreground(hwnd) {
        return true;
    }
    if uia::from_handle(hwnd).is_some_and(|el| el.focus()) {
        thread::sleep(Duration::from_millis(60));
        if win::foreground() == hwnd {
            return true;
        }
    }
    win::restore_cycle(hwnd);
    thread::sleep(Duration::from_millis(120));
    win::foreground() == hwnd || win::set_foreground(hwnd)
}

pub fn click(bg: bool, x: i32, y: i32, button: Button, count: u32, hwnd: isize) -> Res<()> {
    if bg {
        calm(hwnd)?;
        input::bg_click(hwnd, x, y, button, count);
        return Ok(());
    }
    if !wait_idle() {
        return Err(Halt::Busy);
    }
    if hwnd != 0 {
        front(hwnd);
    }
    let saved = input::cursor();
    input::set_cursor(x, y);
    thread::sleep(Duration::from_millis(10));
    input::click(button, count);
    thread::sleep(Duration::from_millis(10));
    input::set_cursor(saved.0, saved.1);
    Ok(())
}

pub fn drag(bg: bool, a: (i32, i32), b: (i32, i32), hwnd: isize) -> Res<()> {
    if bg {
        calm(hwnd)?;
        input::bg_drag(hwnd, a, b);
        return Ok(());
    }
    if !wait_idle() {
        return Err(Halt::Busy);
    }
    front(hwnd);
    let saved = input::cursor();
    input::set_cursor(a.0, a.1);
    input::mouse_down();
    for i in 1..=DRAG_STEPS {
        input::set_cursor(a.0 + (b.0 - a.0) * i / DRAG_STEPS, a.1 + (b.1 - a.1) * i / DRAG_STEPS);
        thread::sleep(Duration::from_millis(12));
    }
    input::mouse_up();
    input::set_cursor(saved.0, saved.1);
    Ok(())
}

pub enum Keys {
    Front,
    Behind(isize),
}

pub fn with_focus<T>(bg: bool, hwnd: isize, stay: bool, f: impl FnOnce(Keys) -> Result<T, Fail>) -> Res<T> {
    if bg {
        calm(hwnd)?;
        return f(Keys::Behind(hwnd)).map_err(Halt::Fail);
    }
    if !wait_idle() {
        return Err(Halt::Busy);
    }
    let prev = win::foreground();
    if !front(hwnd) {
        return f(Keys::Behind(hwnd)).map_err(Halt::Fail);
    }
    let out = f(Keys::Front);
    if !stay && prev != 0 && prev != hwnd && win::alive(prev) {
        thread::sleep(Duration::from_millis(50));
        win::set_foreground(prev);
    }
    out.map_err(Halt::Fail)
}
