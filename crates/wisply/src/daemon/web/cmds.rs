use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use regex::Regex;
use serde_json::{Map, Value, json};
use wisply_core::{Call, Fail, Flag, OUT_CHARS};

use super::super::config::NAV_LINES;
use super::super::ctx::Ctx;
use super::super::words::{best_only, terms, tiers};
use super::super::{Daemon, brain};
use super::url::{leak_check, local_file, normalize_url, watch_url, web_scheme};
use super::{
    CALL_TIMEOUT, LOAD_TIMEOUT, REF_RE, SNAP_LINES, TEXT_CHARS, call, capped, flag, keys, num, number, obj, out,
    plural, re, sites, tabkeys, target_args, text_of,
};

const ACTED_LINES: usize = NAV_LINES + 8;
const SEARCH_NAV: Duration = Duration::from_millis(1500);
const UPLOAD_INLINE: u64 = 8 << 20;

static FULL_SNAP: LazyLock<Regex> =
    LazyLock::new(|| re(r"since your last snap[^\n]*\(all \d+ lines, ~([\d,]+) tokens"));
static SEARCHY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)such|search"));
static CLOSED: LazyLock<Regex> = LazyLock::new(|| re(r"^closed (t\d+)"));
static VIEWPORT: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d{2,4})x(\d{2,4})$"));

fn with(c: &Call, name: &str, pos: Vec<String>, keep: &[&str]) -> Call {
    let flags = c.flags.iter().filter(|(k, _)| keep.is_empty() || keep.contains(&k.as_str()));
    Call { name: name.into(), pos, flags: flags.map(|(k, v)| (k.clone(), v.clone())).collect() }
}

pub fn open(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let Some(first) = c.pos.first() else { return Err(Fail::hint("open needs a url", "wisply open github.com")) };
    let mut url = normalize_url(first);
    if !web_scheme(&url) {
        return Err(super::super::route::later("open for apps"));
    }
    if let Some(watch) = watch_url(&url) {
        url = watch;
        ctx.note("opened the embed link as its watch page (YouTube shows Error 153 for embeds opened directly)");
    }
    leak_check(d, &url, c)?;
    if c.on("text") {
        let args = obj(json!({"url": url, "new": c.on("new"), "quiet": true}));
        let head = out(d, ctx, "open", args, LOAD_TIMEOUT + 10.0, SNAP_LINES)?;
        let first_line = head.split('\n').next().unwrap_or("").to_string();
        return Ok(format!("{first_line}\n{}", text(d, ctx, c)?));
    }
    let args = obj(json!({"url": url, "new": c.on("new"), "filter": flag(c, "f"), "scope": flag(c, "s"),
                          "media": c.on("media")}));
    let msg = call(d, ctx, "open", args, LOAD_TIMEOUT + 10.0)?;
    let res = capped(d, ctx, text_of(&msg, "out"), "open", Some(SNAP_LINES), OUT_CHARS);
    let mut extra = keys::hint(&d.home, &keys::app_of(ctx));
    let net = num(msg.get("net"));
    if net != 0 {
        extra.push_str(&format!("\ndata: {} (wisply net)", plural(net, "JSON call")));
    }
    let tools = num(msg.get("webmcp"));
    if tools != 0 {
        extra.push_str(&format!("\nwebmcp: {} (wisply tools)", plural(tools, "tool")));
    }
    Ok(match res.split_once('\n') {
        Some((head, rest)) => format!("{head}{extra}\n{rest}"),
        None => format!("{res}{extra}"),
    })
}

pub fn snap(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let mut args =
        obj(json!({"all": c.on("all"), "diff": c.on("diff"), "force": c.on("force"), "media": c.on("media"),
                              "full": c.on("full"), "links": c.on("links")}));
    if c.on("f") {
        args.insert("filter".into(), flag(c, "f"));
    }
    let scope =
        if c.on("s") { Some(flag(c, "s")) } else { c.pos.first().filter(|p| REF_RE.is_match(p)).map(|p| json!(p)) };
    let trim = scope.is_none() && brain::wants_prune(c);
    if let Some(scope) = scope {
        args.insert("scope".into(), scope);
    }
    if c.on("frame") {
        args.insert("frame".into(), flag(c, "frame"));
    }
    let limit = if c.on("all") { 100_000 } else { SNAP_LINES };
    let msg = call(d, ctx, "snap", args, 30.0)?;
    let text = text_of(&msg, "out");
    if let Some(m) = FULL_SNAP.captures(text) {
        let tokens: usize = m[1].replace(',', "").parse().unwrap_or(0);
        ctx.cut += tokens.saturating_mul(4).min(OUT_CHARS).saturating_sub(text.chars().count());
    }
    let text = if trim { brain::prune(d, ctx, text, &brain::page_key(&ctx.url())) } else { text.to_string() };
    Ok(capped(d, ctx, &text, "snap", Some(limit), OUT_CHARS))
}

