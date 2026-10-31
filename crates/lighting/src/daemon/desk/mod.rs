#[cfg(any(windows, test))]
mod combo;
mod view;
#[cfg(not(windows))]
mod xa;
#[cfg(not(windows))]
mod xact;
#[cfg(any(not(windows), test))]
mod xview;

#[cfg(any(windows, test))]
mod talk;

#[cfg(windows)]
mod act;
#[cfg(windows)]
mod autoload;
#[cfg(windows)]
mod chat;
#[cfg(windows)]
mod focus;
#[cfg(windows)]
mod grab;
#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod input;
#[cfg(windows)]
mod keyway;
#[cfg(windows)]
mod launch;
#[cfg(windows)]
mod look;
#[cfg(windows)]
mod ocr;
#[cfg(windows)]
pub mod probe;
#[cfg(windows)]
mod rec;
#[cfg(windows)]
mod snap;
#[cfg(windows)]
mod state;
#[cfg(windows)]
mod tab;
#[cfg(windows)]
mod uia;
#[cfg(windows)]
mod video;
#[cfg(windows)]
mod win;

#[cfg(windows)]
use lt_core::Fail;
#[cfg(not(windows))]
pub use xa::{access, browser_in_front, cleanup, keep, run};

#[cfg(not(windows))]
fn windowish(p: &str) -> bool {
    super::web::ref_kind(p) == Some("w") || p.to_lowercase().starts_with("app:")
}

#[cfg(windows)]
pub use autoload::autoload;
#[cfg(windows)]
pub use imp::{cleanup, keep, run};
#[cfg(windows)]
pub use rec::Recorder;
#[cfg(windows)]
pub use tab::{find_tab, tab_text};

#[cfg(windows)]
pub enum Halt {
    Fail(Fail),
    Busy,
}

#[cfg(windows)]
impl From<Fail> for Halt {
    fn from(f: Fail) -> Halt {
        Halt::Fail(f)
    }
}

#[cfg(windows)]
pub type Res<T> = Result<T, Halt>;

#[cfg(windows)]
pub use imp::{Desk, TERMINALS, windowish};

#[cfg(windows)]
pub fn browser_in_front() -> bool {
    let fg = win::foreground();
    fg != 0 && Desk::is_browser(fg)
}

#[cfg(windows)]
pub fn access() -> String {
    "apps ok (Windows UI Automation needs no permission)".into()
}

#[cfg(windows)]
pub fn app_where(hwnd: isize) -> Option<(String, String)> {
    win::alive(hwnd).then(|| (win::exe_of_window(hwnd), win::text_of(hwnd)))
}

#[cfg(not(windows))]
pub fn app_where(_: isize) -> Option<(String, String)> {
    None
}

#[cfg(windows)]
pub fn ocr_lines(img: &image::RgbImage) -> Result<Vec<String>, Fail> {
    ocr::recognize(img, "auto").map(|lines| lines.into_iter().map(|l| l.0).collect())
}

#[cfg(windows)]
mod imp {
    use std::thread;
    use std::time::Duration;

    use lt_core::{Call, Fail, OUT_CHARS};

    use super::super::ctx::Ctx;
    use super::super::web::{TEXT_CHARS, capped, ref_kind};
    use super::super::words::{best_only, terms, tiers};
    use super::super::{Daemon, Out, Pending};
    use super::state::{self, State};
    use super::view::{score, short};
    use super::{Halt, Res, act, chat, focus, input, launch, look, snap, video, win};

