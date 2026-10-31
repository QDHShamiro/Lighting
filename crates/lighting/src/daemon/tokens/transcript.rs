use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use lt_core::local_to_utc;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::num::date_of;

const KEEP_DAYS: i64 = 90;
const FORMAT: u32 = 2;
const CACHE: &str = "claude-usage.json";
const IMAGE_CHARS: i64 = 3500;
const DAY_S: i64 = 86_400;
const PARSE_WORKERS: usize = 4;

static EFFORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""effort":\s*"(\w+)""#).expect("effort"));

#[derive(Clone, Default)]
pub struct Req {
    pub t: String,
    pub ctx: i64,
    pub write: i64,
    pub out: i64,
    pub vis: i64,
    pub effort: String,
    pub got: [i64; 3],
}

impl Req {
    pub fn json(&self) -> Value {
        json!([self.t, self.ctx, self.write, self.out, self.vis, self.effort, self.got[0], self.got[1], self.got[2]])
    }

    pub fn from(v: &Value) -> Option<Req> {
        let a = v.as_array().filter(|a| a.len() == 9)?;
        let n = |i: usize| a[i].as_i64().or_else(|| a[i].as_f64().map(|f| f as i64)).unwrap_or(0);
        Some(Req {
            t: a[0].as_str()?.to_string(),
            ctx: n(1),
            write: n(2),
            out: n(3),
            vis: n(4),
            effort: a[5].as_str().unwrap_or("").to_string(),
            got: [n(6), n(7), n(8)],
        })
    }
}

pub fn claude_home() -> PathBuf {
    env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::home_dir().map(|h| h.join(".claude")))
        .unwrap_or_default()
}

fn chars(s: &str) -> i64 {
    i64::try_from(s.chars().count()).unwrap_or(0)
}

fn py_str_len(s: &str) -> i64 {
    2 + s
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if (c as u32) < 0x20 => 6,
            c if c.is_ascii() => 1,
            c if (c as u32) > 0xFFFF => 12,
            _ => 6,
        })
        .sum::<i64>()
}

pub fn py_len(v: &Value) -> i64 {
    let n = |len: usize| i64::try_from(len).unwrap_or(0);
    match v {
        Value::Null => 4,
        Value::Bool(b) => 4 + i64::from(!*b),
        Value::Number(x) => n(x.to_string().len()),
        Value::String(s) => py_str_len(s),
        Value::Array(a) => 2 + a.iter().map(py_len).sum::<i64>() + 2 * n(a.len().saturating_sub(1)),
        Value::Object(o) => {
            2 + o.iter().map(|(k, v)| py_str_len(k) + 2 + py_len(v)).sum::<i64>() + 2 * n(o.len().saturating_sub(1))
        }
    }
}

fn visible(content: &Value) -> i64 {
    content
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| match b["type"].as_str() {
            Some("text") => chars(b["text"].as_str().unwrap_or("")),
            Some("tool_use") => py_len(b.get("input").filter(|i| !i.is_null()).unwrap_or(&json!({}))),
            _ => 0,
        })
        .sum()
}

fn size(content: &Value) -> i64 {
    if let Some(s) = content.as_str() {
        return chars(s);
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b.is_object())
        .map(|b| {
            if b["type"] == "image" {
                IMAGE_CHARS
            } else {
                chars(b["text"].as_str().unwrap_or("")) + size(&b["content"])
            }
        })
        .sum()
}