pub fn text(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let args = obj(json!({"filter": flag(c, "f"), "raw": c.on("raw"), "links": c.on("links")}));
    let limit = number(c, "max")?.map_or(TEXT_CHARS, |n| usize::try_from(n).unwrap_or(0));
    let msg = call(d, ctx, "text", args, 40.0)?;
    Ok(capped(d, ctx, text_of(&msg, "out"), "text", None, limit))
}

pub fn click(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let (mut args, _) = target_args(&c.pos, c)?;
    for key in ["yes", "force", "trace"] {
        args.insert(key.into(), json!(c.on(key)));
    }
    if c.on("right") {
        args.insert("button".into(), json!("right"));
    }
    if c.on("double") {
        args.insert("count".into(), json!(2));
    }
    out(d, ctx, "click", args, LOAD_TIMEOUT + 10.0, ACTED_LINES)
}

pub fn type_in(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let (mut args, rest) = target_args(&c.pos, c)?;
    let value = match ctx.secret.clone() {
        Some(secret) => {
            args.insert("secret".into(), json!(true));
            secret
        }
        None if rest.is_empty() => return Err(Fail::hint("type needs text", "wisply type e3 \"hello\"")),
        None => rest.join(" "),
    };
    args.insert("value".into(), json!(value));
    args.insert("append".into(), json!(c.on("append")));
    args.insert("enter".into(), json!(c.on("enter")));
    let res = out(d, ctx, "type", args, LOAD_TIMEOUT + 10.0, ACTED_LINES)?;
    let target = ctx.last_target.clone().unwrap_or(Value::Null);
    let role = target.get("role").and_then(Value::as_str).unwrap_or("");
    let name = target.get("name").and_then(Value::as_str).unwrap_or("");
    let searchy = role == "searchbox" || role == "combobox" || SEARCHY.is_match(name);
    if c.on("enter") && !c.on("nolearn") && ctx.secret.is_none() && searchy {
        sites::learn(&d.home, &ctx.url(), &value);
    }
    Ok(res)
}

pub fn search(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    if c.pos.is_empty() {
        return Err(Fail::hint("search needs words", "wisply search \"paper plugin\" | search youtube \"lofi\""));
    }
    let site = if c.pos.len() > 1 { sites::named(&d.home, &c.pos[0]) } else { None };
    let words = if site.is_some() { c.pos[1..].join(" ") } else { c.pos.join(" ") };
    let host = site.clone().or_else(|| Some(sites::host_of(&ctx.url())).filter(|h| !h.is_empty()));
    if let Some(tpl) = host.and_then(|h| sites::template(&d.home, &h)) {
        return open(d, ctx, &with(c, "open", vec![sites::build(&tpl, &words)], &["f", "new", "text"]));
    }
    if let Some(site) = &site {
        open(d, ctx, &with(c, "open", vec![format!("https://{site}")], &["new"]))?;
    }
    let mut found = call(d, ctx, "search-field", Map::new(), CALL_TIMEOUT)?;
    if let Some(button) = found.get("button").filter(|b| super::yes(b)).cloned() {
        call(d, ctx, "click", obj(json!({"ref": button})), LOAD_TIMEOUT + 10.0)?;
        found = call(d, ctx, "search-field", Map::new(), CALL_TIMEOUT)?;
    }
    let Some(field) = found.get("ref").and_then(Value::as_str).filter(|r| !r.is_empty()).map(String::from) else {
        return Err(Fail::hint("no search field on this page", "wisply snap -f search, then type e<N> words --enter"));
    };
    let before = ctx.url();
    let mut typing = with(c, "type", vec![field, words.clone()], &[]);
    typing.flags.insert("enter".into(), Flag::On);
    typing.flags.insert("nolearn".into(), Flag::On);
    let res = type_in(d, ctx, &typing)?;
    let mut url = ctx.url();
    let end = Instant::now() + SEARCH_NAV;
    while url == before && Instant::now() < end {
        thread::sleep(Duration::from_millis(100));
        let place = call(d, ctx, "where", Map::new(), CALL_TIMEOUT)?;
        if let Some(now) =
            place.get("where").and_then(|w| w.get("url")).and_then(Value::as_str).filter(|u| !u.is_empty())
        {
            url = now.to_string();
        }
    }
    Ok(match sites::learn(&d.home, &url, &words) {
        Some(tpl) => format!(
            "{res}\n! learned the search URL of {}: next time search jumps there directly",
            sites::host_of(&tpl)
        ),
        None => res,
    })
}

