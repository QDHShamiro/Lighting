use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use lt_core::{Call, Fail};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::super::ctx::{Ctx, now};
use super::super::hosts::lock;
use super::super::words::{terms, tiers};
use super::{pystr, re};

const HINT_EVERY_S: f64 = 7200.0;
const WHAT_CHARS: usize = 60;
const HARVEST_MAX: usize = 40;
const BROWSER: &[(&str, &str)] =
    &[("ctrl+l", "address bar"), ("ctrl+t", "new tab"), ("ctrl+w", "close tab"), ("ctrl+f", "find on page")];
const SEED: &[(&str, &[(&str, &str)])] = &[
    (
        "discord.exe",
        &[
            ("ctrl+k", "quick switcher (find a channel or DM)"),
            ("ctrl+/", "list of all shortcuts"),
            ("alt+up", "channel above"),
            ("alt+down", "channel below"),
        ],
    ),
    (
        "obsidian.exe",
        &[
            ("ctrl+o", "quick switcher (open a note)"),
            ("ctrl+p", "command palette"),
            ("ctrl+n", "new note"),
            ("ctrl+shift+f", "search all files"),
        ],
    ),
    ("explorer.exe", &[("ctrl+l", "address bar"), ("f2", "rename"), ("alt+up", "parent folder"), ("ctrl+f", "search")]),
    ("spotify.exe", &[("space", "play/pause"), ("ctrl+l", "search")]),
    ("brave.exe", BROWSER),
    ("chrome.exe", BROWSER),
    ("msedge.exe", BROWSER),
    (
        "*",
        &[
            ("win+e", "Explorer"),
            ("win+d", "show desktop"),
            ("win+v", "clipboard history"),
            ("win+shift+s", "screen snip"),
            ("playpause", "media play/pause (any app)"),
        ],
    ),
];
const ALIAS: &[(&str, &str)] = &[
    ("control", "ctrl"),
    ("strg", "ctrl"),
    ("umschalt", "shift"),
    ("umsch", "shift"),
    ("option", "alt"),
    ("cmd", "win"),
    ("command", "win"),
    ("meta", "win"),
    ("escape", "esc"),
    ("return", "enter"),
    ("eingabe", "enter"),
    ("entf", "delete"),
    ("del", "delete"),
    ("pfeil nach oben", "up"),
    ("arrowup", "up"),
    ("arrowdown", "down"),
    ("arrowleft", "left"),
    ("arrowright", "right"),
];

static SPLIT: LazyLock<Regex> = LazyLock::new(|| re(r"\s*[+-]\s*|\s+"));
static OPENED: LazyLock<Regex> = LazyLock::new(|| re(r#"^[de]\d+\s+(\w+)(?:\s+"([^"]*)")?"#));
static SITE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^[a-z]+://(?:www\.)?([^/:]+)"));
static SHOWN: LazyLock<Mutex<HashMap<String, f64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn rank(src: &str) -> u8 {
    match src {
        "learned" | "user" => 3,
        "ui" => 2,
        "seed" => 1,
        _ => 0,
    }
}

fn seed(app: &str) -> &'static [(&'static str, &'static str)] {
    SEED.iter().find(|(name, _)| *name == app).map_or(&[][..], |(_, keys)| *keys)
}

fn cut(text: &str) -> String {
    text.chars().take(WHAT_CHARS).collect()
}

pub fn norm(combo: &str) -> String {
    let low = combo.trim().to_lowercase().replace(" then ", " ").replace(", ", " ");
    let s = low.split_whitespace().collect::<Vec<_>>().join(" ");
    if !s.contains('+') && !s.contains('-') && s.contains(' ') && s.split(' ').all(|p| p.chars().count() == 1) {
        return s;
    }
    let parts: Vec<&str> = SPLIT.split(&s).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return s;
    }
    parts
        .iter()
        .map(|p| ALIAS.iter().find(|(from, _)| from == p).map_or(*p, |(_, to)| *to))
        .collect::<Vec<_>>()
        .join("+")
}

fn load(home: &Path) -> Map<String, Value> {
    match fs::read_to_string(home.join("keys.json")).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn save(home: &Path, data: Map<String, Value>) {
    if let Ok(text) = serde_json::to_string_pretty(&Value::Object(data)) {
        let _ = fs::create_dir_all(home).and_then(|()| fs::write(home.join("keys.json"), text));
    }
}

fn entry<'a>(data: &'a mut Map<String, Value>, app: &str) -> &'a mut Map<String, Value> {
    let slot = data.entry(app.to_lowercase()).or_insert_with(|| json!({}));
    if !slot.is_object() {
        *slot = json!({});
    }
    match slot {
        Value::Object(keys) => keys,
        _ => unreachable!("slot was just made an object"),
    }
}

fn src_of(v: Option<&Value>) -> &str {
    v.and_then(|v| v.get("src")).and_then(Value::as_str).unwrap_or("")
}

