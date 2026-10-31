use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use wisply_core::{Call, Fail, OUT_CHARS};
use xa11y::{App, AppExt, Element, Error, Toggled};

use super::super::ctx::{Ctx, SESSION_IDLE_S, now};
use super::super::web::{SNAP_LINES, capped, ref_kind};
use super::super::words::{best_only, terms, tiers};
use super::super::{Daemon, Out};
use super::view::{changes, fold, score, short};
use super::xact;
use super::xview::{self, Node};

const MAX_NODES: usize = 4000;
const SECRET_ROLES: &[&str] = &["AXSecureTextField", "password text"];
pub const FRONT_HINT: &str = "Wisply does not take the screen from the user; bring the window to the front yourself";
const PERMISSION_HINT: &str = if cfg!(target_os = "macos") {
    "allow the terminal in System Settings > Privacy & Security > Accessibility (and Screen Recording for shot)"
} else {
    "turn on accessibility (gsettings set org.gnome.desktop.interface toolkit-accessibility true) and restart the app"
};

pub struct Win {
    pub pid: u32,
    pub app: String,
    pub title: String,
}

#[derive(Default)]
pub struct State {
    pub windows: BTreeMap<String, Win>,
    pub target: Option<String>,
    pub refs: HashMap<String, Element>,
    pub ids: HashMap<String, String>,
    pub seq: u64,
    pub last_snap: HashMap<String, String>,
    pub last_lines: HashMap<String, (f64, Vec<String>)>,
    pub launched: BTreeMap<String, String>,
    used: f64,
}

thread_local! {
    static STATES: RefCell<HashMap<String, State>> = RefCell::new(HashMap::new());
}

pub fn with<T>(ctx: &mut Ctx, f: impl FnOnce(&mut State, &mut Ctx) -> T) -> T {
    STATES.with(|cell| {
        let mut map = cell.borrow_mut();
        let t = now();
        map.retain(|_, s| t - s.used <= SESSION_IDLE_S);
        let st = map.entry(ctx.sid.clone()).or_default();
        st.used = t;
        f(st, ctx)
    })
}

const BROWSER_APPS: &[&str] = &["chrome", "chromium", "brave", "edge", "vivaldi", "opera"];

pub fn browser_in_front() -> bool {
    App::list().is_ok_and(|apps| {
        apps.iter().any(|a| a.is_foreground() && BROWSER_APPS.iter().any(|b| a.name.to_lowercase().contains(b)))
    })
}

pub fn access() -> String {
    match App::list() {
        Ok(apps) => format!("apps ok ({} readable)", apps.len()),
        Err(e) => format!("apps missing: {}", fail(&e).text()),
    }
}

pub fn fail(e: &Error) -> Fail {
    match e {
        Error::PermissionDenied { .. } | Error::AccessibilityNotEnabled { .. } => {
            Fail::hint("this system does not let Wisply read apps yet", PERMISSION_HINT)
        }
        other => Fail::new(format!("{other}")),
    }
}

pub fn windows_of(app: &App) -> Vec<Element> {
    app.children()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| xview::role(&w.role.to_string()) == Some("window"))
        .collect()
}

pub fn node(el: &Element) -> Node {
    let raw = |k: &str| el.raw.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    Node {
        role: el.role.to_string(),
        name: el.name.clone().or_else(|| el.description.clone()).unwrap_or_default(),
        value: el.value.clone(),
        checked: el.states.checked.map(|t| match t {
            Toggled::On => Some(true),
            Toggled::Off => Some(false),
            Toggled::Mixed => None,
        }),
        selected: el.states.selected,
        expanded: el.states.expanded,
        enabled: el.states.enabled,
        secret: SECRET_ROLES.contains(&raw("ax_subrole").as_str())
            || SECRET_ROLES.contains(&raw("atspi_role").as_str()),
    }
}

pub fn walk(root: &Element) -> Vec<Element> {
    let mut out = Vec::new();
    let mut stack = root.children().unwrap_or_default();
    stack.reverse();
    while let Some(el) = stack.pop() {
        if out.len() >= MAX_NODES {
            break;
        }
        let mut kids = el.children().unwrap_or_default();
        kids.reverse();
        stack.extend(kids);
        out.push(el);
    }
    out
}

fn line_of(w: &Win, r: &str, front: bool) -> String {
    format!("{r} {} - {}{}", short(&w.title, 60), w.app, if front { " *" } else { "" })
}

impl State {
    fn wref(&mut self, pid: u32, app: &str, title: &str) -> String {
        if let Some((r, _)) = self.windows.iter().find(|(_, w)| w.pid == pid && w.title == title) {
            return r.clone();
        }
        let mut n = self.windows.len() + 1;
        while self.windows.contains_key(&format!("w{n}")) {
            n += 1;
        }
        let r = format!("w{n}");
        self.windows.insert(r.clone(), Win { pid, app: app.to_string(), title: title.to_string() });
        r
    }

