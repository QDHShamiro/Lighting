use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

use lt_core::{Call, Fail};
use serde_json::json;

use super::super::web::{keys, ref_kind};
use super::combo;
use super::focus::{self, Keys};
use super::input::{self, Button};
use super::uia::{self, El};
use super::win;
use super::{Desk, Halt, Res, TERMINALS, windowish};

const TYPE_SETTLE_S: f64 = 0.3;
const KEY_QUIET_S: f64 = 0.25;
const FIELD_ROLES: &[&str] = &["edit", "combobox", "document", "spinner"];

fn press_keys(keys: &Keys, raw: &str, scancode: bool) -> Result<(), Fail> {
    let parsed = combo::parse(raw, input::scan).map_err(Fail::new)?;
    match keys {
        Keys::Behind(hwnd) => input::bg_press(*hwnd, raw, &parsed).map_err(Fail::new),
        Keys::Front => {
            input::press(&parsed, scancode);
            Ok(())
        }
    }
}

fn type_keys(keys: &Keys, text: &str) {
    match keys {
        Keys::Behind(hwnd) => input::bg_type(*hwnd, text),
        Keys::Front => input::type_text(text),
    }
}

fn settle_ui(hwnd: isize, least: f64, most: f64) {
    thread::sleep(Duration::from_secs_f64(least));
    let t0 = Instant::now();
    let mut last = None;
    while t0.elapsed().as_secs_f64() < most - least {
        let n = uia::collect(hwnd, false, None).len();
        if last == Some(n) {
            return;
        }
        last = Some(n);
        thread::sleep(Duration::from_millis(80));
    }
}

fn button(c: &Call) -> (Button, u32) {
    (if c.on("right") { Button::Right } else { Button::Left }, if c.on("double") { 2 } else { 1 })
}

fn window_set() -> HashSet<isize> {
    win::windows().into_iter().map(|w| w.hwnd).collect()
}

impl Desk<'_> {
    fn note_target(&mut self, el: &El) {
        self.ctx.last_target = Some(json!({"name": el.name(), "role": el.role()}));
    }

    fn new_windows(&mut self, known: &HashSet<isize>) -> String {
        let fresh: Vec<isize> = win::windows().into_iter().map(|w| w.hwnd).filter(|h| !known.contains(h)).collect();
        if fresh.is_empty() {
            return String::new();
        }
        self.list_windows();
        fresh.iter().take(3).map(|h| format!("\n! new window: {}", self.label(*h))).collect()
    }

    fn ocr_click(&mut self, spec: &str, c: &Call) -> Res<Option<String>> {
        let hit = if ref_kind(spec) == Some("o") {
            let Some(hit) = self.st.ocr.iter().find(|o| o.0 == spec) else {
                return Err(Fail::hint(format!("{spec} is unknown"), "lighting read").into());
            };
            hit.clone()
        } else {
            let tl = spec.to_lowercase();
            let exact = self.st.ocr.iter().find(|o| o.1.to_lowercase() == tl);
            match exact.or_else(|| self.st.ocr.iter().find(|o| o.1.to_lowercase().contains(&tl))) {
                Some(hit) => hit.clone(),
                None => return Ok(None),
            }
        };
        let (_, text, x, y) = hit;
        let (b, n) = button(c);
        let hwnd = self.st.hwnd;
        focus::click(self.bg(hwnd), x, y, b, n, hwnd)?;
        let shown: String = text.chars().take(40).collect();
        Ok(Some(format!("ok (clicked \"{shown}\" at {x},{y} via mouse, cursor restored)")))
    }

    fn focused_in(&mut self, hwnd: isize) -> Res<El> {
        for attempt in 1..=2 {
            if let Some(el) = uia::focused()
                && (el.pid() == win::pid_of(hwnd) || win::foreground() == hwnd)
            {
                return Ok(el);
            }
            if attempt == 1 {
                if self.bg(hwnd) {
                    let child = input::focus_child(hwnd);
                    if child != 0
                        && child != hwnd
                        && let Some(el) = uia::from_handle(child)
                    {
                        return Ok(el);
                    }
                    break;
                }
                if !focus::wait_idle() {
                    return Err(Halt::Busy);
                }
                if focus::front(hwnd) {
                    thread::sleep(Duration::from_millis(150));
                }
            }
        }
        let title: String = win::text_of(hwnd).chars().take(40).collect();
        Err(Fail::hint(format!("nothing focused in {title}"), "lighting click the field first, or type d<N> text")
            .into())
    }

    pub(super) fn put_text(&mut self, hwnd: isize, el: &El, label: &str, text: &str, c: &Call) -> Res<String> {
        let shown = format!("{} {}chars", text.chars().count(), if self.ctx.secret.is_some() { "secret " } else { "" });
        let keys: Vec<&str> = ["tab", "enter"].into_iter().filter(|k| c.on(k)).collect();
        let plus: String = keys.iter().map(|k| format!(" +{k}")).collect();
        let append = c.on("append");
        if keys.is_empty() && !Desk::is_chromium(hwnd) {
            let full = if append { el.value().unwrap_or_default() + text } else { text.to_string() };
            if el.set_value(&full) {
                return Ok(format!("ok {label} (typed {shown} in background)"));
            }
        }
        let terminal = TERMINALS.contains(&win::class_of(hwnd).as_str());
        if self.bg(hwnd) {
            focus::calm(hwnd)?;
            let native = el.native();
            if native == 0 {
                let (x, y) = Desk::center(el)?;
                input::bg_click(hwnd, x, y, Button::Left, 1);
            }
            if native != 0 && !append && !terminal {
                win::select_all_in(native);
            }
            let to = Keys::Behind(if native != 0 { native } else { hwnd });
            type_keys(&to, text);
            for k in &keys {
                settle_ui(hwnd, 0.35, 1.0);
                press_keys(&to, k, false)?;
            }
            return Ok(format!("ok {label} (typed {shown} in background{plus})"));
        }
        let saved = win::clip_get();
        let pasted = focus::with_focus(false, hwnd, c.on("stay"), |to| {
            el.focus();
            if !append && !terminal {
                press_keys(&to, "ctrl+a", false)?;
            }
            win::clip_set(text);
            press_keys(&to, "ctrl+v", false)?;
            thread::sleep(Duration::from_millis(80));
            for k in &keys {
                settle_ui(hwnd, 0.35, 1.0);
                press_keys(&to, k, false)?;
            }
            Ok(())
        });
        if let Some(saved) = saved {
            win::clip_set(&saved);
        }
        pasted?;
        Ok(format!("ok {label} (pasted {shown}{plus}, window was briefly in front)"))
    }
}