fn put(home: &Path, app: &str, combo: &str, what: &str, src: &str) {
    if app.is_empty() || combo.is_empty() || what.is_empty() {
        return;
    }
    let mut data = load(home);
    let keys = entry(&mut data, app);
    let k = norm(combo);
    let what = cut(what);
    let cur = keys.get(&k);
    if cur.is_some() && rank(src_of(cur)) > rank(src) {
        return;
    }
    let same = cur.and_then(|c| c.get("what")).and_then(Value::as_str) == Some(what.as_str());
    if same && src_of(cur) == src && src != "learned" {
        return;
    }
    let n = cur.and_then(|c| c.get("n")).and_then(Value::as_i64).unwrap_or(0) + i64::from(src == "learned");
    keys.insert(k, json!({"what": what, "src": src, "n": n}));
    save(home, data);
}

pub fn learn(home: &Path, app: &str, combo: &str, line: &str) {
    if let Some(m) = OPENED.captures(line) {
        let what = match m.get(2).filter(|name| !name.as_str().is_empty()) {
            Some(name) => format!("{} \"{}\" opens", &m[1], name.as_str()),
            None => format!("{} opens", &m[1]),
        };
        put(home, app, combo, &what, "learned");
    }
}

pub fn harvest(home: &Path, app: &str, pairs: &[Value]) {
    if app.is_empty() || pairs.is_empty() {
        return;
    }
    let mut data = load(home);
    let keys = entry(&mut data, app);
    let mut changed = false;
    for pair in pairs.iter().take(HARVEST_MAX) {
        let Some([combo, what]) = pair.as_array().map(Vec::as_slice).and_then(|p| <&[Value; 2]>::try_from(p).ok())
        else {
            continue;
        };
        let k = norm(&pystr(combo));
        let what = cut(&pystr(what).split_whitespace().collect::<Vec<_>>().join(" "));
        let cur = keys.get(&k);
        let known = cur.and_then(|c| c.get("what")).and_then(Value::as_str) == Some(what.as_str());
        if k.is_empty() || what.is_empty() || rank(src_of(cur)) > rank("ui") || known {
            continue;
        }
        keys.insert(k, json!({"what": what, "src": "ui", "n": 0}));
        changed = true;
    }
    if changed {
        save(home, data);
    }
}

fn known(home: &Path, app: &str) -> Vec<(String, Value)> {
    let low = app.to_lowercase();
    let app = if ["*", "windows", "windows.exe", "desktop"].contains(&low.as_str()) { "*" } else { low.as_str() };
    let mut out: Vec<(String, Value)> =
        seed(app).iter().map(|(k, what)| ((*k).to_string(), json!({"what": what, "src": "seed", "n": 0}))).collect();
    if let Some(Value::Object(saved)) = load(home).get(app) {
        for (k, v) in saved {
            match out.iter_mut().find(|(have, _)| have == k) {
                Some(slot) if rank(src_of(Some(v))) >= rank(src_of(Some(&slot.1))) => slot.1 = v.clone(),
                Some(_) => {}
                None => out.push((k.clone(), v.clone())),
            }
        }
    }
    out
}

fn ranked(home: &Path, app: &str) -> Vec<(String, Value)> {
    let mut rows = known(home, app);
    rows.sort_by_key(|(k, v)| {
        let n = v.get("n").and_then(Value::as_i64).unwrap_or(0);
        (std::cmp::Reverse(rank(src_of(Some(v)))), std::cmp::Reverse(n), k.clone())
    });
    rows
}

pub fn hint(home: &Path, app: &str) -> String {
    let app = app.to_lowercase();
    if app.is_empty() {
        return String::new();
    }
    let t = now();
    let mut shown = lock(&SHOWN);
    if shown.get(&app).is_some_and(|at| t - at < HINT_EVERY_S) {
        return String::new();
    }
    let own = seed(&app);
    let top: Vec<String> = ranked(home, &app)
        .into_iter()
        .filter(|(k, v)| src_of(Some(v)) != "seed" || own.iter().any(|(combo, _)| combo == k))
        .take(3)
        .map(|(k, v)| format!("{k} {}", v.get("what").and_then(Value::as_str).unwrap_or("")))
        .collect();
    if top.is_empty() {
        return String::new();
    }
    shown.insert(app, t);
    format!("\nkeys: {} (lighting keys)", top.join(" | "))
}

pub fn app_of(ctx: &Ctx) -> String {
    SITE.captures(&ctx.url()).map(|m| m[1].to_lowercase()).unwrap_or_default()
}

fn app_name(raw: &str) -> String {
    let app = raw.to_lowercase();
    if app.contains('.') { app } else { format!("{app}.exe") }
}

