use std::fs;
use std::path::Path;

use lt_core::Fail;
use serde_json::{Map, Value, json};

use super::hosts::yes;

pub const SHOT_WIDTH: i64 = 1024;
pub const NAV_LINES: usize = 40;
const NAV_CHARS: usize = 1600;
const SNAP_TOP: i64 = 15;
const FOCUS_MODES: [&str; 3] = ["never", "away", "idle"];
const RISK_WORDS: &[&str] = &[
    "buy",
    "purchase",
    "pay",
    "checkout",
    "place order",
    "order now",
    "subscribe",
    "delete",
    "remove account",
    "close account",
    "transfer",
    "wire",
    "merge pull request",
    "confirm merge",
    "squash and merge",
    "rebase and merge",
    "kaufen",
    "bezahlen",
    "zahlungspflichtig",
    "jetzt bestellen",
    "abonnieren",
    "löschen",
    "konto schließen",
    "überweisen",
];

fn defaults() -> Map<String, Value> {
    let pairs = [
        ("browser", json!("auto")),
        ("pointer", json!(true)),
        ("ocr_lang", json!("auto")),
        ("shot_width", json!(SHOT_WIDTH)),
        ("learn", json!(true)),
        ("cleanup", json!(true)),
        ("audio", json!("auto")),
        ("window", json!(true)),
        ("focus", json!("never")),
        ("brain", json!(true)),
        ("snap_top", json!(SNAP_TOP)),
        ("memory", json!(true)),
        ("memory_learn", json!(true)),
        ("memory_api", json!("")),
        ("memory_api_model", json!("")),
        ("memory_api_embed", json!("")),
        ("memory_server", json!("")),
        ("memory_model", json!("")),
        ("memory_embed", json!("")),
        ("gemini", json!(true)),
        ("gemini_model", json!("gemini-2.5-flash")),
        ("models_dir", json!("")),
        ("vault", json!("")),
        ("vault_inbox", json!("Inbox")),
    ];
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

pub fn load(home: &Path) -> Map<String, Value> {
    let mut cfg = defaults();
    if let Ok(text) = fs::read_to_string(home.join("config.json"))
        && let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(&text)
    {
        cfg.extend(saved);
    }
    cfg
}

pub fn on(cfg: &Map<String, Value>, key: &str) -> bool {
    cfg.get(key).is_none_or(yes)
}

pub fn text(cfg: &Map<String, Value>, key: &str) -> String {
    match cfg.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub fn payload(home: &Path) -> Value {
    let cfg = load(home);
    json!({"blocklist": blocklist(home), "risk": RISK_WORDS, "pointer": on(&cfg, "pointer"),
           "window": on(&cfg, "window"), "navLines": NAV_LINES, "navChars": NAV_CHARS})
}

fn blocklist(home: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(home.join("blocklist.txt")) else { return Vec::new() };
    text.lines()
        .map(str::trim)
        .filter(|r| !r.is_empty() && !r.starts_with('#'))
        .map(|r| r.to_lowercase().trim_start_matches('.').to_string())
        .collect()
}

pub fn command(home: &Path, pos: &[String]) -> Result<(String, bool), Fail> {
    let mut cfg = load(home);
    let dumps = |v: &Value| serde_json::to_string(v).unwrap_or_default();
    let Some(key) = pos.first() else {
        let mut keys: Vec<&String> = cfg.keys().collect();
        keys.sort();
        let lines: Vec<String> = keys.iter().map(|k| format!("{k} = {}", dumps(&cfg[k.as_str()]))).collect();
        return Ok((lines.join("\n"), false));
    };
    let all = defaults();
    let Some(default) = all.get(key.as_str()) else {
        let mut keys: Vec<&str> = all.keys().map(String::as_str).collect();
        keys.sort_unstable();
        return Err(Fail::hint(format!("unknown config key '{key}'"), format!("one of: {}", keys.join(", "))));
    };
    if pos.len() == 1 {
        return Ok((format!("{key} = {}", dumps(cfg.get(key.as_str()).unwrap_or(&Value::Null))), false));
    }
    let raw = pos[1..].join(" ");
    let val = match default {
        Value::Bool(_) => json!(["1", "true", "on", "yes"].contains(&raw.to_lowercase().as_str())),
        Value::Number(_) => match raw.trim().parse::<i64>() {
            Ok(n) => json!(n),
            Err(_) => {
                return Err(Fail::hint(
                    format!("{key} takes a whole number"),
                    format!("lighting config {key} {default}"),
                ));
            }
        },
        _ => json!(raw),
    };
    if key == "focus" && !FOCUS_MODES.contains(&raw.as_str()) {
        return Err(Fail::hint(format!("focus is one of {}", FOCUS_MODES.join(", ")), "lighting config focus never"));
    }
    let line = format!("{key} = {}", dumps(&val));
    cfg.insert(key.clone(), val);
    let pretty = serde_json::to_string_pretty(&Value::Object(cfg)).unwrap_or_default();
    let path = home.join("config.json");
    fs::create_dir_all(home)
        .and_then(|()| fs::write(&path, pretty))
        .map_err(|e| Fail::new(format!("cannot save {}: {e}", path.display())))?;
    Ok((line, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("lt-config-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        home
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn config_reads_and_writes_like_2x() {
        let h = home("rw");
        let (all, changed) = command(&h, &[]).unwrap();
        assert!(!changed && all.starts_with("audio = \"auto\"\nbrain = true\nbrowser = \"auto\""), "{all}");
        assert_eq!(command(&h, &args(&["shot_width"])).unwrap(), ("shot_width = 1024".into(), false));
        assert_eq!(command(&h, &args(&["pointer", "off"])).unwrap(), ("pointer = false".into(), true));
        assert_eq!(command(&h, &args(&["shot_width", "800"])).unwrap(), ("shot_width = 800".into(), true));
        assert!(!on(&load(&h), "pointer") && on(&load(&h), "window"));
        assert_eq!(text(&load(&h), "browser"), "auto");
        let fail = |a: &[&str]| command(&h, &args(a)).unwrap_err().text();
        assert!(fail(&["nope"]).starts_with("err: unknown config key 'nope' -> try: one of: audio, brain"));
        assert_eq!(
            fail(&["focus", "sometimes"]),
            "err: focus is one of never, away, idle -> try: lighting config focus never"
        );
        assert!(fail(&["shot_width", "big"]).starts_with("err: shot_width takes a whole number"));
        fs::write(h.join("blocklist.txt"), "# no\n.Bank.example\n\nshop.test \n").unwrap();
        assert_eq!(payload(&h)["blocklist"], json!(["bank.example", "shop.test"]));
        assert_eq!(payload(&h)["pointer"], json!(false));
        fs::remove_dir_all(&h).unwrap();
    }
}
