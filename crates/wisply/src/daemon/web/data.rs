use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};
use wisply_core::{Call, Fail, OUT_CHARS};

use super::super::ctx::Ctx;
use super::super::words::{hit, terms};
use super::super::{Daemon, hosts::truthy};
use super::url::leak_check;
use super::{CALL_TIMEOUT, TEXT_CHARS, call, capped, flag, head, html, num, number, obj, re};

static WWW: LazyLock<Regex> = LazyLock::new(|| re(r"^https?://(www\.)?"));
static PATH_PART: LazyLock<Regex> = LazyLock::new(|| re(r"[^.\[\]]+|\[\d*\]"));

pub fn net(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let got = call(d, ctx, "net", obj(json!({"op": "list"})), CALL_TIMEOUT)?;
    let calls = got.get("calls").and_then(Value::as_array).cloned().unwrap_or_default();
    if calls.is_empty() {
        return Ok(
            "no JSON answers seen in this tab yet (only GET calls after Wisply attached; reload to capture)".into()
        );
    }
    let rows: Vec<&Value> = calls.iter().rev().collect();
    let digits = c.pos.first().map(|p| p.trim_start_matches('n')).unwrap_or("");
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        let n: usize = digits.parse().unwrap_or(0);
        if n == 0 || n > rows.len() {
            return Err(Fail::hint(format!("no call n{n}"), "wisply net"));
        }
        let got = call(d, ctx, "net", obj(json!({"op": "body", "id": rows[n - 1].get("id")})), CALL_TIMEOUT)?;
        let url = pystr(got.get("url").unwrap_or(&Value::Null));
        let body = got.get("body").and_then(Value::as_str).unwrap_or("");
        let Ok(data) = serde_json::from_str::<Value>(body) else { return Ok(format!("n{n} {url}: not JSON")) };
        return Ok(format!(
            "n{n} GET {} ({})\n{}\n-> wisply fetch \"{url}\" --pick <path>",
            short_url(&url),
            kb(num(got.get("size"))),
            shape(&data, 0)
        ));
    }
    let words = terms(c.val("f").unwrap_or(""));
    let lines: Vec<String> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let url = r.get("url").and_then(Value::as_str).unwrap_or("");
            (words.is_empty() || hit(&words, &[url]))
                .then(|| format!("n{} GET {} (json {})", i + 1, short_url(url), kb(num(r.get("size")))))
        })
        .collect();
    let body = if lines.is_empty() { "no call matches".to_string() } else { lines.join("\n") };
    Ok(format!("{body}\n-> wisply net <n> shows the data"))
}

pub fn tools(d: &Daemon, ctx: &mut Ctx) -> Result<String, Fail> {
    let got = call(d, ctx, "webmcp", obj(json!({"op": "list"})), CALL_TIMEOUT)?;
    let tools = got.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
    if tools.is_empty() {
        let why = if truthy(got.get("api")) { "" } else { " (the browser has no WebMCP here)" };
        return Ok(format!("this page offers no WebMCP tools{why}"));
    }
    let lines: Vec<String> = tools
        .iter()
        .map(|t| {
            let mut args: Vec<&str> = t
                .get("schema")
                .and_then(|s| s.get("properties"))
                .and_then(Value::as_object)
                .map(|p| p.keys().map(String::as_str).collect())
                .unwrap_or_default();
            args.sort_unstable();
            let read_only = if truthy(t.get("readOnly")) { " read-only" } else { "" };
            let name = t.get("name").and_then(Value::as_str).unwrap_or("");
            let what = t.get("description").and_then(Value::as_str).unwrap_or("");
            format!("{name}({}){read_only} - {what}", args.join(", "))
        })
        .collect();
    Ok(format!("{}\n-> wisply call <tool> '{{\"arg\": 1}}'", lines.join("\n")))
}

