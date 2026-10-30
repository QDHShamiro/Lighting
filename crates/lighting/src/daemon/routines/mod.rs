mod align;
mod diff;
mod episode;
mod learn;
mod run;
mod schedule;
mod store;

use std::collections::{BTreeMap, HashSet};
use std::fs;

use lt_core::{Call, Fail, OUT_CHARS};
use serde_json::{Value, json};

use super::ctx::{Ctx, now};
use super::words::{terms, tiers};
use super::{Daemon, Out, web};
pub use episode::Episode;
use store::{file, load, save, score, slug, stem, step_line, text, usage};

pub use align::truthy;
pub use episode::{close, observe};
pub use run::run;
pub use store::{load_all, words};

const LIST_LINES: usize = 60;
const NAME_WORDS: usize = 6;
const SUBS: &str = "lighting routine list|show|save|rm|rename|forget|schedule|schedules";
const PROMPTS: usize = 5;
const OFFER_EVERY_S: f64 = 600.0;
const OFFER_IGNORED: u64 = 5;
const OFFERS: usize = 2;

pub fn recorded(d: &Daemon, ctx: &Ctx, name: &str, steps: &[Value], pairs: &[String]) -> Result<String, Fail> {
    let task = learn::Task { steps, cost: 0, start: &Value::Null };
    let r = learn::from_steps(&d.home, name, pairs, "recorded", &task, &d.prompts_of(&ctx.sid))?;
    let list = r["steps"].as_array().cloned().unwrap_or_default();
    let mut lines = vec![format!(
        "saved routine {} ({} steps) -> {}",
        r["name"].as_str().unwrap_or(name),
        list.len(),
        usage(&r, &BTreeMap::new())
    )];
    lines.extend(list.iter().enumerate().map(|(i, s)| format!("{} {}", i + 1, step_line(s, None))));
    Ok(lines.join("\n"))
}

