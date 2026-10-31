mod clean;
mod cmds;
mod data;
mod entity;
mod html;
pub mod keys;
mod launch;
mod read;
mod shot;
mod sites;
mod tabkeys;
mod url;
mod wait;

use std::sync::LazyLock;
use std::time::Duration;

use lt_core::{Call, Fail, Flag, OUT_CHARS, cap};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::ctx::Ctx;
use super::hosts::{Called, truthy, yes};
use super::{Daemon, Out, STOPPED};

pub use clean::{cleanup_now, done, keep};
pub use data::pystr;
#[cfg(windows)]
pub use launch::browser_exe;
pub use launch::{any_browser, ensure_host, installed_browsers, pick};
pub use read::{read, wants as wants_read};
#[cfg(windows)]
pub use shot::out_path;
pub use shot::save_picture;
pub use shot::{distinct, save_sheet, sheet, thumb};
pub use url::{leak_check, local_file, normalize_url};
pub use wait::parse_ms;

pub const CALL_TIMEOUT: f64 = 20.0;
pub const LOAD_TIMEOUT: f64 = 25.0;
pub const SNAP_LINES: usize = 150;
pub const TEXT_CHARS: usize = 6000;
const SESSION_FREE: &[&str] = &[
    "config",
    "abort",
    "reload-extension",
    "ping",
    "close-url",
    "tabs",
    "downloads",
    "cleanup",
    "keep",
    "tab",
    "record-start",
    "record-stop",
    "record-status",
];
const REOPEN: &[&str] = &["snap", "text", "shot", "frames", "expect"];

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("web pattern")
}

pub static REF_RE: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:f\d+\.)?e\d+$"));
static REF: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:(f\d+)\.)?(e|d|o|w|t)(\d+)$"));
static NEW_TAB: LazyLock<Regex> = LazyLock::new(|| re(r"new tab (t\d+) opened from (t\d+)"));

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let text = match c.name.as_str() {
        "open" => cmds::open(d, ctx, c)?,
        "snap" => cmds::snap(d, ctx, c)?,
        "text" => cmds::text(d, ctx, c)?,
        "click" => cmds::click(d, ctx, c)?,
        "type" => cmds::type_in(d, ctx, c)?,
        "search" => cmds::search(d, ctx, c)?,
        "fill" => cmds::fill(d, ctx, c)?,
        "press" => cmds::press(d, ctx, c)?,
        "select" => cmds::select(d, ctx, c)?,
        "check" => cmds::check(d, ctx, c)?,
        "hover" => cmds::hover(d, ctx, c)?,
        "drag" => cmds::drag(d, ctx, c)?,
        "scroll" => cmds::scroll(d, ctx, c)?,
        "wait" => return wait::wait(d, ctx, c),
        "expect" => out(d, ctx, "expect", wait::spec(c)?, 20.0, SNAP_LINES)?,
        "table" => {
            let args = obj(json!({"target": c.pos.first()}));
            out(d, ctx, "table", args, 20.0, 120)?
        }
        "js" => cmds::js(d, ctx, c)?,
        "dismiss" => out(d, ctx, "dismiss", Map::new(), 20.0, SNAP_LINES)?,
        "upload" => cmds::upload(d, ctx, c)?,
        "tabs" => cmds::tabs(d, ctx, c)?,
        "tab" => match c.pos.first() {
            Some(id) => out(d, ctx, "tab", obj(json!({"id": id})), 15.0, SNAP_LINES)?,
            None => return Err(Fail::hint("tab needs an id", "lighting tabs")),
        },
        "close" => cmds::close(d, ctx, c)?,
        "back" | "forward" | "reload" => cmds::history(d, ctx, c)?,
        "dialog" => cmds::dialog(d, ctx, c)?,
        "downloads" => out(d, ctx, "downloads", Map::new(), 15.0, SNAP_LINES)?,
        "console" => out(d, ctx, "console", Map::new(), 15.0, 60)?,
        "viewport" => cmds::viewport(d, ctx, c)?,
        "net" => data::net(d, ctx, c)?,
        "tools" => data::tools(d, ctx)?,
        "call" => data::call_tool(d, ctx, c)?,
        "fetch" => data::fetch(d, ctx, c)?,
        "shot" => shot::shot(d, ctx, c)?,
        "frames" => shot::frames(d, ctx, c)?,
        name => return Err(super::route::later(name)),
    };
    Ok(Out::Text(text))
}

