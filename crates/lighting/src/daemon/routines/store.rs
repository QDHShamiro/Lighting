use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use lt_core::{Fail, local_now};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::super::ctx::Ctx;

static SAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w.@:/+-]+$").expect("safe"));
static PARAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{([a-z][a-z0-9_]*)\}").expect("param"));
static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9_-]+").expect("file"));
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z]+://(www\.)?").expect("scheme"));
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[@#]|[^\W\d_][\w-]*").expect("word"));
static APP_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z]+://(?:www\.)?([^/.]+)").expect("app"));
const STOP: &str = "a an and are auf aus bei bin bis bitte can da dann das dass dem den der des die dir do doch du ein \
    eine einen einem einer er es for from für geht gib hab habe hast hat he hey hi i ich ihr im in is ist it ja kann \
    kannst mach mache machen mal me mein meine mich mir mit my nach nicht noch nur oder of on or please sag so soll the \
    then to um und uns unter vom von vor was we wie will wir with wo you zu zum zur über lighting claude okay ok jetzt \
    schnell einfach auch alle alles aber also hier dort mehr sehr gut kurz neu neue schau schaue guck gucke check checke \
    look see warum wieso weshalb welche welcher welches welchen wann wer wen wem woher wohin womit wofür this that these \
    those what when where why how have has had get got want need just also more some any all";
const APP_PREFIX: usize = 4;
const RECENT: usize = 10;

pub fn dir(home: &Path) -> PathBuf {
    home.join("routines")
}

pub fn file(home: &Path, name: &str) -> PathBuf {
    let base = FILE.replace_all(&name.to_lowercase(), "-").trim_matches('-').to_string();
    dir(home).join(format!("{base}.json"))
}

pub fn slug(text: &str, n: usize) -> String {
    let t = text.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss");
    t.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()))
        .take(n)
        .collect::<Vec<_>>()
        .join("-")
}

pub fn load_all(home: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(dir(home)) else { return Vec::new() };
    let mut names: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    names.sort();
    names
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(|v| v["name"].is_string() && v["steps"].is_array())
        .collect()
}

pub fn save(home: &Path, r: &Value) -> Result<(), Fail> {
    fs::create_dir_all(dir(home)).map_err(|e| Fail::new(format!("cannot write routines ({e})")))?;
    let name = r["name"].as_str().unwrap_or("");
    let text = serde_json::to_string_pretty(r).unwrap_or_default();
    fs::write(file(home, name), indent_one(&text)).map_err(|e| Fail::new(format!("cannot save routine {name} ({e})")))
}