pub fn command(home: &Path, ctx: &Ctx, c: &Call) -> Result<String, Fail> {
    let verb = c.pos.first().map(|p| p.to_lowercase()).unwrap_or_default();
    if verb == "add" || verb == "rm" {
        if c.pos.len() < 3 {
            return Err(Fail::hint(
                format!("keys {} needs an app and a key", c.pos[0]),
                "lighting keys add discord.exe \"ctrl+k=quick switcher\"",
            ));
        }
        let app = app_name(&c.pos[1]);
        let rest = c.pos[2..].join(" ");
        if verb == "add" {
            let (combo, what) = rest.split_once('=').unwrap_or((rest.as_str(), ""));
            if what.is_empty() {
                return Err(Fail::hint(
                    "write it as key=what it does",
                    format!("lighting keys add {app} \"ctrl+k=quick switcher\""),
                ));
            }
            put(home, &app, combo, what.trim(), "user");
            return Ok(format!("saved {} {} for {app}", norm(combo), what.trim()));
        }
        let mut data = load(home);
        let k = norm(&rest);
        let removed = data.get_mut(&app).and_then(Value::as_object_mut).and_then(|keys| keys.remove(&k)).is_some();
        if !removed {
            return Err(Fail::hint(format!("no saved key {k} for {app}"), format!("lighting keys {app}")));
        }
        save(home, data);
        return Ok(format!("removed {k} for {app}"));
    }
    let app = match c.pos.first() {
        Some(raw) => app_name(raw),
        None => app_of(ctx),
    };
    if app.is_empty() {
        let mut apps: Vec<String> = load(home).keys().cloned().collect();
        apps.extend(SEED.iter().map(|(name, _)| (*name).to_string()).filter(|n| n != "*"));
        apps.sort();
        apps.dedup();
        return Ok(format!("known apps and sites: {}, windows -> lighting keys <app>", apps.join(", ")));
    }
    let app = if ["windows", "windows.exe", "desktop"].contains(&app.as_str()) { "*".to_string() } else { app };
    let mut rows = ranked(home, &app);
    if let Some(f) = c.val("f").filter(|f| !f.is_empty()) {
        let words = terms(f);
        rows.retain(|(k, v)| {
            tiers(&words, &[k, v.get("what").and_then(Value::as_str).unwrap_or("")]).iter().any(|t| *t > 0)
        });
    }
    if rows.is_empty() {
        return Ok(format!(
            "no keys known for {app} -> use it once (lighting press ...), or lighting keys add {app} \"ctrl+k=...\""
        ));
    }
    let title = if app == "*" { "windows (any app)" } else { app.as_str() };
    let mut lines = vec![format!("keys for {title} ({})", rows.len())];
    for (k, v) in &rows {
        let note = match src_of(Some(v)) {
            "learned" => format!(" (learned {}x)", v.get("n").and_then(Value::as_i64).unwrap_or(0)),
            "ui" => " (from the app)".into(),
            "user" => " (saved)".into(),
            _ => String::new(),
        };
        lines.push(format!("{k:<14} {}{note}", v.get("what").and_then(Value::as_str).unwrap_or("")));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_normalized_learned_and_listed() {
        assert_eq!(norm("Ctrl + Shift + F"), "ctrl+shift+f");
        assert_eq!(norm("Strg-K"), "ctrl+k");
        assert_eq!(norm("g then i"), "g i");
        assert_eq!(norm("Escape"), "esc");
        let home = std::env::temp_dir().join(format!("lt-keys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        harvest(&home, "github.com", &[json!(["Ctrl+K", "  command   palette "]), json!(["bad"])]);
        learn(&home, "github.com", "g i", "e12 link \"Issues\" new");
        let mut ctxs = super::super::super::ctx::Contexts::default();
        let ctx = ctxs.get("k");
        let list = command(&home, ctx, &lt_core::parse(&["keys".into(), "github.com".into()])).unwrap();
        let want = [
            "keys for github.com (2)".to_string(),
            format!("{:<14} link \"Issues\" opens (learned 1x)", "g i"),
            format!("{:<14} command palette (from the app)", "ctrl+k"),
        ];
        assert_eq!(list, want.join("\n"));
        assert_eq!(
            hint(&home, "github.com"),
            "\nkeys: g i link \"Issues\" opens | ctrl+k command palette (lighting keys)"
        );
        assert_eq!(hint(&home, "github.com"), "");
        let saved = command(
            &home,
            ctx,
            &lt_core::parse(&["keys".into(), "add".into(), "discord".into(), "ctrl+q=quit".into()]),
        );
        assert_eq!(saved.unwrap(), "saved ctrl+q quit for discord.exe");
        let rm =
            |k: &str| command(&home, ctx, &lt_core::parse(&["keys".into(), "rm".into(), "discord".into(), k.into()]));
        assert_eq!(rm("ctrl+q").unwrap(), "removed ctrl+q for discord.exe");
        assert!(rm("ctrl+q").unwrap_err().text().starts_with("err: no saved key ctrl+q for discord.exe"));
        fs::remove_dir_all(&home).unwrap();
    }
}
