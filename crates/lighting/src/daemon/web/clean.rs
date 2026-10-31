use std::collections::BTreeSet;

use lt_core::{Call, Fail};
use serde_json::{Map, Value, json};

use super::super::ctx::{Ctx, now};
use super::super::hosts::Called;
use super::super::{Daemon, STOPPED, config, desk};
use super::{num, obj, plural, ref_kind, secs};

const DONE_GRACE_S: f64 = 20.0;
const ASK_TIMEOUT_S: f64 = 5.0;

fn scope(ctx: &Ctx) -> Map<String, Value> {
    if ctx.sid.is_empty() { Map::new() } else { obj(json!({"only": ctx.owned})) }
}

fn browsers(d: &Daemon, ctx: &Ctx, cmd: &str, args: Map<String, Value>) -> Result<Vec<Map<String, Value>>, Fail> {
    let mut replies = Vec::new();
    for host in d.hosts.alive() {
        match host.call(cmd, Value::Object(args.clone()), None, secs(ASK_TIMEOUT_S), Some(ctx.group.as_str())) {
            Ok(Value::Object(reply)) => replies.push(reply),
            Ok(_) | Err(Called::Timeout) => {}
            Err(Called::Aborted) => return Err(Fail::uncounted(STOPPED)),
        }
    }
    Ok(replies)
}

pub fn done(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let quiet = c.on("quiet");
    let mut keep_open = scope(ctx);
    keep_open.insert("close".into(), json!(false));
    ctx.closing = 0.0;
    if !config::on(&config::load(&d.home), "cleanup") {
        browsers(d, ctx, "cleanup", keep_open)?;
        return Ok(if quiet {
            String::new()
        } else {
            "left everything open (cleanup is off: wisply config cleanup on)".into()
        });
    }
    if !ctx.sid.is_empty() && ctx.owned.is_empty() && !ctx.launched_apps() {
        return Ok(if quiet { String::new() } else { "nothing to close".into() });
    }
    if quiet && !ctx.sid.is_empty() {
        browsers(d, ctx, "cleanup", keep_open)?;
        ctx.closing = now() + DONE_GRACE_S;
        return Ok(String::new());
    }
    cleanup_now(d, ctx, quiet)
}

pub fn cleanup_now(d: &Daemon, ctx: &mut Ctx, quiet: bool) -> Result<String, Fail> {
    let web_tab = ctx.place.as_ref().is_some_and(|w| w.get("kind").and_then(Value::as_str) == Some("web"));
    if !ctx.sid.is_empty() && web_tab && ctx.tabs.first().is_some_and(|t| ctx.owned.contains(t)) {
        ctx.closed_url = Some(ctx.url());
    }
    let mut args = scope(ctx);
    args.insert("front".into(), json!(desk::browser_in_front()));
    let replies = browsers(d, ctx, "cleanup", args)?;
    let tabs: i64 = replies.iter().map(|r| num(r.get("closed"))).sum();
    let viewed: i64 = replies.iter().map(|r| num(r.get("kept"))).sum();
    let list = |r: &Map<String, Value>, key: &str| -> Vec<String> {
        r.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default()
    };
    let (apps, left) = desk::cleanup(ctx);
    if !ctx.sid.is_empty() {
        let gone: BTreeSet<String> = replies.iter().flat_map(|r| list(r, "shut")).collect();
        ctx.tabs.retain(|t| !gone.contains(t));
        ctx.owned.clear();
    }
    if quiet {
        return Ok(String::new());
    }
    let mut parts = Vec::new();
    if tabs > 0 {
        parts.push(plural(tabs, "tab"));
    }
    if !apps.is_empty() {
        parts.push(format!("{} ({})", plural(apps.len() as i64, "app"), apps.join(", ")));
    }
    let mut lines =
        vec![if parts.is_empty() { "nothing to close".into() } else { format!("closed {}", parts.join(", ")) }];
    if viewed > 0 {
        let why: BTreeSet<String> = replies.iter().flat_map(|r| list(r, "why")).collect();
        let place = if why.contains("yours") {
            " (in the user's own window)"
        } else if why.is_empty() {
            ""
        } else {
            " (the user brought it to the front)"
        };
        lines.push(format!("left {} open that the user is looking at{place}", plural(viewed, "tab")));
    }
    if !left.is_empty() {
        lines.push(format!("still open, it may ask to save: {}", left.join(", ")));
    }
    Ok(lines.join("\n"))
}

pub fn keep(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let spec = c.pos.first();
    let kind = spec.map(String::as_str).and_then(ref_kind);
    let mut tabs = 0;
    if spec.is_none() || kind == Some("t") {
        let mut args = obj(json!({"id": spec}));
        if spec.is_none() && !ctx.sid.is_empty() {
            args.insert("only".into(), json!(ctx.owned));
        }
        tabs = browsers(d, ctx, "keep", args)?.iter().map(|r| num(r.get("kept"))).sum();
    }
    let mut apps = 0;
    if spec.is_none_or(|s| kind == Some("w") || s.to_lowercase().starts_with("app:")) {
        apps = desk::keep(d, ctx, spec.map(String::as_str)).unwrap_or(0) as i64;
    }
    let parts: Vec<String> =
        [(tabs, "tab"), (apps, "app")].into_iter().filter(|(n, _)| *n > 0).map(|(n, w)| plural(n, w)).collect();
    if parts.is_empty() {
        return Ok("nothing to keep".into());
    }
    Ok(format!("kept {}: stays open after the task", parts.join(" and ")))
}
