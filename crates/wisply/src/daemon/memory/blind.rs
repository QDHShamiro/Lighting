use std::fs;
use std::path::Path;

use base64::Engine;
use serde_json::{Value, json};
use wisply_core::{Call, Fail};

use super::super::Daemon;

const TEXT_MAX: usize = 200_000;
const IMAGE_MAX: u64 = 8_000_000;
const IMAGES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
];

fn version(path: &str) -> Value {
    let p = Path::new(path);
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if let Some((_, mime)) = IMAGES.iter().find(|(e, _)| *e == ext) {
        if fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX) > IMAGE_MAX {
            return json!({"kind": "missing"});
        }
        return match fs::read(p) {
            Ok(raw) => json!({"kind": "image", "src": format!("data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(raw))}),
            Err(_) => json!({"kind": "missing"}),
        };
    }
    match fs::read(p) {
        Ok(raw) => {
            let text = String::from_utf8_lossy(&raw[..raw.len().min(TEXT_MAX)]).to_string();
            json!({"kind": "text", "text": text, "cut": raw.len() > TEXT_MAX})
        }
        Err(_) => json!({"kind": "missing"}),
    }
}

pub fn open(d: &Daemon) -> Value {
    let list = wisply_think::blind_open(&d.home);
    let items: Vec<Value> = list
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let side = |k: &str| version(e["paths"][k].as_str().unwrap_or_default());
            json!({"run": e["run"], "name": e["name"], "n": e["n"], "a": side("1"), "b": side("2")})
        })
        .collect();
    json!({ "open": items })
}

pub fn pick(d: &Daemon, c: &Call) -> Result<Value, Fail> {
    let (Some(run), Some(n), Some(verdict)) = (c.pos.get(1), c.pos.get(2).and_then(|n| n.parse().ok()), c.pos.get(3))
    else {
        return Err(Fail::hint("which comparison?", "wisply board blind-pick <run> <n> 1|2|same"));
    };
    let winner = wisply_think::blind_pick(&d.home, run, n, verdict).map_err(Fail::new)?;
    Ok(json!({ "winner": winner }))
}
