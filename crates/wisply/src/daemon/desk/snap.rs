use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::{Value, json};
use wisply_core::{Call, Fail, OUT_CHARS};

use super::super::brain;
use super::super::ctx::now;
use super::super::web::{SNAP_LINES, keys};
use super::super::words::{best_only, terms, tiers};
use super::uia::{self, El};
use super::view::{changes, fold, short};
use super::win::{self, Rect};
use super::{Desk, Res};

const DIFF_LINES: usize = 5;
const FRESH_S: f64 = 1.5;
const SLOW: Duration = Duration::from_millis(500);
const NAMELESS: &[&str] = &["listitem", "dataitem", "treeitem", "tab", "button", "link", "menuitem"];
const ROLE_RANK: &[(&str, u8)] = &[
    ("button", 0),
    ("link", 1),
    ("menuitem", 2),
    ("splitbutton", 2),
    ("checkbox", 3),
    ("radio", 3),
    ("tab", 3),
    ("edit", 4),
    ("combobox", 4),
    ("treeitem", 5),
    ("listitem", 6),
    ("dataitem", 6),
];

static BIDI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("[\u{200e}\u{200f}\u{202a}-\u{202e}\u{2066}-\u{2069}]").expect("bidi"));
static HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*\(((?:ctrl|strg|alt|shift|umschalt|win|cmd)[ +-][^)]{1,20}|[a-z0-9] then [a-z0-9])\)\s*$")
        .expect("hint")
});
static VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"="([^"]*)""#).expect("value"));

#[derive(Default)]
pub struct Look {
    pub text: bool,
    pub all: bool,
    pub web: bool,
    pub f: Option<String>,
}

impl Look {
    pub fn of(c: &Call) -> Look {
        Look {
            text: c.on("text"),
            all: c.on("all"),
            web: c.on("web"),
            f: c.val("f").filter(|f| !f.is_empty()).map(String::from),
        }
    }
}

fn from_name(name: &str) -> Option<(String, String)> {
    let m = HINT.captures(name)?;
    Some((m[1].to_string(), HINT.replace(name, "").into_owned()))
}

fn inside(a: Rect, b: Rect) -> bool {
    a.1 >= b.1 - 2 && a.3 <= b.3 + 2 && a.0 >= b.0 - 2
}

fn rank(role: &str) -> u8 {
    ROLE_RANK.iter().find(|(r, _)| *r == role).map_or(9, |(_, n)| *n)
}

impl Desk<'_> {
    pub fn snapshot(&mut self, hwnd: isize, look: &Look) -> Res<String> {
        if win::blocked(win::pid_of(hwnd)) {
            return Err(Fail::hint(
                "this window runs as administrator; Windows blocks control from normal apps",
                "start the terminal as administrator, or use wisply read / shot",
            )
            .into());
        }
        let skip = if Desk::is_browser(hwnd) && !look.web { uia::page_rect(hwnd) } else { None };
        let mut items = uia::collect(hwnd, look.text, skip);
        if items.len() < 3 && Desk::is_chromium(hwnd) {
            thread::sleep(Duration::from_millis(400));
            items = uia::collect(hwnd, look.text, skip);
        }
        let filt = look.f.as_deref().map(terms);
        let mut lines: Vec<String> = Vec::new();
        let mut refs: Vec<(String, El)> = Vec::new();
        let mut row: Option<(usize, Rect, String)> = None;
        let mut ft: HashMap<usize, Vec<u8>> = HashMap::new();
        let mut pairs: Vec<Value> = Vec::new();
        for el in items {
            let (role, name, extra) = el.describe();
            let (mut name, extra) =
                (BIDI.replace_all(&name, "").into_owned(), BIDI.replace_all(&extra, "").into_owned());
            if let Some((combo, rest)) = from_name(&name) {
                pairs.push(json!([combo, rest]));
                name = rest;
            } else if let Some(ak) = el.shortcut().filter(|_| !name.is_empty()) {
                pairs.push(json!([ak, name]));
            }
            if role == "text" && (name.is_empty() || name.chars().count() > 120) {
                continue;
            }
            if name.is_empty() && NAMELESS.contains(&role) && extra.is_empty() {
                continue;
            }
            let rect = el.rect();
            let key = match el.runtime_id() {
                Some(id) if !id.is_empty() => format!("r:{id:?}"),
                _ => format!("k:{role}|{name}|{rect:?}"),
            };
            let r = self.st.next_ref(key);
            refs.push((r.clone(), el));
            if let Some((at, area, row_name)) = &row
                && (role == "edit" || role == "text")
                && inside(rect, *area)
            {
                let val = VALUE
                    .captures(&extra)
                    .map(|m| m[1].to_string())
                    .unwrap_or_else(|| if role == "text" { name.clone() } else { String::new() });
                if !val.is_empty() && val != *row_name && lines[*at].chars().count() < 160 {
                    lines[*at] += &format!(" | {val}");
                }
                continue;
            }
            row = None;
            if let Some(filt) = &filt {
                let t = tiers(filt, &[name.as_str(), role, extra.as_str()]);
                if !t.iter().any(|x| *x > 0) {
                    continue;
                }
                ft.insert(lines.len(), t);
            }
            if role == "text" {
                lines.push(format!("  {name}"));
                continue;
            }
            let label = short(&name, 60);
            let quoted = if label.is_empty() { String::new() } else { format!(" \"{}\"", label.replace('"', "'")) };
            lines.push(format!("{r} {role}{quoted}{extra}"));
            if role == "listitem" || role == "dataitem" {
                row = Some((lines.len() - 1, rect, name));
            }
        }
        let count = refs.len();
        self.st.refs = refs;
        self.st.snap_t = now();
        self.st.snap_hwnd = hwnd;
        if !pairs.is_empty() {
            keys::harvest(&self.d.home, &win::exe_of_window(hwnd).to_lowercase(), &pairs);
        }
        let mut folded = 0;
        if filt.is_some() {
            let rows: Vec<(Vec<u8>, &str)> =
                lines.iter().enumerate().map(|(i, l)| (ft.get(&i).cloned().unwrap_or_default(), l.as_str())).collect();
            lines = best_only(&rows).into_iter().map(String::from).collect();
        } else {
            if !look.text {
                self.st.full = Some((hwnd, now(), lines.clone()));
            }
            if !look.all {
                (lines, folded) = fold(&lines);
            }
        }
        let hidden = if skip.is_some() { ", browser page hidden: use wisply snap for the page" } else { "" };
        let head = self.header(hwnd, &format!(" ({count} controls{hidden})"));
        let wref = self.wref(hwnd);
        if lines.is_empty() {
            lines.push(format!("(no controls found: try wisply read {wref} for screen text)"));
        }
        if folded > 0 {
            lines.push(format!("({folded} repeated lines folded: wisply snap {wref} --all)"));
        }
        Ok(format!("{head}\n{}", lines.join("\n")))
    }

    fn full_lines(&self, hwnd: isize) -> Option<&Vec<String>> {
        self.st.full.as_ref().filter(|f| f.0 == hwnd).map(|f| &f.2)
    }

    pub fn before_action(&mut self, hwnd: isize) -> Option<HashSet<String>> {
        if self.st.slow.contains(&hwnd) || !win::alive(hwnd) {
            return None;
        }
        if let Some((h, at, lines)) = &self.st.full
            && *h == hwnd
            && now() - at < FRESH_S
        {
            return Some(lines.iter().cloned().collect());
        }
        let t0 = Instant::now();
        self.snapshot(hwnd, &Look::default()).ok()?;
        if t0.elapsed() > SLOW {
            self.st.slow.insert(hwnd);
            return None;
        }
        self.full_lines(hwnd).map(|l| l.iter().cloned().collect())
    }

    pub fn after_action(
        &mut self,
        hwnd: isize,
        before: Option<HashSet<String>>,
        wait: f64,
        quiet: Option<f64>,
    ) -> String {
        let Some(before) = before else { return String::new() };
        let t0 = Instant::now();
        let (mut last, mut seen, mut lines): (Option<HashSet<String>>, HashSet<String>, Vec<String>) =
            (None, before.clone(), Vec::new());
        while t0.elapsed().as_secs_f64() < wait {
            thread::sleep(Duration::from_millis(100));
            if !win::alive(hwnd) {
                return " (window closed)".into();
            }
            if self.snapshot(hwnd, &Look::default()).is_err() {
                return String::new();
            }
            let Some(now_lines) = self.full_lines(hwnd) else { return String::new() };
            lines.clone_from(now_lines);
            seen = lines.iter().cloned().collect();
            if seen != before {
                if last.as_ref() == Some(&seen) {
                    break;
                }
                last = Some(seen.clone());
            } else if last.is_none() && quiet.is_some_and(|q| t0.elapsed().as_secs_f64() >= q) {
                break;
            }
        }
        if seen == before {
            return String::new();
        }
        let added: Vec<&String> = lines.iter().filter(|l| !before.contains(*l)).collect();
        let gone = before.difference(&seen).count();
        if added.is_empty() {
            return format!(" (-{gone} gone)");
        }
        let gone_text = if gone > 0 { format!(", -{gone} gone") } else { String::new() };
        let shown: Vec<&str> = added.iter().take(DIFF_LINES).map(|s| s.as_str()).collect();
        let mut s = format!(" (+{} new{gone_text})\n{}", added.len(), shown.join("\n"));
        if added.len() > shown.len() {
            s += &format!("\n... +{} more (wisply snap {})", added.len() - shown.len(), self.wref(hwnd));
        }
        s
    }

    pub fn element(&self, r: &str) -> Res<El> {
        self.st
            .refs
            .iter()
            .find(|(k, _)| k == r)
            .map(|(_, el)| el.clone())
            .ok_or_else(|| Fail::hint(format!("{r} is unknown or from an older snapshot"), "wisply snap").into())
    }

    pub fn center(el: &El) -> Res<(i32, i32)> {
        let (l, t, r, b) = el.live_rect();
        if r - l < 1 {
            return Err(Fail::hint("element has no position on screen", "wisply snap").into());
        }
        Ok(((l + r) / 2, (t + b) / 2))
    }

    pub fn by_text(&mut self, hwnd: isize, text: &str, roles: Option<&[&str]>) -> Res<Option<(String, El)>> {
        let tl = text.to_lowercase();
        for attempt in 0..2 {
            if attempt > 0 || self.st.refs.is_empty() || now() - self.st.snap_t > FRESH_S || self.st.snap_hwnd != hwnd {
                self.snapshot(hwnd, &Look::default())?;
            }
            let mut best: Option<(u8, u8, String, El)> = None;
            for (r, el) in &self.st.refs {
                let role = el.role();
                if roles.is_some_and(|list| !list.contains(&role)) {
                    continue;
                }
                let name = el.name().to_lowercase();
                let hit = if name == tl { 4 } else { tiers(std::slice::from_ref(&tl), &[name.as_str()])[0] };
                if hit == 0 {
                    continue;
                }
                let better = best.as_ref().is_none_or(|(h, k, ..)| hit > *h || (hit == *h && rank(role) < *k));
                if better {
                    best = Some((hit, rank(role), r.clone(), el.clone()));
                }
            }
            if let Some((_, _, r, el)) = best {
                return Ok(Some((r, el)));
            }
        }
        Ok(None)
    }
}

pub fn snap(x: &mut Desk, c: &Call) -> Res<String> {
    let hwnd = x.target(c.pos.first().map(String::as_str))?;
    let look = Look::of(c);
    let text = x.snapshot(hwnd, &look)?;
    if ["f", "text", "force", "all"].iter().any(|k| c.flags.contains_key(*k)) {
        return Ok(x.cap(&text, "app-snap", Some(SNAP_LINES), OUT_CHARS));
    }
    let (head, rest) = text.split_once('\n').unwrap_or((text.as_str(), ""));
    if x.st.last_snap.get(&hwnd) == Some(&text) {
        return Ok(format!("{head}\nunchanged since last snap (refs still valid)"));
    }
    x.st.last_snap.insert(hwnd, text.clone());
    let exe = win::exe_of_window(hwnd).to_lowercase();
    let lines = x.full_lines(hwnd).cloned();
    let old = match &lines {
        Some(l) => x.st.last_lines.insert(hwnd, (now(), l.clone())),
        None => x.st.last_lines.remove(&hwnd),
    };
    let wref = x.wref(hwnd);
    let delta = match (&lines, c.on("full") || c.on("web")) {
        (Some(l), false) => changes(old.as_ref(), l, &wref, now()),
        _ => None,
    };
    if let Some(delta) = delta {
        let shown: String = rest.split('\n').take(SNAP_LINES).collect::<Vec<_>>().join("\n");
        x.ctx.cut += shown.chars().count().min(OUT_CHARS).saturating_sub(delta.chars().count());
        return Ok(x.cap(&format!("{head}\n{delta}"), "app-snap", Some(SNAP_LINES), OUT_CHARS));
    }
    let hint = keys::hint(&x.d.home, &exe);
    let text = format!("{head}{hint}\n{rest}");
    let text = if c.on("full") || c.on("web") { text } else { brain::prune(x.d, x.ctx, &text, &format!("app:{exe}")) };
    Ok(x.cap(&text, "app-snap", Some(SNAP_LINES), OUT_CHARS))
}
