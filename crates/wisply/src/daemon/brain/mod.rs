mod improve;
mod watch;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use serde_json::{Map, Value, json};
use wisply_core::{Call, Fail};

use super::config;
use super::ctx::{Ctx, now};
use super::routines::episodes_text;
use super::{Daemon, Out, web};

pub use improve::{improve, pct, summary, weekly_tip};
pub use watch::{Miss, hint_for, observe, retry};

const TOP: usize = 15;
const PAGE_USES: u64 = 3;
const HOST_USES: u64 = 10;
const MIN_HITS: usize = 3;
const MIN_HIDDEN: usize = 5;
const SAVE_S: f64 = 30.0;
const MAX_PAGES: usize = 400;
const MAX_ELS: usize = 150;
const HALF_LIFE_S: f64 = 30.0 * 86_400.0;
const NORM_CHARS: usize = 60;
const LABEL_CHARS: usize = 80;
const PATH_SEGS: usize = 4;
const LIST_LINES: usize = 60;
const SHOWN: usize = 8;
const TOP_LABELS: usize = 5;
const FIX_SITES: usize = 6;
const FIX_EACH: usize = 2;
const LEARN_CMDS: &[&str] = &["click", "type", "select", "check", "hover", "fill"];
const ALWAYS: &[&str] = &["alert", "alertdialog", "dialog", "status"];
const NO_PRUNE: &[&str] = &["f", "all", "diff", "force", "media", "full", "frame", "links"];

pub static ELEMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^((?:f\d+\.)?[ed]\d+) ([\w-]+) "((?:[^"\\]|\\.)*)""#).expect("element"));
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").expect("digits"));
static SEGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:\d+|[0-9a-f]{7,}|[\w-]{25,})$").expect("segment"));