pub fn click(x: &mut Desk, c: &Call) -> Res<String> {
    let Some(arg) = c.pos.first() else { return Err(Fail::new("click needs a ref or \"text\"").into()) };
    let kind = ref_kind(arg);
    if kind == Some("o") {
        return x.ocr_click(arg, c).map(Option::unwrap_or_default);
    }
    if windowish(arg) {
        return x.focus_cmd(c);
    }
    let hwnd = x.target(None)?;
    let (name, el) = if kind == Some("d") {
        (arg.clone(), x.element(arg)?)
    } else {
        match x.by_text(hwnd, arg, None)? {
            Some(found) => found,
            None => {
                if !x.st.ocr.is_empty()
                    && let Some(done) = x.ocr_click(arg, c)?
                {
                    return Ok(done);
                }
                let wref = x.wref(hwnd);
                return Err(Fail::hint(
                    format!("nothing called \"{arg}\" in {wref}"),
                    "lighting snap, or lighting read for screen text",
                )
                .into());
            }
        }
    };
    x.note_target(&el);
    let (px, py) = Desk::center(&el)?;
    let before = x.before_action(hwnd);
    let known = window_set();
    let bg = x.bg(hwnd);
    if c.on("mouse") || c.on("right") || c.on("double") {
        let (b, n) = button(c);
        focus::click(bg, px, py, b, n, hwnd)?;
        let after = x.after_action(hwnd, before, 0.6, None);
        return Ok(format!("ok {name} (mouse click, cursor restored){after}{}", x.new_windows(&known)));
    }
    if let Some(how) = el.act(focus::front_ok(&x.d.home)) {
        if before.is_none() {
            thread::sleep(Duration::from_millis(150));
        }
        let after = x.after_action(hwnd, before, 0.6, None);
        return Ok(format!("ok {name} ({how} in background){after}{}", x.new_windows(&known)));
    }
    focus::click(bg, px, py, Button::Left, 1, hwnd)?;
    let after = x.after_action(hwnd, before, 0.6, None);
    Ok(format!("ok {name} (mouse click, cursor restored){after}{}", x.new_windows(&known)))
}