pub fn episodes_text(home: &std::path::Path) -> String {
    fs::read(episode::file(home)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

pub fn offer(d: &Daemon, ctx: &mut Ctx, typed: Vec<String>) -> Vec<String> {
    let t = now();
    if !typed.is_empty() {
        let mut all = d.prompts.lock().unwrap_or_else(|e| e.into_inner());
        let mine = all.entry(ctx.sid.clone()).or_default();
        mine.push((t, typed.clone()));
        let extra = mine.len().saturating_sub(PROMPTS);
        mine.drain(..extra);
    }
    if typed.iter().map(|w| stem(w)).collect::<HashSet<_>>().len() < 2 {
        return Vec::new();
    }
    let mut hits: Vec<(bool, usize, u64, Value)> = Vec::new();
    for r in load_all(&d.home) {
        let key = format!("r:{}", text(&r["name"]));
        let ignored = r["stats"]["offered"].as_u64().unwrap_or(0) >= OFFER_IGNORED;
        if r["flaky"] == true || ignored || t - ctx.told.get(&key).copied().unwrap_or(0.0) < OFFER_EVERY_S {
            continue;
        }
        let (app_hit, other) = score(&typed, &r);
        if app_hit && other >= 1 {
            hits.push((app_hit, other, r["stats"]["ok"].as_u64().unwrap_or(0), r));
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse((h.0, h.1, h.2)));
    let mut lines = Vec::new();
    for (_, _, _, mut r) in hits.into_iter().take(OFFERS) {
        ctx.told.insert(format!("r:{}", text(&r["name"])), t);
        r["stats"]["offered"] = json!(r["stats"]["offered"].as_u64().unwrap_or(0) + 1);
        let _ = save(&d.home, &r);
        lines.push(format!(
            "Lighting routine fits this request (one call, swap the values): {}",
            usage(&r, &BTreeMap::new())
        ));
    }
    lines
}

fn listing(d: &Daemon, ctx: &mut Ctx, c: &Call) -> String {
    learn::prune(&d.home);
    let mut rs = load_all(&d.home);
    if let Some(f) = c.val("f").filter(|f| !f.is_empty()) {
        let ws = terms(f);
        rs.retain(|r| {
            let tags: Vec<String> = r["tags"].as_array().into_iter().flatten().map(text).collect();
            let args: Vec<String> = r["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| s["args"].as_array().into_iter().flatten().map(text).collect::<Vec<_>>().join(" "))
                .collect();
            let (name, tags, args) = (text(&r["name"]), tags.join(" "), args.join(" "));
            tiers(&ws, &[&name, &tags, &args]).iter().any(|t| *t > 0)
        });
    }
    if rs.is_empty() {
        return "no routines yet (Lighting learns one when a task repeats; lighting routine save <name> saves the last task)"
            .into();
    }
    let n = |r: &Value, k: &str| r["stats"][k].as_u64().unwrap_or(0);
    rs.sort_by(|a, b| n(b, "runs").cmp(&n(a, "runs")).then_with(|| text(&a["name"]).cmp(&text(&b["name"]))));
    let lines: Vec<String> = rs
        .iter()
        .map(|r| {
            let (runs, ok) = (n(r, "runs"), n(r, "ok"));
            let rate = match runs {
                0 => "new".to_string(),
                _ if ok == runs => format!("{ok}x ok"),
                _ => format!("{ok}/{runs} ok"),
            };
            let ms = n(r, "ms");
            let extra = if ms > 0 { format!(" | {:.1} s", ms as f64 / 1000.0) } else { String::new() };
            let args: String = r["params"].as_object().into_iter().flatten().map(|(p, _)| format!(" {p}=")).collect();
            let flaky = if r["flaky"].as_bool() == Some(true) { " | FLAKY" } else { "" };
            format!("{}{args} | {rate}{extra}{flaky}", text(&r["name"]))
        })
        .collect();
    let body = format!("{}\n(steps: lighting routine show <name>)", lines.join("\n"));
    web::capped(d, ctx, &body, "routines", Some(LIST_LINES), OUT_CHARS)
}

fn show(r: &Value) -> String {
    let st = &r["stats"];
    let n = |k: &str| st[k].as_u64().unwrap_or(0);
    let flaky = if r["flaky"].as_bool() == Some(true) { " FLAKY" } else { "" };
    let mut out = vec![format!(
        "{} v{} ({}, {}){flaky}",
        text(&r["name"]),
        r["version"].as_u64().unwrap_or(1),
        text(&r["source"]),
        text(&r["created"])
    )];
    let params: Vec<String> =
        r["params"].as_object().into_iter().flatten().map(|(k, v)| format!("{k} (last \"{}\")", text(v))).collect();
    if !params.is_empty() {
        out.push(format!("params: {}", params.join(", ")));
    }
    for (i, s) in r["steps"].as_array().into_iter().flatten().enumerate() {
        out.push(format!("{} {}", i + 1, step_line(s, None)));
    }
    let err = st["last_error"].as_str().map(|e| format!(", last error: {e}")).unwrap_or_default();
    out.push(format!(
        "runs {}, ok {}, fail {}, ~{} ms, saved ~{} tokens, manual cost ~{} tokens{err}",
        n("runs"),
        n("ok"),
        n("fail"),
        n("ms"),
        n("saved"),
        r["cost"].as_u64().unwrap_or(0)
    ));
    out.join("\n")
}

fn need<'a>(rest: &'a [String], what: &str) -> Result<&'a str, Fail> {
    rest.first().map(String::as_str).ok_or_else(|| Fail::new(format!("routine {what} needs a name")))
}

pub fn command(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    if c.name == "routines" {
        return Ok(Out::Text(listing(d, ctx, c)));
    }
    if c.name == "run" {
        return run(d, ctx, c);
    }
    let sub = c.pos.first().map_or("list".to_string(), |s| s.to_lowercase());
    let rest = c.pos.get(1..).unwrap_or(&[]);
    let text_out = match sub.as_str() {
        "list" | "ls" => listing(d, ctx, c),
        "show" => show(&load(&d.home, need(rest, "show")?)?),
        "rm" | "delete" => {
            let r = load(&d.home, need(rest, "rm")?)?;
            let name = text(&r["name"]);
            fs::remove_file(file(&d.home, &name)).map_err(|e| Fail::new(format!("cannot remove {name} ({e})")))?;
            if schedule::load(&d.home).contains_key(&name) {
                schedule::remove(&d.home, &name)?;
                format!("removed routine {name} and its schedule")
            } else {
                format!("removed routine {name}")
            }
        }
        "rename" => {
            let [old, new, ..] = rest else {
                return Err(Fail::hint("rename needs old and new name", "lighting routine rename old new"));
            };
            let mut r = load(&d.home, old)?;
            let wanted = slug(new, NAME_WORDS);
            let wanted = if wanted.is_empty() { new.clone() } else { wanted };
            if file(&d.home, &wanted).exists() {
                return Err(Fail::new(format!("a routine {wanted} exists already")));
            }
            let _ = fs::remove_file(file(&d.home, &text(&r["name"])));
            r["name"] = json!(wanted);
            save(&d.home, &r)?;
            format!("renamed to {wanted}")
        }
        "save" => learn::save_current(d, ctx, need(rest, "save")?, &rest[1..], c.val("last"))?,
        "forget" => {
            let _ = fs::remove_file(episode::file(&d.home));
            ctx.ep = Episode::default();
            "task history cleared (routines stay)".into()
        }
        "learn" => match close(d, ctx) {
            Some(r) => format!("learned {}", text(&r["name"])),
            None => "nothing new to learn".into(),
        },
        "schedules" => schedule::listing(&d.home),
        "unschedule" => schedule::remove(&d.home, need(rest, "unschedule")?)?,
        "schedule" => schedule::add(&d.home, need(rest, "schedule")?, &rest[1..])?,
        other => return Err(Fail::hint(format!("unknown routine command {other}"), SUBS)),
    };
    Ok(Out::Text(text_out))
}
