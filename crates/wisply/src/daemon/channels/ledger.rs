use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

const FILE: &str = "channels.json";
const KEEP: usize = 400;
const REMEMBER_S: u64 = 7 * 86_400;
const WHY_CHARS: usize = 120;

static LOCK: Mutex<()> = Mutex::new(());

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn load(home: &Path) -> Map<String, Value> {
    fs::read_to_string(home.join(FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn save(home: &Path, all: &Map<String, Value>) {
    let tmp = home.join(format!("{FILE}.tmp"));
    if fs::write(&tmp, Value::Object(all.clone()).to_string()).is_ok() {
        let _ = fs::rename(&tmp, home.join(FILE));
    }
}

pub fn note(home: &Path, key: &str, way: &str, ok: bool, why: &str) {
    let _held = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut all = load(home);
    let at = now();
    let entry = all.entry(key.to_string()).or_insert_with(|| json!({"ways": {}}));
    let Some(obj) = entry.as_object_mut() else { return };
    let why: String = why.chars().take(WHY_CHARS).collect();
    obj.insert("last".into(), json!({"way": way, "ok": ok, "at": at, "why": why}));
    let ways = obj.entry("ways").or_insert_with(|| json!({}));
    if let Some(ways) = ways.as_object_mut() {
        let w = ways.entry(way.to_string()).or_insert_with(|| json!({"ok": 0, "fail": 0}));
        let count = if ok { "ok" } else { "fail" };
        w[count] = json!(w[count].as_u64().unwrap_or(0) + 1);
        w[if ok { "ok_at" } else { "fail_at" }] = json!(at);
        if !ok {
            w["why"] = json!(why);
        }
    }
    if all.len() > KEEP {
        let mut ages: Vec<(u64, String)> =
            all.iter().map(|(k, v)| (v["last"]["at"].as_u64().unwrap_or(0), k.clone())).collect();
        ages.sort();
        for (_, k) in ages.into_iter().take(all.len() - KEEP) {
            all.remove(&k);
        }
    }
    save(home, &all);
}

pub fn skip(home: &Path, key: &str, way: &str, instead: &str) -> bool {
    let all = load(home);
    let ways = &all.get(key).cloned().unwrap_or_default()["ways"];
    let failed = ways[way]["fail_at"].as_u64().unwrap_or(0);
    let worked = ways[way]["ok_at"].as_u64().unwrap_or(0);
    let other = ways[instead]["ok_at"].as_u64().unwrap_or(0);
    failed > worked && other >= failed && now().saturating_sub(failed) < REMEMBER_S
}

pub fn last(home: &Path, key: &str) -> Option<String> {
    let all = load(home);
    let last = all.get(key)?.get("last")?;
    let way = last["way"].as_str()?;
    let ago = ago(now().saturating_sub(last["at"].as_u64().unwrap_or(0)));
    let state = if last["ok"].as_bool() == Some(true) { "worked" } else { "failed" };
    let why = last["why"].as_str().filter(|w| !w.is_empty()).map(|w| format!(": {w}")).unwrap_or_default();
    Some(format!("{way} {state} {ago} ago{why}"))
}

pub fn hosts(home: &Path, prefix: &str) -> Vec<String> {
    let all = load(home);
    let mut found: Vec<(u64, String)> = all
        .iter()
        .filter_map(|(k, v)| {
            let host = k.strip_prefix(prefix)?;
            let way = v["last"]["way"].as_str()?;
            let ok = v["last"]["ok"].as_bool() == Some(true);
            ok.then(|| (v["last"]["at"].as_u64().unwrap_or(0), format!("{host} -> {way}")))
        })
        .collect();
    found.sort_by_key(|a| std::cmp::Reverse(a.0));
    found.into_iter().map(|(_, s)| s).collect()
}

fn ago(s: u64) -> String {
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        3600..86_400 => format!("{} h", s / 3600),
        _ => format!("{} d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_way_is_skipped_once_another_worked() {
        let home = std::env::temp_dir().join(format!("lt-ledger-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        assert!(!skip(&home, "web:a.com", "direct", "brave"));
        note(&home, "web:a.com", "direct", false, "403 Forbidden");
        assert!(!skip(&home, "web:a.com", "direct", "brave"));
        note(&home, "web:a.com", "brave", true, "");
        assert!(skip(&home, "web:a.com", "direct", "brave"));
        assert_eq!(last(&home, "web:a.com").as_deref(), Some("brave worked 0 s ago"));
        assert_eq!(hosts(&home, "web:"), vec!["a.com -> brave"]);
        note(&home, "web:a.com", "direct", true, "");
        assert!(!skip(&home, "web:a.com", "direct", "brave"));
        let _ = fs::remove_dir_all(&home);
    }
}