pub fn type_in(x: &mut Desk, c: &Call) -> Res<String> {
    let Some(first) = c.pos.first() else {
        return Err(Fail::hint(
            "type needs a d-ref, a field name or focused",
            "lighting type d5 \"hello\" | type \"Search\" hello",
        )
        .into());
    };
    let (label, el) = if ref_kind(first) == Some("d") {
        (first.clone(), x.element(first)?)
    } else if first.eq_ignore_ascii_case("focused") {
        let hwnd = x.target(None)?;
        (first.clone(), x.focused_in(hwnd)?)
    } else {
        let hwnd = x.target(None)?;
        let found = match x.by_text(hwnd, first, Some(FIELD_ROLES))? {
            Some(found) => Some(found),
            None => x.by_text(hwnd, first, None)?,
        };
        let Some(found) = found else {
            let wref = x.wref(hwnd);
            return Err(Fail::hint(format!("no field called \"{first}\" in {wref}"), "lighting snap").into());
        };
        found
    };
    x.note_target(&el);
    let text = x.ctx.secret.clone().unwrap_or_else(|| c.pos[1..].join(" "));
    let hwnd = x.target(None)?;
    let before = x.before_action(hwnd);
    let wait = if c.on("enter") || c.on("tab") { 0.6 } else { TYPE_SETTLE_S };
    let done = x.put_text(hwnd, &el, &label, &text, c)?;
    Ok(done + &x.after_action(hwnd, before, wait, None))
}

pub fn press(x: &mut Desk, c: &Call) -> Res<String> {
    if c.pos.is_empty() {
        return Err(Fail::hint("press needs keys", "lighting press ctrl+s").into());
    }
    let all = c.pos.join(" ");
    if c.pos.iter().all(|p| combo::media(p)) {
        for p in &c.pos {
            press_keys(&Keys::Front, p, false)?;
            thread::sleep(Duration::from_millis(30));
        }
        return Ok(format!("ok ({all}, media key for the whole system)"));
    }
    let hwnd = x.target(None)?;
    let game = c.on("game");
    let before = if game { None } else { x.before_action(hwnd) };
    let known = window_set();
    let bg = x.bg(hwnd);
    focus::with_focus(bg, hwnd, c.on("stay"), |to| {
        for p in &c.pos {
            press_keys(&to, p, game)?;
            thread::sleep(Duration::from_millis(30));
        }
        Ok(())
    })?;
    let quiet = c.pos.iter().all(|p| combo::quick(p)).then_some(KEY_QUIET_S);
    let diff = x.after_action(hwnd, before, 0.6, quiet);
    if diff.starts_with(" (+")
        && c.pos.len() == 1
        && let Some(line) = diff.split('\n').nth(1)
    {
        keys::learn(&x.d.home, &win::exe_of_window(hwnd).to_lowercase(), &c.pos[0], line);
    }
    let label = x.label(hwnd);
    Ok(format!("ok ({all} sent to {label}){diff}{}", x.new_windows(&known)))
}

pub fn scroll(x: &mut Desk, c: &Call) -> Res<String> {
    let dir = ["up", "down", "top", "bottom"]
        .into_iter()
        .find(|d| c.on(d) || c.pos.iter().any(|p| p.eq_ignore_ascii_case(d)));
    let down = !matches!(dir, Some("up" | "top"));
    let word = if down { "down" } else { "up" };
    let refs: Vec<&String> = c.pos.iter().filter(|p| ref_kind(p) == Some("d")).collect();
    let hwnd = x.target(c.pos.iter().find(|p| windowish(p)).map(String::as_str))?;
    let (px, py) = if let Some(r) = refs.first() {
        let el = x.element(r)?;
        if el.scroll(down) {
            return Ok(format!("ok (scrolled {r} {word} in background)"));
        }
        Desk::center(&el)?
    } else {
        let el = if Desk::is_chromium(hwnd) { None } else { uia::main_scroller(hwnd) };
        if el.is_some_and(|el| el.scroll(down)) {
            return Ok(format!("ok (scrolled {} {word} in background)", x.wref(hwnd)));
        }
        let (l, t, r, b) = win::rect(hwnd);
        ((l + r) / 2, (t + b) / 2)
    };
    let delta = if down { -360 } else { 360 };
    if x.bg(hwnd) {
        focus::calm(hwnd)?;
        input::bg_wheel(hwnd, px, py, delta);
        return Ok(format!("ok (wheel {word} in background)"));
    }
    focus::with_focus(false, hwnd, false, |_| {
        let saved = input::cursor();
        input::set_cursor(px, py);
        input::wheel(delta);
        input::set_cursor(saved.0, saved.1);
        Ok(())
    })?;
    Ok(format!("ok (wheel {word}, cursor restored)"))
}

pub fn drag(x: &mut Desk, c: &Call) -> Res<String> {
    let [a, b, ..] = c.pos.as_slice() else {
        return Err(Fail::hint("drag needs two d-refs", "lighting drag d3 d9").into());
    };
    let from = Desk::center(&x.element(a)?)?;
    let to = Desk::center(&x.element(b)?)?;
    let hwnd = x.target(None)?;
    focus::drag(x.bg(hwnd), from, to, hwnd)?;
    Ok(format!("ok (dragged {a} to {b}, cursor restored)"))
}