fn indent_one(pretty: &str) -> String {
    pretty
        .lines()
        .map(|l| {
            let spaces = l.len() - l.trim_start_matches(' ').len();
            format!("{}{}", " ".repeat(spaces / 2), l.trim_start_matches(' '))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn load(home: &Path, name: &str) -> Result<Value, Fail> {
    let path = file(home, name);
    if let Some(r) = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        return Ok(r);
    }
    let low = name.to_lowercase();
    let hits: Vec<Value> =
        load_all(home).into_iter().filter(|r| r["name"].as_str().is_some_and(|n| n.starts_with(&low))).collect();
    match hits.len() {
        1 => Ok(hits.into_iter().next().unwrap_or_default()),
        0 => Err(Fail::hint(format!("no routine \"{name}\""), "lighting routines")),
        _ => Err(Fail::hint(format!("\"{name}\" matches {}", names(&hits)), "use the full name")),
    }
}

fn names(rs: &[Value]) -> String {
    rs.iter().filter_map(|r| r["name"].as_str()).collect::<Vec<_>>().join(", ")
}

pub fn words(text: &str) -> Vec<String> {
    let stop: Vec<&str> = STOP.split_whitespace().collect();
    WORD.find_iter(&text.to_lowercase())
        .map(|m| m.as_str().to_string())
        .filter(|t| t == "@" || t == "#" || (t.chars().count() > 2 && !stop.contains(&t.as_str())))
        .collect()
}

fn stem(w: &str) -> String {
    w.chars().take(5).collect()
}

fn app_word(r: &Value) -> String {
    let first = &r["steps"][0];
    let w = &first["where"];
    let from_where = if w["kind"] == "app" {
        w["exe"].as_str().unwrap_or("").to_lowercase().trim_end_matches(".exe").to_string()
    } else {
        APP_URL.captures(w["url"].as_str().unwrap_or("")).map(|m| m[1].to_lowercase()).unwrap_or_default()
    };
    if !from_where.is_empty() {
        return from_where.split_whitespace().next().unwrap_or("").to_string();
    }
    let arg = first["args"][0].as_str().unwrap_or("").to_lowercase();
    let arg = match arg.strip_prefix("app:") {
        Some(rest) => rest.split_whitespace().next().unwrap_or("").to_string(),
        None => arg,
    };
    let bare = SCHEME.replace(&arg, "").to_string();
    let host = bare.split('/').next().unwrap_or("").split('.').next().unwrap_or("").split(':').next().unwrap_or("");
    host.split_whitespace().next().unwrap_or("").to_string()
}

fn score(query: &[String], r: &Value) -> (bool, usize) {
    let app = app_word(r);
    let mut own: Vec<String> = words(&r["name"].as_str().unwrap_or("").replace('-', " "));
    own.extend(r["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)));
    let own: Vec<String> = own.iter().filter(|w| !w.is_empty()).map(|w| stem(w)).collect();
    let mut shared: Vec<String> = query.iter().map(|w| stem(w)).filter(|w| own.contains(w)).collect();
    shared.sort();
    shared.dedup();
    let app_hit = !app.is_empty()
        && query.iter().any(|w| *w == app || (app.chars().count() >= APP_PREFIX && w.starts_with(&app)));
    let other = shared.iter().filter(|w| **w != stem(&app)).count();
    (app_hit, other)
}

pub fn find(home: &Path, query: &str) -> Result<Value, Fail> {
    let ws = words(query);
    let mut ranked: Vec<(usize, Value)> = load_all(home)
        .into_iter()
        .filter_map(|r| {
            let (app_hit, other) = score(&ws, &r);
            (app_hit || other > 0).then(|| (other + usize::from(app_hit), r))
        })
        .collect();
    ranked.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    match ranked.as_slice() {
        [] => Err(Fail::hint(format!("no routine fits \"{query}\""), "lighting routines")),
        [a, b, ..] if a.0 == b.0 => {
            let top: Vec<Value> = ranked.iter().take(3).map(|(_, r)| r.clone()).collect();
            Err(Fail::hint(format!("\"{query}\" fits several routines: {}", names(&top)), "use the full name"))
        }
        _ => Ok(ranked.swap_remove(0).1),
    }
}

pub fn kv(p: &str, v: &str) -> String {
    if SAFE.is_match(v) { format!("{p}={v}") } else { format!("{p}='{}'", v.replace('\'', "")) }
}

pub fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn usage(r: &Value, values: &BTreeMap<String, String>) -> String {
    let params: String = r["params"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(p, v)| format!(" {}", kv(p, values.get(p).map_or_else(|| text(v), Clone::clone).as_str())))
        .collect();
    format!("lighting run {}{params}", r["name"].as_str().unwrap_or(""))
}

pub fn fill(text: &str, params: &BTreeMap<String, String>) -> String {
    PARAM.replace_all(text, |m: &regex::Captures| params.get(&m[1]).cloned().unwrap_or_else(|| m[0].to_string())).into()
}

pub fn flag_argv(flags: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in flags {
        match v {
            Value::Bool(true) => out.push(format!("--{k}")),
            Value::Null | Value::Bool(false) => {}
            Value::String(s) if s.is_empty() => {}
            other => {
                out.push(format!("--{k}"));
                out.push(text(other));
            }
        }
    }
    out
}

pub fn step_line(s: &Value, params: Option<&BTreeMap<String, String>>) -> String {
    let args: Vec<String> = s["args"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let a = text(a);
            let a = params.map_or(a.clone(), |p| fill(&a, p));
            if a.contains(' ') || a.is_empty() { format!("\"{a}\"") } else { a }
        })
        .collect();
    let mut flags = s["flags"].as_object().cloned().unwrap_or_default();
    flags.shift_remove("on");
    let fl = flag_argv(&flags).join(" ");
    let line = format!(
        "{} {}{}",
        text(&s["cmd"]),
        args.join(" "),
        if fl.is_empty() { String::new() } else { format!(" {fl}") }
    );
    let optional = if s["optional"].as_bool() == Some(true) { " (optional)" } else { "" };
    format!("{}{optional}", line.trim())
}

pub fn argv_of(s: &Value, params: &BTreeMap<String, String>) -> Vec<String> {
    let mut argv = vec![text(&s["cmd"])];
    argv.extend(s["args"].as_array().into_iter().flatten().map(|a| fill(&text(a), params)));
    argv.extend(flag_argv(&s["flags"].as_object().cloned().unwrap_or_default()));
    argv
}

pub fn where_of(ctx: &Ctx) -> Option<Map<String, Value>> {
    if ctx.kind() == Some("app") {
        let hwnd: isize = ctx.target.as_ref()?.1.parse().ok()?;
        let (exe, title) = super::super::desk::app_where(hwnd)?;
        let mut w = Map::new();
        w.insert("kind".into(), json!("app"));
        w.insert("exe".into(), json!(exe));
        w.insert("title".into(), json!(title));
        return Some(w);
    }
    ctx.place.clone()
}

pub fn place(w: Option<&Map<String, Value>>) -> String {
    let Some(w) = w else { return String::new() };
    let title: String = w.get("title").map(text).unwrap_or_default().chars().take(60).collect();
    if w.get("kind").and_then(Value::as_str) == Some("app") {
        return format!("{title} - {}", w.get("exe").map(text).unwrap_or_default());
    }
    let url = SCHEME.replace(&w.get("url").map(text).unwrap_or_default(), "").chars().take(70).collect::<String>();
    format!("{title} - {url}")
}

pub fn verify(r: &Value, w: Option<&Map<String, Value>>, params: &BTreeMap<String, String>) -> &'static str {
    let check = &r["check"];
    let Some(w) = w.filter(|_| check.is_object()) else { return "" };
    let hay =
        format!("{} {}", w.get("title").map(text).unwrap_or_default(), w.get("url").map(text).unwrap_or_default())
            .to_lowercase();
    let has: Vec<String> = check["has"].as_array().into_iter().flatten().map(text).collect();
    let ok = if !has.is_empty() {
        has.iter().all(|h| hay.contains(&fill(h, params).to_lowercase()))
    } else if check["kind"] == "app" {
        w.get("exe").map(text).unwrap_or_default().eq_ignore_ascii_case(&text(&check["exe"]))
    } else {
        let want = text(&check["url"]);
        !want.is_empty() && w.get("url").map(text).unwrap_or_default().starts_with(&want)
    };
    if ok { "verified" } else { "not verified" }
}

pub fn check_of(w: Option<&Map<String, Value>>, values: &BTreeMap<String, String>) -> Value {
    let Some(w) = w else { return Value::Null };
    let hay =
        format!("{} {}", w.get("title").map(text).unwrap_or_default(), w.get("url").map(text).unwrap_or_default())
            .to_lowercase();
    let has: Vec<String> =
        values.iter().filter(|(v, _)| hay.contains(&v.to_lowercase())).map(|(_, p)| format!("{{{p}}}")).collect();
    if w.get("kind").and_then(Value::as_str) == Some("app") {
        return json!({"kind": "app", "exe": w.get("exe").cloned().unwrap_or(Value::Null), "has": has});
    }
    let url = w.get("url").map(text).unwrap_or_default();
    let url = url.split(['?', '#']).next().unwrap_or("").to_string();
    json!({"kind": "web", "has": has, "url": if has.is_empty() { json!(url) } else { Value::Null }})
}

pub fn record(home: &Path, r: &mut Value, ok: bool, ms: f64, out_tokens: usize, err: Option<String>) {
    let cost = r["cost"].as_u64().unwrap_or(0);
    let st = &mut r["stats"];
    if !st.is_object() {
        *st = json!({"runs": 0, "ok": 0, "fail": 0, "ms": 0, "saved": 0, "seen": 1, "recent": []});
    }
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    st["runs"] = json!(n(&st["runs"]) + 1);
    st["offered"] = json!(0);
    let key = if ok { "ok" } else { "fail" };
    st[key] = json!(n(&st[key]) + 1);
    let old = n(&st["ms"]) as f64;
    st["ms"] = json!(if old == 0.0 { ms as u64 } else { (old * 0.7 + ms * 0.3) as u64 });
    if ok {
        st["saved"] = json!(n(&st["saved"]) + cost.saturating_sub(out_tokens as u64));
    }
    let mut recent: Vec<Value> = st["recent"].as_array().cloned().unwrap_or_default();
    recent.push(json!(u8::from(ok)));
    let cut = recent.len().saturating_sub(RECENT);
    recent.drain(..cut);
    st["last"] = json!(local_now().chars().take(16).collect::<String>());
    st["last_error"] = err.map_or(Value::Null, Value::String);
    let last5: Vec<u64> = recent.iter().rev().take(5).map(n).collect();
    st["recent"] = json!(recent);
    r["flaky"] = json!(last5.len() >= 3 && last5.iter().filter(|v| **v == 0).count() >= 3);
    let _ = save(home, r);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routines_keep_the_2x_format_and_helpers() {
        let mut p = BTreeMap::new();
        p.insert("page".to_string(), "Context-Engine".to_string());
        assert_eq!(fill("github.com/QDHShamiro/{page}/{other}", &p), "github.com/QDHShamiro/Context-Engine/{other}");
        assert_eq!(kv("q", "a b's"), "q='a bs'");
        assert_eq!(kv("page", "Context-Engine"), "page=Context-Engine");
        assert_eq!(slug("Öffne GitHub 2 Issues & PRs", 6), "oeffne-github-issues-prs");
        let step = json!({"cmd": "type", "args": ["Search", "{q}"], "flags": {"on": "web", "enter": true, "max": 3}});
        assert_eq!(step_line(&step, None), "type Search {q} --enter --max 3");
        assert_eq!(argv_of(&step, &p), vec!["type", "Search", "{q}", "--on", "web", "--enter", "--max", "3"]);
        let r = json!({"name": "github-issues", "params": {"page": "x"}, "steps": [{"cmd": "open", "args": ["github.com/QDHShamiro/{page}"]}],
            "check": {"kind": "web", "has": ["{page}"], "url": null}, "tags": ["issues"]});
        assert_eq!(usage(&r, &p), "lighting run github-issues page=Context-Engine");
        let mut w = Map::new();
        w.insert("url".into(), json!("https://github.com/QDHShamiro/Context-Engine/issues"));
        w.insert("title".into(), json!("Issues"));
        assert_eq!(verify(&r, Some(&w), &p), "verified");
        assert_eq!(score(&words("zeig die github issues"), &r), (true, 1));
        let home = std::env::temp_dir().join(format!("lt-routines-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let mut saved = r.clone();
        saved["cost"] = json!(900);
        save(&home, &saved).unwrap();
        let text = fs::read_to_string(file(&home, "github-issues")).unwrap();
        assert!(text.starts_with("{\n \"name\": \"github-issues\",\n \"params\": {\n  \"page\": \"x\"\n }"), "{text}");
        record(&home, &mut saved, true, 1200.0, 100, None);
        let back = load(&home, "github").unwrap();
        assert_eq!((back["stats"]["runs"].as_u64(), back["stats"]["saved"].as_u64()), (Some(1), Some(800)));
        assert_eq!(find(&home, "github issues bitte").unwrap()["name"], "github-issues");
        let _ = fs::remove_dir_all(&home);
    }
}