    const BUSY_WAIT_S: u64 = 10;
    const CHROMIUM_CLASS: &str = "Chrome_WidgetWin_1";
    pub const BROWSER_EXES: &[&str] = &[
        "brave.exe",
        "chrome.exe",
        "msedge.exe",
        "opera.exe",
        "vivaldi.exe",
        "brave browser",
        "google chrome",
        "microsoft edge",
        "chromium",
        "opera",
        "vivaldi",
        "brave",
        "chrome",
        "msedge",
        "chromium-browser",
        "google-chrome",
        "brave-browser",
    ];
    pub const TERMINALS: &[&str] =
        &["CASCADIA_HOSTING_WINDOW_CLASS", "ConsoleWindowClass", "mintty", "VirtualConsoleClass", "PuTTY", "terminal"];
    const SHELL_CLASSES: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "desktop"];
    const AGENT_EXES: &[&str] = &[
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
    ];
    pub const CHAT_EXES: &[&str] =
        &["discord.exe", "slack.exe", "whatsapp.exe", "ms-teams.exe", "telegram.exe", "signal.exe"];

    pub struct Desk<'a> {
        pub d: &'a Daemon,
        pub st: &'a mut State,
        pub ctx: &'a mut Ctx,
    }

    pub fn windowish(p: &str) -> bool {
        ref_kind(p) == Some("w") || p.to_lowercase().starts_with("app:")
    }

    pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call, aim: Option<String>) -> Result<Out, Fail> {
        if c.name == "wait" {
            return state::with(ctx, |st, ctx| look::wait(&mut Desk { d, st, ctx }, c, aim.as_deref()));
        }
        if c.name == "shot" && c.on("seconds") {
            return state::with(ctx, |st, ctx| video::video(&mut Desk { d, st, ctx }, c));
        }
        let said = if lt_core::CHAT.contains(&c.name.as_str()) {
            state::with(ctx, |st, ctx| chat::run(&mut Desk { d, st, ctx }, c, aim.as_deref(), true))
        } else {
            text_cmd(d, ctx, c, aim.as_deref()).map(Out::Text)
        };
        match said {
            Ok(out) => Ok(out),
            Err(Halt::Fail(f)) => Err(f),
            Err(Halt::Busy) => Ok(Out::Wait(when_idle(c.clone(), aim))),
        }
    }

    fn when_idle(c: Call, aim: Option<String>) -> Pending {
        Pending::new(Duration::from_secs(BUSY_WAIT_S), Duration::from_millis(150), move |d, ctx, last| {
            if focus::idle_now() {
                match text_cmd(d, ctx, &c, aim.as_deref()) {
                    Ok(text) => return Ok(Some(text)),
                    Err(Halt::Fail(f)) => return Err(f),
                    Err(Halt::Busy) => {}
                }
            }
            if last {
                return Err(Fail::hint(
                    format!("you kept using mouse or keyboard for {BUSY_WAIT_S} s, so nothing was sent"),
                    "retry when the user is done, or ask them to pause for a moment",
                ));
            }
            Ok(None)
        })
    }

    fn text_cmd(d: &Daemon, ctx: &mut Ctx, c: &Call, aim: Option<&str>) -> Res<String> {
        state::with(ctx, |st, ctx| {
            let mut x = Desk { d, st, ctx };
            if let Some(spec) = aim {
                x.target(Some(spec))?;
            }
            match c.name.as_str() {
                "windows" | "desktop" => x.windows(c),
                "focus" => x.focus_cmd(c),
                "launch" => launch::launch(&mut x, c),
                "close" => x.close(c),
                "snap" => snap::snap(&mut x, c),
                "click" => act::click(&mut x, c),
                "type" => act::type_in(&mut x, c),
                "press" => act::press(&mut x, c),
                "scroll" => act::scroll(&mut x, c),
                "drag" => act::drag(&mut x, c),
                "hover" => Err(Fail::hint(
                    "hover on desktop apps would steal your mouse",
                    "use wisply shot or read to look, or click",
                )
                .into()),
                "read" | "text" => look::read(&mut x, c),
                "shot" => look::shot(&mut x, c),
                "clip" => x.clip(c),
                "inbox" | "reply" | "unread" => match chat::run(&mut x, c, aim, false)? {
                    Out::Text(text) => Ok(text),
                    _ => Ok(String::new()),
                },
                other => Err(Fail::new(format!("{other} does not work on apps")).into()),
            }
        })
    }

    pub fn cleanup(ctx: &mut Ctx) -> (Vec<String>, Vec<String>) {
        state::with(ctx, launch::cleanup)
    }

    pub fn keep(d: &Daemon, ctx: &mut Ctx, spec: Option<&str>) -> Result<usize, Fail> {
        let kept = state::with(ctx, |st, ctx| {
            let Some(spec) = spec else {
                let n = st.launched.len();
                st.launched.clear();
                return Ok(n);
            };
            let hwnd = Desk { d, st: &mut *st, ctx: &mut *ctx }.find_window(spec)?;
            Ok(usize::from(st.launched.remove(&hwnd).is_some()))
        });
        kept.map_err(|h| match h {
            Halt::Fail(f) => f,
            Halt::Busy => Fail::new("busy"),
        })
    }

    impl Desk<'_> {
        pub fn bg(&self, hwnd: isize) -> bool {
            hwnd != 0 && !focus::front_ok(&self.d.home)
        }

        pub fn note(&mut self, text: &str) {
            self.ctx.note(text);
        }

        pub fn cap(&mut self, text: &str, name: &str, lines: Option<usize>, chars: usize) -> String {
            capped(self.d, self.ctx, text, name, lines, chars)
        }

        pub fn list_windows(&mut self) -> Vec<String> {
            let mut lines = Vec::new();
            for w in win::windows() {
                let r = match self.st.window_ref(w.hwnd) {
                    Some(r) => r,
                    None => {
                        let mut n = self.st.windows.len() + 1;
                        while self.st.windows.contains_key(&format!("w{n}")) {
                            n += 1;
                        }
                        self.st.windows.insert(format!("w{n}"), w.hwnd);
                        format!("w{n}")
                    }
                };
                self.st.info.insert(r.clone(), (w.exe.to_lowercase(), w.title.clone()));
                if w.fg {
                    self.st.seen_front = w.hwnd;
                }
                let flag = format!("{}{}", if w.fg { " *" } else { "" }, if w.min { " (min)" } else { "" });
                let who = if w.exe.is_empty() { &w.cls } else { &w.exe };
                let n: u64 = r[1..].parse().unwrap_or(0);
                lines.push((n, format!("{r} {} - {who}{flag}", short(&w.title, 60))));
            }
            lines.sort();
            lines.into_iter().map(|(_, l)| l).collect()
        }

        fn successor(&self, spec: &str) -> Option<isize> {
            let (exe, title) = self.st.info.get(spec)?;
            let mut same: Vec<win::Win> = win::windows().into_iter().filter(|w| w.exe.to_lowercase() == *exe).collect();
            if same.len() > 1 {
                same.retain(|w| w.title == *title);
            }
            (same.len() == 1).then(|| same[0].hwnd)
        }

        pub fn find_window(&mut self, spec: &str) -> Res<isize> {
            if ref_kind(spec) == Some("w") {
                if !self.st.windows.contains_key(spec) {
                    self.list_windows();
                }
                let hwnd = self.st.windows.get(spec).copied().unwrap_or(0);
                if hwnd != 0
                    && !win::alive(hwnd)
                    && let Some(new) = self.successor(spec)
                {
                    self.st.windows.insert(spec.to_string(), new);
                    let exe = self.st.info.get(spec).map(|i| i.0.clone()).unwrap_or_default();
                    self.note(&format!("the old {spec} was closed; {spec} is now the new {exe} window"));
                    return Ok(new);
                }
                if hwnd == 0 || !win::alive(hwnd) {
                    return Err(Fail::hint(format!("window {spec} is gone (closed)"), "wisply windows").into());
                }
                return Ok(hwnd);
            }
            let text = if spec.to_lowercase().starts_with("app:") { &spec[4..] } else { spec };
            let mut best: Option<(u8, isize)> = None;
            for w in win::windows() {
                let s = score(text, &w.title, &w.exe);
                if s > 0 && best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, w.hwnd));
                }
            }
            let Some((_, hwnd)) = best else {
                return Err(Fail::hint(format!("no window matches \"{text}\""), "wisply windows").into());
            };
            self.list_windows();
            Ok(hwnd)
        }

        fn front_app(&self) -> Option<isize> {
            let hwnd = win::foreground();
            let cls = win::class_of(hwnd);
            if hwnd == 0
                || hwnd != self.st.seen_front
                || !win::alive(hwnd)
                || TERMINALS.contains(&cls.as_str())
                || SHELL_CLASSES.contains(&cls.as_str())
            {
                return None;
            }
            let pid = win::pid_of(hwnd);
            let exe = win::exe_of(pid).to_lowercase();
            let skip = [AGENT_EXES, BROWSER_EXES, CHAT_EXES].iter().any(|list| list.contains(&exe.as_str()));
            (!exe.is_empty() && !skip && !win::blocked(pid)).then_some(hwnd)
        }

        pub fn target(&mut self, first: Option<&str>) -> Res<isize> {
            let hwnd = match first.filter(|p| windowish(p)) {
                Some(spec) => self.find_window(spec)?,
                None if self.st.hwnd != 0 && win::alive(self.st.hwnd) => self.st.hwnd,
                None => {
                    let Some(hwnd) = self.front_app() else {
                        return Err(Fail::hint(
                            "no window chosen",
                            "wisply snap app:<name> picks that app's window (several: wisply windows -f <app>)",
                        )
                        .into());
                    };
                    let label = self.label(hwnd);
                    self.note(&format!("no window was chosen: used {label}, still in front since your last windows"));
                    hwnd
                }
            };
            self.st.hwnd = hwnd;
            self.ctx.target = Some(("app".into(), hwnd.to_string()));
            Ok(hwnd)
        }

        pub fn label(&mut self, hwnd: isize) -> String {
            let title = short(&win::text_of(hwnd), 40);
            format!("{} {title} - {}", self.wref(hwnd), win::exe_of_window(hwnd))
        }

        pub fn wref(&mut self, hwnd: isize) -> String {
            if let Some(r) = self.st.window_ref(hwnd) {
                return r;
            }
            self.list_windows();
            self.st.window_ref(hwnd).unwrap_or_else(|| "w?".into())
        }

        pub fn header(&mut self, hwnd: isize, extra: &str) -> String {
            let title = short(&win::text_of(hwnd), 70);
            format!("[{}] {title} - {}{extra}", self.wref(hwnd), win::exe_of_window(hwnd))
        }

        pub fn is_browser(hwnd: isize) -> bool {
            win::class_of(hwnd) == CHROMIUM_CLASS
                && BROWSER_EXES.contains(&win::exe_of_window(hwnd).to_lowercase().as_str())
        }

        pub fn is_chromium(hwnd: isize) -> bool {
            win::class_of(hwnd) == CHROMIUM_CLASS
        }

        fn choose(&mut self, r: &str) -> bool {
            let Some(hwnd) = self.st.windows.get(r).copied() else { return false };
            if !win::alive(hwnd) || TERMINALS.contains(&win::class_of(hwnd).as_str()) || Desk::is_browser(hwnd) {
                return false;
            }
            if AGENT_EXES.contains(&win::exe_of_window(hwnd).to_lowercase().as_str()) {
                return false;
            }
            self.st.hwnd = hwnd;
            self.ctx.target = Some(("app".into(), hwnd.to_string()));
            true
        }

        fn windows(&mut self, c: &Call) -> Res<String> {
            let mut lines = self.list_windows();
            if let Some(f) = c.val("f").filter(|f| !f.is_empty()) {
                let filt = terms(f);
                let rows: Vec<(Vec<u8>, &str)> = lines
                    .iter()
                    .map(|l| (tiers(&filt, &[l.as_str()]), l.as_str()))
                    .filter(|(t, _)| t.iter().any(|x| *x > 0))
                    .collect();
                lines = best_only(&rows).into_iter().map(String::from).collect();
                if lines.len() == 1 {
                    let r = lines[0].split(' ').next().unwrap_or("").to_string();
                    if self.choose(&r) {
                        lines[0] += "  (chosen: app commands without a window go here)";
                    }
                }
            }
            let text = if lines.is_empty() { "no windows".to_string() } else { lines.join("\n") };
            Ok(self.cap(&text, "windows", Some(80), OUT_CHARS))
        }

        pub fn focus_cmd(&mut self, c: &Call) -> Res<String> {
            if c.pos.is_empty() {
                return Err(Fail::hint("focus needs a window", "wisply focus w2").into());
            }
            let hwnd = self.target(c.pos.first().map(String::as_str))?;
            if self.bg(hwnd) && win::foreground() != hwnd {
                let head = self.header(hwnd, "");
                return Ok(format!("ok (target set, the window stays where it is: {}) {head}", input::FRONT_HINT));
            }
            let ok = focus::front(hwnd);
            let head = self.header(hwnd, "");
            Ok(format!("{}{head}", if ok { "ok " } else { "err: the system refused to switch, " }))
        }

        fn close(&mut self, c: &Call) -> Res<String> {
            let hwnd = self.find_window(c.pos.first().map_or("", String::as_str))?;
            let head = self.header(hwnd, "");
            win::close(hwnd);
            for _ in 0..20 {
                if !win::alive(hwnd) || !win::visible(hwnd) {
                    return Ok(format!("closed {head}"));
                }
                thread::sleep(Duration::from_millis(100));
            }
            Ok(format!("asked to close {head} (still open, it may show a save dialog) -> try: wisply windows"))
        }

        fn clip(&mut self, c: &Call) -> Res<String> {
            if c.pos.first().is_some_and(|p| p == "set") {
                let text = self.ctx.secret.clone().unwrap_or_else(|| c.pos[1..].join(" "));
                win::clip_set(&text);
                return Ok("ok (clipboard set)".into());
            }
            let Some(text) = win::clip_get() else { return Err(Fail::new("clipboard is busy").into()) };
            let text = if text.is_empty() { "(clipboard is empty)".to_string() } else { text };
            Ok(self.cap(&text, "clip", None, TEXT_CHARS))
        }
    }
}