    pub fn list(&mut self) -> Result<Vec<String>, Fail> {
        let mut rows = Vec::new();
        for app in App::list().map_err(|e| fail(&e))? {
            let (Some(pid), front) = (app.pid, app.is_foreground()) else { continue };
            for (i, w) in windows_of(&app).iter().enumerate() {
                let title = w.name.clone().unwrap_or_default();
                let r = self.wref(pid, &app.name, &title);
                let n: u64 = r[1..].parse().unwrap_or(0);
                rows.push((n, line_of(&self.windows[&r], &r, front && i == 0)));
            }
        }
        rows.sort();
        Ok(rows.into_iter().map(|(_, l)| l).collect())
    }

    pub fn find(&mut self, spec: &str) -> Result<String, Fail> {
        if ref_kind(spec) == Some("w") {
            if !self.windows.contains_key(spec) {
                self.list()?;
            }
            return self
                .windows
                .contains_key(spec)
                .then(|| spec.to_string())
                .ok_or_else(|| Fail::hint(format!("window {spec} is gone (closed)"), "wisply windows"));
        }
        let text = if spec.to_lowercase().starts_with("app:") { &spec[4..] } else { spec };
        self.list()?;
        let best = self
            .windows
            .iter()
            .map(|(r, w)| (score(text, &w.title, &w.app), r))
            .filter(|(s, _)| *s > 0)
            .max_by_key(|(s, _)| *s);
        best.map(|(_, r)| r.clone())
            .ok_or_else(|| Fail::hint(format!("no window matches \"{text}\""), "wisply windows"))
    }

    pub fn element(&mut self, r: &str) -> Result<Element, Fail> {
        let gone = || Fail::hint(format!("window {r} is gone (closed)"), "wisply windows");
        let w = self.windows.get_mut(r).ok_or_else(gone)?;
        let app = App::by_pid(w.pid, Duration::ZERO).map_err(|_| gone())?;
        let wins = windows_of(&app);
        let same = wins.iter().position(|e| e.name.as_deref().unwrap_or("") == w.title);
        let pick = same.or_else(|| (wins.len() == 1).then_some(0)).ok_or_else(gone)?;
        let el = wins[pick].clone();
        w.title = el.name.clone().unwrap_or_default();
        Ok(el)
    }

    pub fn target(&mut self, first: Option<&str>) -> Result<String, Fail> {
        let r = match first.filter(|p| super::windowish(p)) {
            Some(spec) => self.find(spec)?,
            None => self.target.clone().filter(|r| self.windows.contains_key(r)).ok_or_else(|| {
                Fail::hint(
                    "no window chosen",
                    "wisply snap app:<name> picks that app's window (several: wisply windows -f <app>)",
                )
            })?,
        };
        self.target = Some(r.clone());
        Ok(r)
    }

    pub fn header(&self, r: &str, extra: &str) -> String {
        let w = &self.windows[r];
        format!("[{r}] {} - {}{extra}", short(&w.title, 70), w.app)
    }

    fn next_ref(&mut self, key: String) -> String {
        if let Some(r) = self.ids.get(&key) {
            return r.clone();
        }
        self.seq += 1;
        let r = format!("d{}", self.seq);
        self.ids.insert(key, r.clone());
        r
    }

    pub fn snapshot(&mut self, r: &str) -> Result<(String, Vec<String>), Fail> {
        let root = self.element(r)?;
        let mut lines = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut controls = 0;
        for el in walk(&root) {
            let n = node(&el);
            let Some(row) = xview::line(&n) else { continue };
            let text = match row {
                xview::Row::Text(_) => row.with_ref(""),
                xview::Row::Control(_) => {
                    let base = format!("{r}|{}|{}", n.role, n.name);
                    let k = seen.entry(base.clone()).or_insert(0);
                    *k += 1;
                    let key = el.stable_id.clone().map_or_else(|| format!("{base}|{k}"), |id| format!("{r}|{id}"));
                    let dref = self.next_ref(key);
                    self.refs.insert(dref.clone(), el);
                    controls += 1;
                    row.with_ref(&dref)
                }
            };
            lines.push(text);
        }
        let head = self.header(r, &format!(" ({controls} controls)"));
        Ok((head, lines))
    }