struct State {
    home: PathBuf,
    data: Value,
    dirty: bool,
    saved: f64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn file(home: &Path) -> PathBuf {
    home.join("brain.json")
}

fn empty() -> Value {
    json!({"v": 1, "pages": {}, "fixes": {}, "stats": {"pruned": 0, "chars": 0, "regained": 0, "fixed": 0}})
}

fn with<R>(home: &Path, f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.as_ref().is_none_or(|s| s.home != home) {
        let saved = fs::read_to_string(file(home)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let (data, dirty) = match saved.filter(|d| d["v"] == 1 && d.is_object()) {
            Some(d) => (d, false),
            None => (boot(home), true),
        };
        *guard = Some(State { home: home.to_path_buf(), data, dirty, saved: 0.0 });
    }
    f(guard.as_mut().expect("brain state"))
}

pub fn decay(age: f64) -> f64 {
    0.5f64.powf(age.max(0.0) / HALF_LIFE_S)
}

pub fn norm(name: &str) -> String {
    let joined = name.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    let s = DIGITS.replace_all(&joined, "#");
    s.trim_end_matches(['.', '…']).trim().chars().take(NORM_CHARS).collect()
}

pub fn page_key(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return String::new() };
    if !matches!(scheme.to_lowercase().as_str(), "http" | "https") {
        return String::new();
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (netloc, tail) = rest.split_at(end);
    let host = netloc.rsplit('@').next().unwrap_or("");
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    }
    .to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty() {
        return String::new();
    }
    let path = tail.split(['?', '#']).next().unwrap_or("");
    let mut parts = vec![host];
    for s in path.split('/').filter(|s| !s.is_empty()).take(PATH_SEGS) {
        parts.push(if SEGMENT.is_match(s) { "#".into() } else { s.to_lowercase() });
    }
    parts.join("/")
}

pub fn site(ctx: &Ctx) -> String {
    if ctx.kind() == Some("app") {
        let hwnd = ctx.target.as_ref().and_then(|t| t.1.parse::<isize>().ok());
        return hwnd
            .and_then(super::desk::app_where)
            .map(|(exe, _)| format!("app:{}", exe.to_lowercase()))
            .unwrap_or_default();
    }
    page_key(&ctx.url())
}

pub fn step_labels(cmd: &str, args: &[String]) -> Vec<String> {
    if cmd == "fill" {
        return args.iter().filter_map(|a| a.split_once('=').map(|(k, _)| k.to_string())).collect();
    }
    let a = args.first().map(String::as_str).unwrap_or("");
    let odd = ["#", "url:", "css:", "@"].iter().any(|p| a.starts_with(p));
    if a.is_empty() || web::ref_kind(a).is_some() || odd || matches!(a.to_lowercase().as_str(), "focused" | "screen") {
        return Vec::new();
    }
    vec![a.to_string()]
}

fn learn(data: &mut Value, key: &str, label: &str, role: &Value, t: f64) {
    let n = norm(label);
    if key.is_empty() || n.is_empty() || key == "app:" {
        return;
    }
    let host = key.split('/').next().unwrap_or("");
    let keys: Vec<&str> = if host == key { vec![key] } else { vec![key, host] };
    for k in keys {
        let p = data["pages"]
            .as_object_mut()
            .expect("pages")
            .entry(k)
            .or_insert_with(|| json!({"uses": 0, "t": t, "els": {}}));
        p["uses"] = json!(p["uses"].as_u64().unwrap_or(0) + 1);
        p["t"] = json!(p["t"].as_f64().unwrap_or(t).max(t));
        let els = p["els"].as_object_mut().expect("els");
        match els.get_mut(&n).and_then(Value::as_array_mut) {
            Some(e) => {
                let (w, at) = (e[0].as_f64().unwrap_or(0.0), e[1].as_f64().unwrap_or(t));
                e[0] = json!(if t >= at { w * decay(t - at) + 1.0 } else { w + decay(at - t) });
                e[1] = json!(at.max(t));
                if super::routines::truthy(role) {
                    e[2] = role.clone();
                }
            }
            None => {
                let shown: String = label.chars().take(LABEL_CHARS).collect();
                els.insert(n.clone(), json!([1.0, t, role, shown]));
            }
        }
    }
}

fn boot(home: &Path) -> Value {
    let mut d = empty();
    for row in episodes_text(home).lines() {
        let Ok(ep) = serde_json::from_str::<Value>(row) else { continue };
        for s in ep["steps"].as_array().into_iter().flatten() {
            let cmd = s["cmd"].as_str().unwrap_or("");
            let args: Vec<String> =
                s["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(String::from)).collect();
            if !LEARN_CMDS.contains(&cmd) || args.is_empty() {
                continue;
            }
            let w = &s["where"];
            let key = if w["kind"] == "app" {
                format!("app:{}", w["exe"].as_str().unwrap_or("").to_lowercase())
            } else {
                page_key(w["url"].as_str().unwrap_or(""))
            };
            let t =
                s["t"].as_f64().filter(|t| *t != 0.0).or(ep["t0"].as_f64().filter(|t| *t != 0.0)).unwrap_or_else(now);
            for label in step_labels(cmd, &args) {
                learn(&mut d, &key, &label, &s["role"], t);
            }
        }
    }
    d
}

fn score(e: &Value, at: f64) -> f64 {
    e[0].as_f64().unwrap_or(0.0) * decay(at - e[1].as_f64().unwrap_or(0.0))
}

fn ranked(data: &Value, key: &str) -> Vec<String> {
    let host = key.split('/').next().unwrap_or("");
    let uses = |p: &Value| p["uses"].as_u64().unwrap_or(0);
    let page = data["pages"].get(key).filter(|p| uses(p) >= PAGE_USES);
    let Some(p) = page.or_else(|| data["pages"].get(host).filter(|p| uses(p) >= HOST_USES)) else {
        return Vec::new();
    };
    let at = now();
    let mut els: Vec<(&String, f64)> =
        p["els"].as_object().into_iter().flatten().map(|(k, e)| (k, score(e, at))).collect();
    els.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    els.into_iter().map(|(k, _)| k.clone()).collect()
}

fn on(d: &Daemon) -> bool {
    config::on(&config::load(&d.home), "brain")
}

pub fn wants_prune(c: &Call) -> bool {
    !NO_PRUNE.iter().any(|k| c.on(k))
}

pub fn prune(d: &Daemon, ctx: &mut Ctx, text: &str, key: &str) -> String {
    let cfg = config::load(&d.home);
    if !config::on(&cfg, "brain") || key.is_empty() || text.is_empty() {
        return text.to_string();
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let told =
        lines.iter().take(3).any(|l| l.contains("since your last snap") || l.contains("unchanged since last snap"));
    if lines.len() < 3 || told {
        return text.to_string();
    }
    let top = cfg.get("snap_top").and_then(Value::as_u64).filter(|n| *n > 0).map_or(TOP, |n| n as usize);
    let names: Vec<String> = with(&d.home, |s| ranked(&s.data, key)).into_iter().take(top).collect();
    let Some(out) = only_used(&lines, &names) else { return text.to_string() };
    let cut = text.chars().count().saturating_sub(out.chars().count());
    count(&d.home, "pruned", 1);
    count(&d.home, "chars", cut as u64);
    ctx.cut += cut;
    out
}

fn only_used(lines: &[&str], names: &[String]) -> Option<String> {
    if names.len() < MIN_HITS {
        return None;
    }
    let loose: Vec<&String> = names.iter().filter(|n| n.chars().count() >= 4).collect();
    let (mut keep, mut hidden, mut hits) = (vec![lines[0].to_string()], 0, 0);
    for line in &lines[1..] {
        let Some(m) = ELEMENT.captures(line) else {
            keep.push(line.to_string());
            continue;
        };
        let n = norm(&m[3]);
        if ALWAYS.contains(&&m[2]) {
            keep.push(line.to_string());
        } else if !n.is_empty() && (names.contains(&n) || loose.iter().any(|x| n.contains(x.as_str()))) {
            keep.push(line.to_string());
            hits += 1;
        } else {
            hidden += 1;
        }
    }
    if hits < MIN_HITS || hidden < MIN_HIDDEN {
        return None;
    }
    keep.insert(
        1,
        format!("brain: {hits} controls you use here, {hidden} others hidden (wisply snap --full shows all)"),
    );
    Some(keep.join("\n"))
}

pub fn count(home: &Path, key: &str, n: u64) {
    with(home, |s| {
        let st = &mut s.data["stats"];
        st[key] = json!(st[key].as_u64().unwrap_or(0) + n);
        s.dirty = true;
    });
}

fn trim(data: &mut Value) {
    let at = now();
    let pages = data["pages"].as_object_mut().expect("pages");
    if pages.len() > MAX_PAGES {
        let mut by_age: Vec<(String, f64)> =
            pages.iter().map(|(k, p)| (k.clone(), p["t"].as_f64().unwrap_or(0.0))).collect();
        by_age.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (k, _) in by_age.into_iter().take(pages.len() - MAX_PAGES) {
            pages.shift_remove(&k);
        }
    }
    for p in pages.values_mut() {
        let Some(els) = p["els"].as_object().filter(|e| e.len() > MAX_ELS) else { continue };
        let mut kept: Vec<(String, Value)> = els.iter().map(|(k, e)| (k.clone(), e.clone())).collect();
        kept.sort_by(|a, b| score(&b.1, at).partial_cmp(&score(&a.1, at)).unwrap_or(std::cmp::Ordering::Equal));
        kept.truncate(MAX_ELS);
        p["els"] = Value::Object(kept.into_iter().collect());
    }
}

fn write(s: &mut State) {
    if !s.dirty {
        return;
    }
    trim(&mut s.data);
    let _ = fs::create_dir_all(&s.home);
    let tmp = file(&s.home).with_extension("tmp");
    if fs::write(&tmp, s.data.to_string()).and_then(|_| fs::rename(&tmp, file(&s.home))).is_ok() {
        (s.dirty, s.saved) = (false, now());
    }
}

pub fn stats(home: &Path) -> Value {
    with(home, |s| s.data["stats"].clone())
}

pub fn save(home: &Path) {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = guard.as_mut().filter(|s| s.home == home) {
        write(s);
    }
}

fn save_soon(s: &mut State) {
    if now() - s.saved > SAVE_S {
        write(s);
    }
}

fn listing(data: &Value) -> String {
    let pages = data["pages"].as_object();
    let st = &data["stats"];
    let n = |k: &str| st[k].as_u64().unwrap_or(0);
    let controls: usize = pages.into_iter().flatten().map(|(_, p)| p["els"].as_object().map_or(0, Map::len)).sum();
    let fixed: usize =
        data["fixes"].as_object().into_iter().flatten().map(|(_, f)| f.as_object().map_or(0, Map::len)).sum();
    format!(
        "brain: {} pages and apps, {controls} controls, {fixed} fixes learned | {} snapshots trimmed (-{} tokens) | {} refs regained, {} fixes used",
        pages.map_or(0, Map::len),
        n("pruned"),
        n("chars") / 4,
        n("regained"),
        n("fixed")
    )
}

pub fn command(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let sub = c.pos.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let home = &d.home;
    let text = match sub.as_str() {
        "reset" => {
            if !c.on("yes") {
                return Err(Fail::hint(
                    "this forgets everything Wisply learned about pages and apps",
                    "wisply brain reset --yes (only when the user asked for it)",
                ));
            }
            with(home, |s| {
                (s.data, s.dirty) = (empty(), true);
                write(s);
            });
            "brain reset: nothing learned any more".to_string()
        }
        "forget" => {
            let what = c.pos.get(1).map(|w| w.to_lowercase()).unwrap_or_default();
            let what = what.replace("https://", "").replace("http://", "").trim_matches('/').to_string();
            if what.is_empty() {
                return Err(Fail::hint(
                    "forget needs a page, site or app",
                    "wisply brain forget github.com | app:spotify.exe",
                ));
            }
            let gone = with(home, |s| {
                let keys: Vec<String> = s.data["pages"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(k, _)| k.clone())
                    .filter(|k| *k == what || k.starts_with(&format!("{what}/")))
                    .collect();
                for k in &keys {
                    s.data["pages"].as_object_mut().expect("pages").shift_remove(k);
                    s.data["fixes"].as_object_mut().expect("fixes").shift_remove(k);
                }
                s.dirty = true;
                write(s);
                keys.len()
            });
            format!("forgot {gone} page{}", if gone == 1 { "" } else { "s" })
        }
        _ => {
            let max = c.val("max").and_then(|m| m.parse::<usize>().ok()).unwrap_or(SHOWN);
            let shown = with(home, |s| overview(&s.data, max));
            let place = file(home).display().to_string().replace('\\', "/");
            let body = format!(
                "{}\nkept on this PC only ({place}) | brain forget <site> | brain reset --yes | config brain off",
                shown.join("\n")
            );
            web::capped(d, ctx, &body, "brain", Some(LIST_LINES), wisply_core::OUT_CHARS)
        }
    };
    Ok(Out::Text(text))
}

fn overview(data: &Value, max: usize) -> Vec<String> {
    let mut lines = vec![listing(data)];
    let at = now();
    let mut shown: Vec<(&String, &Value)> = data["pages"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| k.contains('/') || k.starts_with("app:"))
        .collect();
    shown.sort_by_key(|(_, p)| std::cmp::Reverse(p["uses"].as_u64().unwrap_or(0)));
    for (k, p) in shown.into_iter().take(max) {
        let mut els: Vec<&Value> = p["els"].as_object().into_iter().flatten().map(|(_, e)| e).collect();
        els.sort_by(|a, b| score(b, at).partial_cmp(&score(a, at)).unwrap_or(std::cmp::Ordering::Equal));
        let top: Vec<String> = els.iter().take(TOP_LABELS).map(|e| e[3].as_str().unwrap_or("").to_string()).collect();
        lines.push(format!("{k} ({} uses): {}", p["uses"].as_u64().unwrap_or(0), top.join(", ")));
    }
    for (key, fx) in data["fixes"].as_object().into_iter().flatten().take(FIX_SITES) {
        for (k, v) in fx.as_object().into_iter().flatten().take(FIX_EACH) {
            let (cmd, from) = k.split_once('|').unwrap_or((k, ""));
            let (to, n) = (v[0].as_str().unwrap_or(""), v[1].as_u64().unwrap_or(0));
            lines.push(format!("fix on {key}: {cmd} \"{from}\" -> \"{to}\" ({n}x)"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_keys_and_names_are_shaped_like_2x() {
        assert_eq!(
            page_key("https://www.GitHub.com/QDHShamiro/Wisply/pull/27/files?x=1"),
            "github.com/qdhshamiro/wisply/pull/#"
        );
        assert_eq!(page_key("http://user@host.example:8080/a/0123abcd9/"), "host.example/a/#");
        assert_eq!(page_key("file:///C:/x.html"), "");
        assert_eq!(norm("  Save   Draft 12 …"), "save draft #");
        assert!((decay(30.0 * 86_400.0) - 0.5).abs() < 1e-12);
        assert_eq!(step_labels("fill", &["Name=x".into(), "bad".into()]), vec!["Name"]);
        assert!(step_labels("click", &["e12".into()]).is_empty());
        let mut d = empty();
        for _ in 0..3 {
            learn(&mut d, "shop.example/cart", "Checkout 2", &json!("button"), 100.0);
        }
        assert_eq!(d["pages"]["shop.example"]["uses"], 3);
        assert_eq!(ranked(&d, "shop.example/cart"), vec!["checkout #"]);
        assert_eq!(page_key("https://[::1]:8/x"), "::1/x");
        let names: Vec<String> = ["save", "send", "inbox"].iter().map(|s| s.to_string()).collect();
        let mut snap = vec![
            "[t1] Mail",
            "e1 button \"Save\"",
            "e2 button \"Send now\"",
            "e3 link \"Inbox (3)\"",
            "e4 dialog \"Oops\"",
        ];
        let others =
            ["e5 link \"A\"", "e6 link \"B\"", "e7 link \"C\"", "e8 link \"D\"", "e9 link \"E\"", "plain text"];
        snap.extend(others);
        let out = only_used(&snap, &names).unwrap();
        let want = "[t1] Mail\nbrain: 3 controls you use here, 5 others hidden (wisply snap --full shows all)\ne1 button \"Save\"\ne2 button \"Send now\"\ne3 link \"Inbox (3)\"\ne4 dialog \"Oops\"\nplain text";
        assert_eq!(out, want);
        assert!(only_used(&snap[..6], &names).is_none());
    }

    #[test]
    #[ignore]
    fn boot_matches_2x_on_real_episodes() {
        let home = PathBuf::from(std::env::var("LT_PARITY_HOME").unwrap());
        let out = std::env::var("LT_PARITY_OUT").unwrap();
        fs::write(out, boot(&home).to_string()).unwrap();
    }
}
