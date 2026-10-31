use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use image::{DynamicImage, RgbaImage};
use wisply_core::{Call, Fail, OUT_CHARS};
use xa11y::{App, AppExt, Element, Error, Key};

use super::super::Daemon;
use super::super::ctx::Ctx;
use super::super::web::{SNAP_LINES, TEXT_CHARS, capped, ref_kind, save_picture};
use super::view::{score, short};
use super::xa::{FRONT_HINT, State, fail, node, walk};
use super::xview;

const POLL: Duration = Duration::from_millis(300);
const LAUNCH_WAIT: Duration = Duration::from_secs(10);
const CLOSE_WAIT: Duration = Duration::from_millis(1500);
const CLOSE_NAMES: &[&str] = &["close", "close window"];
const CLOSE_SUBROLE: &str = "AXCloseButton";

pub fn click(st: &mut State, c: &Call) -> Result<String, Fail> {
    let spec = c.pos.first().ok_or_else(|| Fail::hint("click needs a control", "wisply click d5"))?;
    let (dref, el) = st.el_ref(spec)?;
    let role = xview::role(&el.role.to_string()).unwrap_or("");
    let tries: [fn(&Element) -> xa11y::Result<()>; 4] = match role {
        "checkbox" | "radio" => [Element::toggle, Element::press, Element::select, Element::focus],
        "listitem" | "tab" | "dataitem" => [Element::select, Element::press, Element::toggle, Element::focus],
        "treeitem" if el.states.expanded == Some(false) => {
            [Element::expand, Element::select, Element::press, Element::focus]
        }
        _ => [Element::press, Element::select, Element::toggle, Element::expand],
    };
    let mut last = None;
    for t in tries {
        match t(&el) {
            Ok(()) => {
                return Ok(format!("ok (clicked {dref} {role} \"{}\")", short(el.name.as_deref().unwrap_or(""), 40)));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.map_or_else(|| Fail::new("click failed"), |e| fail(&e)))
}

pub fn type_in(st: &mut State, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let usage = "wisply type d5 \"text\"";
    let spec = c
        .pos
        .first()
        .filter(|p| ref_kind(p) == Some("d"))
        .ok_or_else(|| Fail::hint("type needs a field ref", usage))?;
    let text = ctx.secret.clone().unwrap_or_else(|| c.pos[1..].join(" "));
    let (dref, el) = st.el_ref(spec)?;
    let typed = if c.on("append") { Err(Error::TextValueNotSupported) } else { el.set_value(&text) };
    if typed.is_err() {
        el.focus().and_then(|()| el.type_text(&text)).map_err(|e| fail(&e))?;
    }
    if c.on("enter") {
        press_keys(st, "enter")?;
    }
    Ok(format!("ok (typed {} chars into {dref})", text.chars().count()))
}

fn held_key(m: &str) -> Key {
    match m {
        "ctrl" => Key::Ctrl,
        "shift" => Key::Shift,
        "alt" => Key::Alt,
        _ => Key::Meta,
    }
}

fn key_of(k: xview::Key) -> Key {
    match k {
        xview::Key::Char(c) => Key::Char(c),
        xview::Key::F(n) => Key::F(n),
        xview::Key::Named(n) => match n {
            "enter" => Key::Enter,
            "escape" => Key::Escape,
            "tab" => Key::Tab,
            "space" => Key::Space,
            "backspace" => Key::Backspace,
            "delete" => Key::Delete,
            "insert" => Key::Insert,
            "up" => Key::ArrowUp,
            "down" => Key::ArrowDown,
            "left" => Key::ArrowLeft,
            "right" => Key::ArrowRight,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            _ => Key::PageDown,
        },
    }
}

fn front_pid(st: &mut State) -> Result<u32, Fail> {
    let r = st.target(None)?;
    let pid = st.windows[&r].pid;
    let front = App::by_pid(pid, Duration::ZERO).is_ok_and(|a| a.is_foreground());
    front
        .then_some(pid)
        .ok_or_else(|| Fail::hint(format!("keys go to the window in front, and {r} is not"), FRONT_HINT))
}

fn press_keys(st: &mut State, combo: &str) -> Result<(), Fail> {
    front_pid(st)?;
    let (held, key) = xview::combo(combo).map_err(|e| Fail::hint(e, "wisply press ctrl+s"))?;
    let sim = xa11y::input_sim().map_err(|e| fail(&e))?;
    let (held, key) = (held.into_iter().map(held_key).collect::<Vec<_>>(), key_of(key));
    sim.keyboard().chord(key, &held).map_err(|e| fail(&e))
}

pub fn press(st: &mut State, c: &Call) -> Result<String, Fail> {
    let combo = c.pos.join(" ");
    if combo.trim().is_empty() {
        return Err(Fail::hint("press needs a key", "wisply press ctrl+s"));
    }
    press_keys(st, &combo)?;
    Ok(format!("ok (pressed {combo})"))
}

pub fn scroll(st: &mut State, c: &Call) -> Result<String, Fail> {
    let spec = c.pos.first().filter(|p| ref_kind(p) == Some("d")).ok_or_else(|| {
        Fail::hint(
            "on this system scroll brings a control into view",
            "wisply scroll d40 (a ref from wisply snap --full)",
        )
    })?;
    let (dref, el) = st.el_ref(spec)?;
    el.scroll_into_view().map_err(|e| fail(&e))?;
    Ok(format!("ok ({dref} is in view)"))
}

pub fn read(d: &Daemon, st: &mut State, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let r = st.target(c.pos.first().map(String::as_str))?;
    let root = st.element(&r)?;
    let mut lines: Vec<String> = Vec::new();
    for el in walk(&root) {
        let n = node(&el);
        for part in [Some(n.name), n.value.filter(|_| !n.secret)].into_iter().flatten() {
            let t = part.split_whitespace().collect::<Vec<_>>().join(" ");
            if !t.is_empty() && lines.last() != Some(&t) {
                lines.push(t);
            }
        }
    }
    let head = st.header(&r, &format!(" (text, {} lines)", lines.len()));
    let body = if lines.is_empty() { "(no text found)".to_string() } else { lines.join("\n") };
    Ok(capped(d, ctx, &format!("{head}\n{body}"), "read", Some(SNAP_LINES), OUT_CHARS))
}

pub fn shot(d: &Daemon, st: &mut State, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let r = st.target(c.pos.first().map(String::as_str))?;
    let el = st.element(&r)?;
    let pic = xa11y::screenshot_element(&el).map_err(|e| fail(&e))?;
    let img = RgbaImage::from_raw(pic.width, pic.height, pic.pixels)
        .ok_or_else(|| Fail::new("the screenshot came back broken"))?;
    save_picture(d, ctx, c, &DynamicImage::ImageRgba8(img), &format!("app-{r}"), "wisply shot w3 --out ~/shots/app.png")
}

fn clip_tools(set: bool) -> (&'static str, &'static [&'static str]) {
    match (cfg!(target_os = "macos"), std::env::var_os("WAYLAND_DISPLAY").is_some(), set) {
        (true, _, true) => ("pbcopy", &[]),
        (true, _, false) => ("pbpaste", &[]),
        (false, true, true) => ("wl-copy", &[]),
        (false, true, false) => ("wl-paste", &["--no-newline"]),
        (false, false, true) => ("xclip", &["-selection", "clipboard"]),
        (false, false, false) => ("xclip", &["-selection", "clipboard", "-o"]),
    }
}

pub fn clip(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let set = c.pos.first().is_some_and(|p| p == "set");
    let (tool, args) = clip_tools(set);
    let missing =
        |e: std::io::Error| Fail::hint(format!("cannot use the clipboard ({tool}: {e})"), format!("install {tool}"));
    if set {
        let text = ctx.secret.clone().unwrap_or_else(|| c.pos[1..].join(" "));
        let mut child =
            Command::new(tool).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(missing)?;
        child.stdin.take().map(|mut s| s.write_all(text.as_bytes())).transpose().map_err(missing)?;
        child.wait().map_err(missing)?;
        return Ok("ok (clipboard set)".into());
    }
    let out = Command::new(tool).args(args).stderr(Stdio::null()).output().map_err(missing)?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let text = if text.is_empty() { "(clipboard is empty)".to_string() } else { text };
    Ok(capped(d, ctx, &text, "clip", None, TEXT_CHARS))
}

pub fn launch(st: &mut State, _: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let name = c.pos.join(" ");
    if name.trim().is_empty() {
        return Err(Fail::hint("launch needs an app", "wisply launch \"Text Editor\""));
    }
    let before: Vec<String> = st.list()?;
    let mut cmd = if cfg!(target_os = "macos") {
        let mut o = Command::new("open");
        o.args(["-g", "-a", &name]);
        o
    } else {
        Command::new(&name)
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map_err(|e| Fail::hint(format!("cannot start {name} ({e})"), "wisply windows"))?;
    let deadline = Instant::now() + LAUNCH_WAIT;
    while Instant::now() < deadline {
        thread::sleep(POLL);
        let now_lines = st.list()?;
        let fresh = now_lines
            .iter()
            .find(|l| !before.contains(l) && score(&name, l, "") > 0)
            .or_else(|| now_lines.iter().find(|l| !before.contains(l)));
        if let Some(line) = fresh {
            let r = line.split(' ').next().unwrap_or("").to_string();
            st.launched.insert(r.clone(), name.clone());
            st.target = Some(r);
            return Ok(format!("ok (started) {}", line.trim_end_matches(" *")));
        }
    }
    Ok(format!("started {name}, no window yet -> try: wisply windows -f {name}"))
}

pub fn close(st: &mut State, r: &str) -> Result<bool, Fail> {
    let win = st.element(r)?;
    let button = walk(&win).into_iter().take(400).find(|e| {
        let subrole = e.raw.get("ax_subrole").and_then(|v| v.as_str()) == Some(CLOSE_SUBROLE);
        let named = xview::role(&e.role.to_string()) == Some("button")
            && CLOSE_NAMES.contains(&e.name.as_deref().unwrap_or("").to_lowercase().as_str());
        subrole || named
    });
    let Some(button) = button else { return Ok(false) };
    button.press().map_err(|e| fail(&e))?;
    let deadline = Instant::now() + CLOSE_WAIT;
    while Instant::now() < deadline {
        if st.element(r).is_err() {
            st.windows.remove(r);
            if st.target.as_deref() == Some(r) {
                st.target = None;
            }
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(false)
}

pub fn close_cmd(st: &mut State, c: &Call) -> Result<String, Fail> {
    let r = st.find(c.pos.first().map_or("", String::as_str))?;
    let head = st.header(&r, "");
    if close(st, &r)? {
        return Ok(format!("closed {head}"));
    }
    Ok(format!(
        "asked to close {head} (still open: no close button answered, it may show a save dialog) -> try: wisply snap {r}"
    ))
}