pub fn call_tool(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let Some(name) = c.pos.first() else { return Err(Fail::hint("call needs a tool name", "wisply tools")) };
    let raw = c.pos[1..].join(" ");
    let args: Value = serde_json::from_str(if raw.is_empty() { "{}" } else { raw.as_str() })
        .map_err(|_| Fail::hint("the arguments must be JSON", format!("wisply call {name} '{{\"q\": \"x\"}}'")))?;
    let got = call(d, ctx, "webmcp", obj(json!({"op": "list"})), CALL_TIMEOUT)?;
    let tools = got.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(tool) = tools.iter().rev().find(|t| t.get("name").and_then(Value::as_str) == Some(name)) else {
        return Err(Fail::hint(format!("no tool {name} on this page"), "wisply tools"));
    };
    if !truthy(tool.get("readOnly")) && !c.on("yes") {
        return Err(Fail::hint(
            format!("{name} can change things on the site"),
            format!("only if the user agreed: wisply call {name} ... --yes"),
        ));
    }
    let got = call(d, ctx, "webmcp", obj(json!({"op": "call", "name": name, "args": args})), 60.0)?;
    let res = pystr(got.get("result").unwrap_or(&Value::Null));
    Ok(capped(d, ctx, &format!("ok {name} -> {res}"), "call", None, OUT_CHARS))
}

pub fn fetch(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let Some(url) = c.pos.first() else {
        return Err(Fail::hint("fetch needs a url", "wisply fetch /api/user --pick login"));
    };
    leak_check(d, url, c)?;
    let method = c.val("method").filter(|m| !m.is_empty()).unwrap_or("GET").to_uppercase();
    let mut args = obj(json!({"url": url, "method": method, "cookies": c.on("cookies")}));
    if c.on("body") {
        args.insert("body".into(), flag(c, "body"));
    }
    let got = call(d, ctx, "fetch", args, 30.0)?;
    let res = got.get("fetch").and_then(Value::as_object).cloned().unwrap_or_default();
    let body = res.get("text").and_then(Value::as_str).unwrap_or("");
    let kind = res.get("type").and_then(Value::as_str).unwrap_or("");
    let status = pystr(res.get("status").unwrap_or(&Value::Null));
    let head = format!("{status} {}", kind.split(';').next().unwrap_or(""));
    let Ok(mut data) = serde_json::from_str::<Value>(body) else {
        let limit = number(c, "max")?.map_or(TEXT_CHARS, |n| usize::try_from(n).unwrap_or(0));
        if !kind.contains("html") {
            return Ok(capped(d, ctx, &format!("{head}\n{body}"), "fetch", None, limit));
        }
        let base = res.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()).unwrap_or(url);
        let text = html::convert(body, base).1;
        return Ok(capped(d, ctx, &format!("{head} (html converted)\n{text}"), "fetch", None, limit));
    };
    if let Some(path) = c.val("pick").filter(|p| !p.is_empty()) {
        data = pick(&data, path);
    }
    let text = serde_json::to_string(&data).unwrap_or_default();
    Ok(capped(d, ctx, &format!("{head}\n{text}"), "fetch", None, TEXT_CHARS))
}

pub fn kb(size: i64) -> String {
    if size < 1024 { format!("{size} B") } else { format!("{:.0} KB", size as f64 / 1024.0) }
}

pub fn short_url(url: &str) -> String {
    let u = WWW.replace(url, "");
    if u.chars().count() <= 110 { u.into_owned() } else { format!("{}...", head(&u, 107)) }
}

