mod run;
mod store;

use std::fs;

use lt_core::{Call, Fail, OUT_CHARS};
use serde_json::{Value, json};

use super::ctx::Ctx;
use super::route::later;
use super::words::{terms, tiers};
use super::{Daemon, Out, web};
use store::{file, load, load_all, save, slug, step_line, text};

pub use run::run;

const LIST_LINES: usize = 60;
const NAME_WORDS: usize = 6;
const SUBS: &str = "lighting routine list|show|save|rm|rename|forget|schedule|schedules";

fn listing(d: &Daemon, ctx: &mut Ctx, c: &Call) -> String {
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
            format!("removed routine {name}")
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
        "save" | "forget" | "learn" | "schedule" | "unschedule" | "schedules" => {
            return Err(later(&format!("routine {sub}")));
        }
        other => return Err(Fail::hint(format!("unknown routine command {other}"), SUBS)),
    };
    Ok(Out::Text(text_out))
}