pub fn fill(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let mut pairs = Vec::new();
    for p in &c.pos {
        let Some((label, value)) = p.split_once('=') else {
            let hint = match c.pos.as_slice() {
                [r, _, ..] if super::ref_kind(r) == Some("e") => {
                    format!("wisply type {r} \"<value>\" (one field by its ref)")
                }
                _ => "wisply fill \"Email=me@x.de\" \"Password=@secret\" --env PASS".into(),
            };
            return Err(Fail::hint("fill wants Label=value pairs", hint));
        };
        let value = match (value, &ctx.secret) {
            ("@secret", Some(secret)) => secret.clone(),
            ("@secret", None) => return Err(Fail::new("@secret needs --env VAR")),
            (value, _) => value.to_string(),
        };
        pairs.push(json!({"label": label.trim(), "value": value}));
    }
    if pairs.is_empty() {
        return Err(Fail::new("fill wants Label=value pairs"));
    }
    let args = obj(json!({"pairs": pairs, "submit": c.on("submit"), "yes": c.on("yes")}));
    out(d, ctx, "fill", args, LOAD_TIMEOUT + 15.0, ACTED_LINES)
}

pub fn press(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    if c.pos.is_empty() {
        return Err(Fail::hint("press needs keys", "wisply press Enter | ctrl+a | Escape"));
    }
    if let Some(done) = tabkeys::press(d, ctx, c)? {
        return Ok(done);
    }
    let res = out(d, ctx, "press", obj(json!({"keys": c.pos})), LOAD_TIMEOUT + 10.0, ACTED_LINES)?;
    let mut lines = res.splitn(3, '\n');
    let first = lines.next().unwrap_or("");
    if c.pos.len() == 1
        && first.contains(" new")
        && let Some(second) = lines.next()
    {
        keys::learn(&d.home, &keys::app_of(ctx), &c.pos[0], second);
    }
    Ok(res)
}

pub fn select(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let (mut args, rest) = target_args(&c.pos, c)?;
    if rest.is_empty() {
        return Err(Fail::hint("select needs an option", "wisply select e7 \"Germany\""));
    }
    args.insert("option".into(), json!(rest.join(" ")));
    out(d, ctx, "select", args, LOAD_TIMEOUT, SNAP_LINES)
}

pub fn check(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let (mut args, rest) = target_args(&c.pos, c)?;
    let off = rest.first().is_some_and(|r| ["off", "false", "0", "no", "uncheck"].contains(&r.to_lowercase().as_str()));
    args.insert("on".into(), json!(!off));
    out(d, ctx, "check", args, LOAD_TIMEOUT, SNAP_LINES)
}

pub fn hover(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let (args, _) = target_args(&c.pos, c)?;
    out(d, ctx, "hover", args, LOAD_TIMEOUT, SNAP_LINES)
}

pub fn drag(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let [from, to, ..] = c.pos.as_slice() else { return Err(Fail::hint("drag needs two refs", "wisply drag e3 e9")) };
    out(d, ctx, "drag", obj(json!({"from": from, "to": to})), LOAD_TIMEOUT, SNAP_LINES)
}

pub fn scroll(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let mut args = Map::new();
    if let Some(r) = c.pos.iter().rev().find(|p| REF_RE.is_match(p)) {
        args.insert("ref".into(), json!(r));
    }
    let way = ["up", "down", "top", "bottom"]
        .into_iter()
        .find(|w| c.on(w) || c.pos.iter().any(|p| p.eq_ignore_ascii_case(w)));
    if let Some(way) = way {
        args.insert("dir".into(), json!(way));
    }
    if c.on("until") {
        args.insert("until".into(), flag(c, "until"));
    }
    for key in ["delta", "max"] {
        if let Some(n) = number(c, key)? {
            args.insert(key.into(), json!(n));
        }
    }
    out(d, ctx, "scroll", args, 60.0, SNAP_LINES)
}

pub fn js(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let mut code = c.pos.join(" ");
    if let Some(file) = c.val("file").filter(|f| !f.is_empty()) {
        let path = local_file(ctx, file);
        code = fs::read_to_string(&path)
            .map_err(|e| Fail::new(format!("cannot read {}: {}", path.display(), reason(&e))))?;
    }
    if code.trim().is_empty() {
        return Err(Fail::hint("js needs code", "wisply js \"document.title\" or wisply js --file script.js"));
    }
    out(d, ctx, "js", obj(json!({"code": code})), 25.0, 200)
}

fn reason(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.find(" (os error") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

fn mime(path: &Path) -> &'static str {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/x-wav",
        _ => "application/octet-stream",
    }
}