fn scalar(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn shorten(s: String, max: usize) -> String {
    if s.chars().count() <= max { s } else { format!("{}...", head(&s, max - 3)) }
}

pub fn shape(data: &Value, depth: usize) -> String {
    match data {
        Value::Array(items) => {
            let inner = match items.first() {
                Some(first) if depth < 2 => format!(": {}", shape(first, depth + 1)),
                _ => String::new(),
            };
            format!("[] ({}){inner}", items.len())
        }
        Value::Object(map) => {
            let parts: Vec<String> = map.iter().take(12).map(|(k, v)| part(k, v, depth)).collect();
            let more = map.len().saturating_sub(12);
            let tail = if more > 0 { format!(", +{more} more") } else { String::new() };
            format!("{}{tail}", parts.join(", "))
        }
        _ => shorten(scalar(data), 60),
    }
}

fn part(key: &str, v: &Value, depth: usize) -> String {
    match v {
        Value::Array(items) => {
            let inner = match items.first() {
                Some(first @ Value::Object(_)) if depth < 2 => format!(" {{{}}}", shape(first, depth + 1)),
                _ => String::new(),
            };
            format!("{key}[] ({}){inner}", items.len())
        }
        Value::Object(inner) if depth >= 1 => {
            format!("{key}{{{}}}", inner.keys().take(6).map(String::as_str).collect::<Vec<_>>().join(", "))
        }
        Value::Object(_) => format!("{key}{{{}}}", shape(v, depth + 1)),
        _ => format!("{key}: {}", shorten(scalar(v), 40)),
    }
}

pub fn pick(data: &Value, path: &str) -> Value {
    let mut cur = vec![data.clone()];
    for part in PATH_PART.find_iter(path).map(|m| m.as_str()) {
        let mut next = Vec::new();
        for c in &cur {
            if part == "[]" {
                if let Value::Array(items) = c {
                    next.extend(items.iter().cloned());
                }
            } else if let Some(index) = part.strip_prefix('[').and_then(|p| p.strip_suffix(']')) {
                if let (Value::Array(items), Ok(i)) = (c, index.parse::<usize>())
                    && let Some(item) = items.get(i)
                {
                    next.push(item.clone());
                }
            } else if let Some(v) = c.as_object().and_then(|m| m.get(part)) {
                next.push(v.clone());
            } else if let Value::Array(items) = c {
                next.extend(items.iter().filter_map(|x| x.as_object().and_then(|m| m.get(part))).cloned());
            }
        }
        cur = next;
    }
    if cur.len() == 1 && !path.contains("[]") { cur.remove(0) } else { Value::Array(cur) }
}

pub fn pystr(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => pyrepr(other),
    }
}

fn pyrepr(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quoted(s),
        Value::Array(items) => format!("[{}]", items.iter().map(pyrepr).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => {
            let pairs: Vec<String> = map.iter().map(|(k, v)| format!("{}: {}", quoted(k), pyrepr(v))).collect();
            format!("{{{}}}", pairs.join(", "))
        }
    }
}

fn quoted(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_helpers_match_2x() {
        let data = json!({"users": [{"login": "a", "id": 1}, {"login": "b", "id": 2}], "meta": {"page": 1}});
        assert_eq!(pick(&data, "users[].login"), json!(["a", "b"]));
        assert_eq!(pick(&data, "users[1].login"), json!("b"));
        assert_eq!(pick(&data, "users.login"), json!(["a", "b"]));
        assert_eq!(pick(&data, "meta.page"), json!(1));
        assert_eq!(pick(&data, "nope"), json!([]));
        assert_eq!(shape(&data, 0), "users[] (2) {login: \"a\", id: 1}, meta{page: 1}");
        assert_eq!(shape(&json!([1, 2]), 0), "[] (2): 1");
        assert_eq!(shape(&json!("x".repeat(70)), 0), format!("\"{}...", "x".repeat(56)));
        assert_eq!(kb(512), "512 B");
        assert_eq!(kb(1536), "2 KB");
        assert_eq!(kb(2560), "2 KB");
        assert_eq!(short_url("https://www.github.com/a"), "github.com/a");
        assert_eq!(
            pystr(&json!({"ok": true, "n": [1, null], "s": "it's"})),
            "{'ok': True, 'n': [1, None], 's': \"it's\"}"
        );
        assert_eq!(pystr(&json!("plain")), "plain");
        assert_eq!(pystr(&Value::Null), "None");
    }
}
