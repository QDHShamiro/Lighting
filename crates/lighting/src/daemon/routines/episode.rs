use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use lt_core::{Flag, parse};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::super::Daemon;
use super::super::config;
use super::super::ctx::{Ctx, now};
use super::super::web::ref_kind;
use super::align::{ENTRY, args, cmd};
use super::learn::{learn, match_step};
use super::store::{load_all, text, usage, where_of};

const ACTIONS: &[&str] = &[
    "open", "launch", "focus", "click", "type", "fill", "press", "select", "check", "hover", "scroll", "back",
    "forward", "reload", "wait", "expect", "dismiss", "dialog",
];
const REF_CMDS: &[&str] = &["click", "type", "select", "check", "hover"];
const KEEP_FLAGS: &[&str] = &[
    "new", "yes", "enter", "append", "submit", "first", "right", "double", "force", "until", "delta", "gone",
    "timeout", "game", "stay", "web", "max", "mouse", "text", "on", "tab", "reload",
];
const SHARED: &[&str] = &["click", "type", "press", "scroll", "hover"];
const IDLE_S: f64 = 45.0;
const KEEP_EPISODES: usize = 300;
const TRIM_EVERY: usize = 50;
const TRIM_SLACK: usize = 100;
const LABEL_CHARS: usize = 80;
const HINT_LEFT: usize = 2;
const HINT_LITERAL: usize = 6;
pub const PAST: usize = 150;

static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z]+://").expect("scheme"));
static HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z]+://([^/]+)").expect("host"));
static SLOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\w+\}").expect("slot"));
static WRITES: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub struct Episode {
    pub steps: Vec<Value>,
    pub cost: usize,
    pub start: Value,
    pub t0: f64,
    pub last: f64,
    pub via: Option<String>,
    pub lossy: bool,
    pub hinted: HashSet<String>,
    pub failed: Option<usize>,
    pub params: Value,
}

impl Episode {
    fn running(&self) -> bool {
        self.via.is_some() && self.failed.is_none()
    }
}

pub fn file(home: &Path) -> PathBuf {
    home.join("episodes.jsonl")
}

pub fn site(w: &Value) -> String {
    if w["kind"] == "app" {
        return text(&w["exe"]).to_lowercase();
    }
    HOST.captures(w["url"].as_str().unwrap_or("")).map(|m| m[1].to_lowercase()).unwrap_or_default()
}

fn here(ctx: &Ctx) -> Value {
    where_of(ctx).map_or(Value::Null, Value::Object)
}

fn stable(ctx: &Ctx, name: &str, pos: &[String], flags: &BTreeMap<String, Flag>) -> Option<Value> {
    let mut a = pos.to_vec();
    let mut role = Value::Null;
    if REF_CMDS.contains(&name) && a.first().is_some_and(|x| matches!(ref_kind(x), Some("e" | "d"))) {
        let t = ctx.last_target.clone().unwrap_or(Value::Null);
        let mut label = text(&t["name"]).trim().to_string();
        if label.is_empty() && !text(&t["hint"]).is_empty() {
            label = format!("#{}", text(&t["hint"]));
        }
        if label.is_empty() {
            return None;
        }
        a[0] = label.chars().take(LABEL_CHARS).collect();
        role = t["role"].clone();
    }
    if name == "type" && ctx.secret.is_some() {
        a.truncate(1);
        a.push("@secret".into());
    }
    let mut kept = Map::new();
    for (k, v) in flags.iter().filter(|(k, _)| KEEP_FLAGS.contains(&k.as_str())) {
        kept.insert(k.clone(), if let Flag::Val(s) = v { json!(s) } else { json!(true) });
    }
    if let Some(kind) = ctx.kind().filter(|k| SHARED.contains(&name) && matches!(*k, "web" | "app")) {
        kept.insert("on".into(), json!(kind));
    }
    let mut step = json!({"cmd": name, "args": a, "flags": kept});
    if super::align::truthy(&role) {
        step["role"] = role;
    }
    Some(step)
}