pub fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

pub fn flag(c: &Call, key: &str) -> Value {
    match c.flags.get(key) {
        Some(Flag::On) => Value::Bool(true),
        Some(Flag::Val(v)) => Value::String(v.clone()),
        None => Value::Null,
    }
}

pub fn text_of<'a>(msg: &'a Map<String, Value>, key: &str) -> &'a str {
    msg.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn num(v: Option<&Value>) -> i64 {
    v.and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0)
}

pub fn plural(n: i64, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

pub fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

pub fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s.max(0.0))
}

pub fn number(c: &Call, key: &str) -> Result<Option<i64>, Fail> {
    match c.val(key).filter(|v| !v.is_empty()) {
        Some(v) => {
            v.trim().parse().map(Some).map_err(|_| Fail::hint(format!("--{key} takes a number"), format!("--{key} 10")))
        }
        None => Ok(None),
    }
}

pub fn ref_kind(token: &str) -> Option<&str> {
    REF.captures(token).and_then(|m| m.get(2)).map(|m| m.as_str())
}

pub fn capped(d: &Daemon, ctx: &mut Ctx, text: &str, name: &str, lines: Option<usize>, chars: usize) -> String {
    let (text, cut) = cap(&d.home.join("out"), text, name, lines, chars);
    ctx.cut += cut;
    text
}

pub fn out(
    d: &Daemon,
    ctx: &mut Ctx,
    cmd: &str,
    args: Map<String, Value>,
    timeout: f64,
    lines: usize,
) -> Result<String, Fail> {
    let msg = call(d, ctx, cmd, args, timeout)?;
    Ok(capped(d, ctx, text_of(&msg, "out"), cmd, Some(lines), OUT_CHARS))
}

pub fn call(
    d: &Daemon,
    ctx: &mut Ctx,
    cmd: &str,
    args: Map<String, Value>,
    timeout: f64,
) -> Result<Map<String, Value>, Fail> {
    call_on(d, ctx, cmd, args, timeout, None)
}

pub fn call_on(
    d: &Daemon,
    ctx: &mut Ctx,
    cmd: &str,
    mut args: Map<String, Value>,
    timeout: f64,
    tab: Option<String>,
) -> Result<Map<String, Value>, Fail> {
    let host = launch::ensure_host(d)?;
    let explicit = tab.is_some();
    let own = match tab {
        Some(tab) => Some(tab),
        None => session_tab(d, ctx, cmd, &mut args)?,
    };
    let reply = host.call(cmd, Value::Object(args.clone()), own.as_deref(), secs(timeout), Some(ctx.group.as_str()));
    let msg = match reply {
        Ok(Value::Object(msg)) => msg,
        Ok(_) => Map::new(),
        Err(Called::Timeout) => {
            return Err(Fail::uncounted(format!("err: browser did not answer '{cmd}' within {}s", timeout as i64)));
        }
        Err(Called::Aborted) => return Err(Fail::uncounted(STOPPED)),
    };
    ctx.target = Some(("web".into(), host.brand.clone()));
    if !truthy(msg.get("ok")) {
        let err = match msg.get("error") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            Some(v) if yes(v) => v.to_string(),
            _ => "browser error".into(),
        };
        if !explicit
            && let Some(own) = &own
            && err.contains("no tab yet")
        {
            ctx.tabs.retain(|t| t != own);
            ctx.owned.remove(own);
            return call_on(d, ctx, cmd, args, timeout, None);
        }
        return Err(Fail::new(err));
    }
    track(d, ctx, cmd, &msg);
    if let Some(target) = msg.get("target").filter(|t| yes(t)) {
        ctx.last_target = Some(target.clone());
    }
    if let Some(Value::Object(place)) = msg.get("where")
        && !place.is_empty()
    {
        let mut place = place.clone();
        place.insert("kind".into(), json!("web"));
        ctx.place = Some(place);
    }
    if let Some(Value::Array(pairs)) = msg.get("keys")
        && !pairs.is_empty()
    {
        keys::harvest(&d.home, &keys::app_of(ctx), pairs);
    }
    Ok(msg)
}