pub fn upload(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let Some(field) = c.pos.first().filter(|p| REF_RE.is_match(p)) else {
        let got = match c.pos.first() {
            None => "nothing".to_string(),
            Some(p) if local_file(ctx, p).is_file() => "the file".to_string(),
            Some(p) => format!("\"{p}\""),
        };
        return Err(Fail::hint(
            format!("upload needs the ref of the upload button or file field first, got {got}"),
            "wisply snap -f \"upload|file\", then wisply upload e5 C:/path/file.png",
        ));
    };
    if c.pos.len() < 2 {
        return Err(Fail::hint(
            "upload needs the files after the ref",
            format!("wisply upload {field} C:/path/file.png"),
        ));
    }
    let mut files = Vec::new();
    for raw in &c.pos[1..] {
        let path = local_file(ctx, raw);
        if !path.exists() {
            return Err(Fail::new(format!("file not found: {}", path.display())));
        }
        files.push(path);
    }
    let names: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let mut args = obj(json!({"ref": c.pos[0], "files": names}));
    let size: u64 = files.iter().map(|p| fs::metadata(p).map_or(0, |m| m.len())).sum();
    if c.pos[0].starts_with('f') && size <= UPLOAD_INLINE {
        let mut inline = Vec::new();
        for p in &files {
            let bytes = fs::read(p).map_err(|e| Fail::new(format!("cannot read {}: {}", p.display(), reason(&e))))?;
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            inline.push(json!({"name": name, "mime": mime(p),
                               "b64": base64::engine::general_purpose::STANDARD.encode(bytes)}));
        }
        args.insert("data".into(), json!(inline));
    }
    out(d, ctx, "upload", args, 30.0, SNAP_LINES)
}

pub fn tabs(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let all = c.on("all") || c.val("f").is_some_and(|f| !f.is_empty());
    let msg = call(d, ctx, "tabs", obj(json!({"all": all})), 15.0)?;
    let mut text = text_of(&msg, "out").to_string();
    if let Some(f) = c.val("f").filter(|f| !f.is_empty()) {
        let words = terms(f);
        let rows: Vec<(Vec<u8>, &str)> = text
            .split('\n')
            .filter(|l| !l.trim().is_empty())
            .map(|l| (tiers(&words, &[l]), l))
            .filter(|(t, _)| t.iter().any(|x| *x > 0))
            .collect();
        let best = best_only(&rows);
        text = if best.is_empty() { format!("no tab matches \"{f}\"") } else { best.join("\n") };
    }
    Ok(capped(d, ctx, &text, "tabs", Some(80), OUT_CHARS))
}

pub fn close(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let id = c.pos.first().cloned();
    let which = id.clone().or_else(|| ctx.tabs.first().cloned());
    let mine = !ctx.sid.is_empty() && which.is_some_and(|t| ctx.owned.contains(&t));
    let res = out(d, ctx, "close", obj(json!({"id": id, "force": c.on("force") || mine})), 15.0, SNAP_LINES)?;
    let Some(gone) = CLOSED.captures(&res).map(|m| m[1].to_string()).filter(|_| !ctx.sid.is_empty()) else {
        return Ok(res);
    };
    ctx.tabs.retain(|t| *t != gone);
    ctx.owned.remove(&gone);
    let rest = match ctx.tabs.first() {
        Some(t) => format!("target now {t}"),
        None => "no tab left in this session".into(),
    };
    Ok(format!("closed {gone}, {rest}"))
}

pub fn history(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let msg = call(d, ctx, &c.name, Map::new(), LOAD_TIMEOUT + 10.0)?;
    Ok(capped(d, ctx, text_of(&msg, "out"), &c.name, Some(SNAP_LINES), OUT_CHARS))
}

pub fn dialog(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let verb = c.pos.first().map(|p| p.to_lowercase()).unwrap_or_default();
    if !["accept", "dismiss", "ok", "cancel"].contains(&verb.as_str()) {
        return Err(Fail::new("dialog accept|dismiss [text]"));
    }
    let text = if c.pos.len() > 1 { json!(c.pos[1..].join(" ")) } else { Value::Null };
    let args = obj(json!({"accept": verb == "accept" || verb == "ok", "text": text}));
    out(d, ctx, "dialog", args, 15.0, SNAP_LINES)
}

pub fn viewport(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let Some(size) = c.pos.first() else {
        return Err(Fail::hint("viewport needs WxH or reset", "wisply viewport 390x844"));
    };
    if size == "reset" || size == "off" {
        return out(d, ctx, "viewport", Map::new(), CALL_TIMEOUT, SNAP_LINES);
    }
    let Some(m) = VIEWPORT.captures(size) else {
        return Err(Fail::hint("viewport wants WxH like 390x844", "wisply viewport 390x844"));
    };
    let args = obj(json!({"width": m[1].parse::<u32>().unwrap_or(0), "height": m[2].parse::<u32>().unwrap_or(0)}));
    out(d, ctx, "viewport", args, CALL_TIMEOUT, SNAP_LINES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_name_their_type() {
        assert_eq!(mime(Path::new("a/B.PNG")), "image/png");
        assert_eq!(mime(Path::new("x.bin")), "application/octet-stream");
    }
}