pub fn parse(text: &str) -> Vec<Req> {
    let (mut reqs, mut index): (Vec<Req>, HashMap<String, usize>) = (Vec::new(), HashMap::new());
    let (mut tools, mut effort, mut got): (HashMap<String, usize>, String, [i64; 3]) =
        (HashMap::new(), String::new(), [0; 3]);
    for line in text.lines() {
        if line.contains(r#""type":"api-request""#) {
            if let Some(m) = EFFORT.captures(line) {
                effort = m[1].to_string();
            }
            continue;
        }
        let answer = line.contains(r#""type":"assistant""#) && line.contains(r#""usage""#);
        if !answer && !line.contains(r#""type":"user""#) && !line.contains(r#""type":"attachment""#) {
            continue;
        }
        let Ok(e) = serde_json::from_str::<Value>(line) else { continue };
        if e.get("isSidechain").is_some_and(super::super::routines::truthy) {
            continue;
        }
        let m = &e["message"];
        match e["type"].as_str() {
            Some("attachment") => got[2] += size(&e["rendered"]),
            Some("user") => {
                let content = &m["content"];
                let blocks = content.as_array().cloned().unwrap_or_else(|| vec![json!({"text": content})]);
                for b in &blocks {
                    if b["type"] == "tool_result" {
                        let slot = b["tool_use_id"].as_str().and_then(|id| tools.get(id)).copied().unwrap_or(2);
                        got[slot] += size(&b["content"]);
                    } else {
                        got[2] += size(&json!([b]));
                    }
                }
            }
            Some("assistant") if answer => {
                for b in m["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_use") {
                    let slot = match b["name"].as_str() {
                        Some("Bash") => 0,
                        Some("Read") => 1,
                        _ => 2,
                    };
                    tools.insert(b["id"].as_str().unwrap_or("").to_string(), slot);
                }
                let u = &m["usage"];
                let n = |k: &str| u[k].as_i64().unwrap_or(0);
                let key = m["id"].as_str().or_else(|| e["uuid"].as_str()).unwrap_or("").to_string();
                let at = match index.get(&key) {
                    Some(at) => *at,
                    None => {
                        let write = n("cache_creation_input_tokens");
                        let ctx = n("input_tokens") + write + n("cache_read_input_tokens");
                        if ctx == 0 {
                            continue;
                        }
                        let t: String = e["timestamp"].as_str().unwrap_or("").chars().take(19).collect();
                        reqs.push(Req { t, ctx, write, effort: effort.clone(), got, ..Req::default() });
                        got = [0; 3];
                        index.insert(key, reqs.len() - 1);
                        reqs.len() - 1
                    }
                };
                let r = &mut reqs[at];
                r.out = r.out.max(n("output_tokens"));
                r.vis += visible(&m["content"]);
            }
            _ => {}
        }
    }
    reqs
}

fn iso(secs: i64) -> String {
    let rest = secs.rem_euclid(DAY_S);
    format!("{}T{:02}:{:02}:{:02}", date_of(secs.div_euclid(DAY_S)), rest / 3600, rest / 60 % 60, rest % 60)
}

fn parse_all(paths: &[&Path], misses: &[usize]) -> Vec<(usize, Option<Vec<Req>>)> {
    let next = AtomicUsize::new(0);
    let done = Mutex::new(Vec::with_capacity(misses.len()));
    let workers = thread::available_parallelism().map_or(PARSE_WORKERS, |n| n.get()).min(misses.len().max(1));
    thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                while let Some(&at) = misses.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let reqs = fs::read(paths[at]).ok().map(|raw| parse(&String::from_utf8_lossy(&raw)));
                    done.lock().unwrap_or_else(PoisonError::into_inner).push((at, reqs));
                }
            });
        }
    });
    done.into_inner().unwrap_or_else(PoisonError::into_inner)
}

pub fn load(home: &Path, first_day: i64) -> Vec<Vec<Req>> {
    let cache_file = home.join(CACHE);
    let cache: Map<String, Value> = fs::read_to_string(&cache_file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let first = local_to_utc(first_day * DAY_S);
    let start = iso(first);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let keep_from = now - KEEP_DAYS * DAY_S;
    let mut paths: Vec<PathBuf> = fs::read_dir(claude_home().join("projects"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|d| d.path().is_dir())
        .flat_map(|d| fs::read_dir(d.path()).into_iter().flatten().flatten().map(|f| f.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl") && p.is_file())
        .collect();
    paths.sort();
    let (mut fresh, mut sessions, mut items) = (Map::new(), Vec::new(), Vec::new());
    for path in paths {
        let Ok(meta) = fs::metadata(&path) else { continue };
        let mtime =
            meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
        let (key, sig) = (path.display().to_string(), format!("{FORMAT}:{}:{mtime}", meta.len()));
        let hit = cache.get(&key);
        if mtime < first {
            if let Some(h) = hit.filter(|_| mtime >= keep_from) {
                fresh.insert(key, h.clone());
            }
            continue;
        }
        let cached = hit
            .filter(|h| h["s"] == sig.as_str())
            .map(|h| h["r"].as_array().into_iter().flatten().filter_map(Req::from).collect::<Vec<Req>>());
        items.push((key, sig, path, cached));
    }
    let misses: Vec<usize> = items.iter().enumerate().filter(|(_, it)| it.3.is_none()).map(|(n, _)| n).collect();
    for (at, reqs) in parse_all(&items.iter().map(|it| it.2.as_path()).collect::<Vec<_>>(), &misses) {
        items[at].3 = reqs;
    }
    for (key, sig, _, reqs) in items {
        let Some(reqs) = reqs else { continue };
        fresh.insert(key, json!({"s": sig, "r": reqs.iter().map(Req::json).collect::<Vec<_>>()}));
        let kept: Vec<Req> = reqs.into_iter().filter(|r| r.t >= start).collect();
        if !kept.is_empty() {
            sessions.push(kept);
        }
    }
    let _ = fs::write(&cache_file, Value::Object(fresh).to_string());
    sessions
}