fn session_tab(d: &Daemon, ctx: &mut Ctx, cmd: &str, args: &mut Map<String, Value>) -> Result<Option<String>, Fail> {
    if ctx.sid.is_empty() || SESSION_FREE.contains(&cmd) || (cmd == "close" && truthy(args.get("id"))) {
        return Ok(None);
    }
    if cmd == "open" {
        if truthy(args.get("new")) {
            return Ok(None);
        }
        if ctx.tabs.is_empty() {
            args.insert("new".into(), json!(true));
        }
        return Ok(ctx.tabs.first().cloned());
    }
    if let Some(first) = ctx.tabs.first() {
        return Ok(Some(first.clone()));
    }
    let Some(last) = ctx.closed_url.clone().filter(|u| !u.is_empty()) else {
        return Err(Fail::hint(
            "this session has no tab yet",
            "lighting open <url>, lighting tabs + lighting tab t3 to use one, or lighting windows -f <app> for an app",
        ));
    };
    if REOPEN.contains(&cmd) {
        call(d, ctx, "open", obj(json!({"url": last, "new": true, "quiet": true})), LOAD_TIMEOUT + 10.0)?;
        if let Some(first) = ctx.tabs.first().cloned() {
            ctx.note(&format!(
                "reopened {}: Lighting had closed this session's tab after an earlier reply",
                head(&last, 80)
            ));
            return Ok(Some(first));
        }
    }
    Err(Fail::hint(
        format!("this session has no tab: its tabs were closed after an earlier reply (last page {})", head(&last, 80)),
        format!("lighting open {last}"),
    ))
}

fn track(d: &Daemon, ctx: &mut Ctx, cmd: &str, msg: &Map<String, Value>) {
    if ctx.sid.is_empty() {
        return;
    }
    let tab = msg.get("where").and_then(|w| w.get("tab")).and_then(Value::as_str).filter(|t| !t.is_empty());
    if let Some(tab) = tab {
        ctx.closed_url = None;
        if cmd == "open" && truthy(msg.get("created")) {
            ctx.owned.insert(tab.to_string());
        }
        if cmd == "open" || cmd == "tab" || ctx.owned.contains(tab) || ctx.tabs.first().is_some_and(|t| t == tab) {
            front(ctx, tab);
        }
    }
    for event in d.hosts.events() {
        if let Some(m) = NEW_TAB.captures(&event)
            && ctx.owned.contains(&m[2])
            && !ctx.owned.contains(&m[1])
        {
            ctx.owned.insert(m[1].to_string());
            front(ctx, &m[1]);
        }
    }
}

fn front(ctx: &mut Ctx, tab: &str) {
    ctx.tabs.retain(|t| t != tab);
    ctx.tabs.insert(0, tab.to_string());
}

pub fn target_args<'a>(pos: &'a [String], c: &Call) -> Result<(Map<String, Value>, &'a [String]), Fail> {
    let Some(first) = pos.first() else { return Err(Fail::new("need a ref (e12) or \"text\"")) };
    let mut args = Map::new();
    if REF_RE.is_match(first) {
        args.insert("ref".into(), json!(first));
        return Ok((args, &pos[1..]));
    }
    args.insert("text".into(), json!(first));
    if c.on("first") {
        args.insert("first".into(), json!(true));
    }
    if c.on("role") {
        args.insert("role".into(), flag(c, "role"));
    }
    Ok((args, &pos[1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_and_counts_read_like_2x() {
        assert_eq!(ref_kind("f2.e14"), Some("e"));
        assert_eq!(ref_kind("w3"), Some("w"));
        assert_eq!(ref_kind("e"), None);
        assert!(REF_RE.is_match("e12") && REF_RE.is_match("f1.e2") && !REF_RE.is_match("t3"));
        assert_eq!(plural(1, "tab"), "1 tab");
        assert_eq!(plural(3, "tool"), "3 tools");
    }
}