    pub fn el_ref(&mut self, spec: &str) -> Result<(String, Element), Fail> {
        if ref_kind(spec) == Some("d") {
            let el =
                self.refs.get(spec).cloned().ok_or_else(|| Fail::hint(format!("{spec} is unknown"), "wisply snap"))?;
            return Ok((spec.to_string(), el));
        }
        let r = self.target(None)?;
        let (_, lines) = self.snapshot(&r)?;
        let want = spec.to_lowercase();
        let controls: Vec<String> = lines.iter().filter(|l| l.starts_with('d')).map(|l| l.to_lowercase()).collect();
        let quoted = format!("\"{want}\"");
        let at = controls
            .iter()
            .position(|l| l.contains(&quoted))
            .or_else(|| controls.iter().position(|l| l.contains(&want)));
        let hit = at.map(|i| &controls[i]);
        let dref = hit
            .and_then(|l| l.split(' ').next())
            .ok_or_else(|| Fail::hint(format!("no control matches \"{spec}\""), "wisply snap"))?;
        let el = self.refs[dref].clone();
        Ok((dref.to_string(), el))
    }
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call, aim: Option<String>) -> Result<Out, Fail> {
    with(ctx, |st, ctx| {
        if let Some(spec) = aim.as_deref() {
            st.target(Some(spec))?;
        }
        let text = match c.name.as_str() {
            "windows" | "desktop" => windows(d, st, ctx, c),
            "focus" => focus(st, ctx, c),
            "launch" => xact::launch(st, ctx, c),
            "close" => xact::close_cmd(st, c),
            "snap" => snap(d, st, ctx, c),
            "click" => xact::click(st, c),
            "type" => xact::type_in(st, ctx, c),
            "press" => xact::press(st, c),
            "scroll" => xact::scroll(st, c),
            "read" | "text" => xact::read(d, st, ctx, c),
            "shot" => xact::shot(d, st, ctx, c),
            "clip" => xact::clip(d, ctx, c),
            "hover" | "drag" => Err(Fail::hint(
                format!("{} on desktop apps would steal your mouse", c.name),
                "use wisply click or scroll",
            )),
            other => Err(super::super::route::later(&format!("{other} on apps on this system"))),
        }?;
        if let Some(r) = &st.target {
            ctx.target = Some(("app".into(), r.clone()));
        }
        Ok(Out::Text(text))
    })
}

fn windows(d: &Daemon, st: &mut State, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let mut lines = st.list()?;
    if let Some(f) = c.val("f").filter(|f| !f.is_empty()) {
        let filt = terms(f);
        let rows: Vec<(Vec<u8>, &str)> = lines
            .iter()
            .map(|l| (tiers(&filt, &[l.as_str()]), l.as_str()))
            .filter(|(t, _)| t.iter().any(|x| *x > 0))
            .collect();
        lines = best_only(&rows).into_iter().map(String::from).collect();
        if let [one] = lines.as_mut_slice() {
            st.target = one.split(' ').next().map(String::from);
            *one += "  (chosen: app commands without a window go here)";
        }
    }
    let text = if lines.is_empty() { "no windows".to_string() } else { lines.join("\n") };
    Ok(capped(d, ctx, &text, "windows", Some(80), OUT_CHARS))
}

fn focus(st: &mut State, _: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let first = c.pos.first().ok_or_else(|| Fail::hint("focus needs a window", "wisply focus w2"))?;
    let r = st.target(Some(first))?;
    Ok(format!("ok (target set, the window stays where it is: {FRONT_HINT}) {}", st.header(&r, "")))
}

fn snap(d: &Daemon, st: &mut State, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let r = st.target(c.pos.first().map(String::as_str))?;
    let (head, lines) = st.snapshot(&r)?;
    let (folded, _) = fold(&lines);
    let text =
        format!("{head}\n{}", if folded.is_empty() { "(no controls found)".to_string() } else { folded.join("\n") });
    if ["f", "text", "force", "all", "full"].iter().any(|k| c.flags.contains_key(*k)) {
        st.last_lines.insert(r.clone(), (now(), lines));
        return Ok(capped(d, ctx, &text, "app-snap", Some(SNAP_LINES), OUT_CHARS));
    }
    if st.last_snap.get(&r) == Some(&text) {
        return Ok(format!("{head}\nunchanged since last snap (refs still valid)"));
    }
    st.last_snap.insert(r.clone(), text.clone());
    let old = st.last_lines.insert(r.clone(), (now(), lines.clone()));
    if let Some(delta) = changes(old.as_ref(), &lines, &r, now()) {
        let shown: usize = text.chars().count().min(OUT_CHARS);
        ctx.cut += shown.saturating_sub(delta.chars().count());
        return Ok(capped(d, ctx, &format!("{head}\n{delta}"), "app-snap", Some(SNAP_LINES), OUT_CHARS));
    }
    Ok(capped(d, ctx, &text, "app-snap", Some(SNAP_LINES), OUT_CHARS))
}

pub fn cleanup(ctx: &mut Ctx) -> (Vec<String>, Vec<String>) {
    with(ctx, |st, _| {
        let launched = std::mem::take(&mut st.launched);
        let (mut closed, mut left) = (Vec::new(), Vec::new());
        for (r, name) in launched {
            if xact::close(st, &r).unwrap_or(false) { closed.push(name) } else { left.push(name) }
        }
        (closed, left)
    })
}

pub fn keep(_: &Daemon, ctx: &mut Ctx, spec: Option<&str>) -> Result<usize, Fail> {
    with(ctx, |st, _| {
        let Some(spec) = spec else {
            let n = st.launched.len();
            st.launched.clear();
            return Ok(n);
        };
        let r = st.find(spec)?;
        Ok(usize::from(st.launched.remove(&r).is_some()))
    })
}