pub fn observe(d: &Daemon, ctx: &mut Ctx, argv: &[String], out: &str, ok: bool) {
    if ctx.quiet_steps > 0 || !config::on(&config::load(&d.home), "learn") {
        return;
    }
    let t = now();
    if !ctx.ep.steps.is_empty() && t - ctx.ep.last > IDLE_S && !ctx.ep.running() {
        close(d, ctx);
    }
    ctx.ep.cost += out.chars().count() / 4 + 15;
    let c = parse(argv);
    if !ACTIONS.contains(&c.name.as_str()) {
        return;
    }
    if !ok {
        if ctx.ep.via.is_some() && ctx.ep.failed.is_none_or(|f| f == 0) {
            ctx.ep.failed = Some(ctx.ep.steps.len());
        }
        return;
    }
    let Some(mut step) = stable(ctx, &c.name, &c.pos, &c.flags) else {
        ctx.ep.lossy = true;
        return;
    };
    let w = here(ctx);
    let ep = &ctx.ep;
    let moved = ep.steps.iter().filter(|s| !ENTRY.contains(&cmd(s))).count() >= 2 && site(&w) != site(&ep.start);
    if ENTRY.contains(&c.name.as_str()) && !ep.running() && moved {
        close(d, ctx);
    }
    let ep = &mut ctx.ep;
    if ep.steps.is_empty() {
        ep.start = w.clone();
        ep.t0 = t;
    }
    step["t"] = json!((t * 100.0).round() / 100.0);
    step["where"] = w;
    ep.steps.push(step);
    ep.last = t;
    hint(d, ctx);
}

pub fn record_of(ep: &Episode) -> Value {
    json!({"t0": ep.t0, "t1": ep.last, "start": ep.start, "steps": ep.steps, "cost": ep.cost, "via": ep.via,
           "failed": ep.failed, "lossy": ep.lossy, "params": ep.params})
}

pub fn close(d: &Daemon, ctx: &mut Ctx) -> Option<Value> {
    let ep = std::mem::take(&mut ctx.ep);
    if ep.steps.len() < 2 || ep.lossy {
        return None;
    }
    let rec = record_of(&ep);
    let past = episodes(&d.home, PAST);
    persist(&d.home, &rec);
    match learn(d, ctx, &rec, &past) {
        Ok(r) => r,
        Err(e) => {
            d.hosts.event(format!("routine learning failed: {}", e.msg));
            None
        }
    }
}

pub fn episodes(home: &Path, limit: usize) -> Vec<Value> {
    let Ok(all) = fs::read_to_string(file(home)) else { return Vec::new() };
    let rows: Vec<&str> = all.lines().collect();
    rows[rows.len().saturating_sub(limit)..].iter().filter_map(|r| serde_json::from_str(r).ok()).collect()
}

fn persist(home: &Path, rec: &Value) {
    let _ = fs::create_dir_all(home);
    let path = file(home);
    let wrote = OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| writeln!(f, "{rec}"));
    if wrote.is_err() || !(WRITES.fetch_add(1, Ordering::SeqCst) + 1).is_multiple_of(TRIM_EVERY) {
        return;
    }
    if let Ok(all) = fs::read_to_string(&path) {
        let rows: Vec<&str> = all.lines().collect();
        if rows.len() > KEEP_EPISODES + TRIM_SLACK {
            let _ = fs::write(&path, rows[rows.len() - KEEP_EPISODES..].join("\n") + "\n");
        }
    }
}

pub fn literal(step: &Value) -> usize {
    args(step)
        .iter()
        .map(|a| {
            let a = SCHEME.replace(a, "").to_string();
            let a =
                if cmd(step) == "open" { a.split_once('/').map(|x| x.1.to_string()).unwrap_or_default() } else { a };
            SLOT.replace_all(&a, "").chars().count()
        })
        .sum()
}

fn hint(d: &Daemon, ctx: &mut Ctx) {
    if ctx.ep.via.is_some() || ctx.ep.steps.is_empty() {
        return;
    }
    let steps = &ctx.ep.steps;
    for r in load_all(&d.home) {
        let name = text(&r["name"]);
        let need: Vec<Value> = r["steps"].as_array().cloned().unwrap_or_default();
        if r["flaky"] == true || ctx.ep.hinted.contains(&name) || need.is_empty() {
            continue;
        }
        for start in (0..steps.len()).rev() {
            let Some(mut got) = match_step(&need[0], &steps[start]) else { continue };
            let (mut k, mut ok) = (1, true);
            for s in &steps[start + 1..] {
                if k >= need.len() {
                    ok = false;
                    break;
                }
                let mut m = match_step(&need[k], s);
                if m.is_none()
                    && need[k]["optional"] == true
                    && k + 1 < need.len()
                    && match_step(&need[k + 1], s).is_some()
                {
                    k += 1;
                    m = match_step(&need[k], s);
                }
                let Some(m) = m else {
                    ok = false;
                    break;
                };
                got.extend(m);
                k += 1;
            }
            let left = need[k..].iter().filter(|s| s["optional"] != true).count();
            let typed: usize = need[..k].iter().map(literal).sum();
            if ok && left >= HINT_LEFT && typed >= HINT_LITERAL {
                d.hosts.event(format!("routine {name} can finish this in one call: {}", usage(&r, &got)));
                ctx.ep.hinted.insert(name);
                return;
            }
            break;
        }
    }
}
