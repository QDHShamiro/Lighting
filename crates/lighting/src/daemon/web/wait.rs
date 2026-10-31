use std::sync::LazyLock;
use std::time::{Duration, Instant};

use lt_core::{Call, Fail, Flag};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::super::ctx::Ctx;
use super::super::{Daemon, Out, Pending};
use super::{CALL_TIMEOUT, LOAD_TIMEOUT, REF_RE, SNAP_LINES, call, call_on, obj, out, re, secs};

const WAIT_SLEEP_MAX_MS: u64 = 60_000;

static TIME: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*(\d+(?:\.\d+)?)\s*(ms|s|m|h)?\s*$"));

pub fn spec(c: &Call) -> Result<Map<String, Value>, Fail> {
    if c.pos.is_empty() {
        return Err(Fail::new("needs \"text\", url:part, css:selector, a ref or milliseconds"));
    }
    let mut s = obj(json!({"gone": c.on("gone")}));
    let p = c.pos.join(" ");
    if let Some(url) = p.strip_prefix("url:") {
        s.insert("url".into(), json!(url));
    } else if let Some(css) = p.strip_prefix("css:") {
        s.insert("css".into(), json!(css));
    } else if REF_RE.is_match(&p) {
        s.insert("ref".into(), json!(p));
    } else if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
        s.insert("ms".into(), json!(p.parse::<u64>().unwrap_or(u64::MAX)));
    } else {
        s.insert("text".into(), json!(p));
    }
    Ok(s)
}

pub fn parse_ms(value: Option<&Flag>, default: u64) -> Result<u64, Fail> {
    let Some(Flag::Val(raw)) = value else { return Ok(default) };
    if raw.is_empty() {
        return Ok(default);
    }
    let low = raw.to_lowercase();
    let bad = || Fail::hint(format!("cannot read the time '{raw}'"), "use 90s, 10m or milliseconds");
    let m = TIME.captures(&low).ok_or_else(bad)?;
    let n: f64 = m[1].parse().map_err(|_| bad())?;
    let unit = match m.get(2).map(|u| u.as_str()) {
        Some("s") => 1000.0,
        Some("m") => 60_000.0,
        Some("h") => 3_600_000.0,
        _ => 1.0,
    };
    Ok((n * unit) as u64)
}

pub fn wait(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let mut s = spec(c)?;
    let reload = c.on("reload");
    if let Some(ms) = s.get("ms").and_then(Value::as_u64)
        && !reload
    {
        let ms = ms.min(WAIT_SLEEP_MAX_MS);
        let pause = Duration::from_millis(ms);
        return Ok(Out::Wait(Pending::new(pause, pause, move |_, _, last| Ok(last.then(|| format!("ok ({ms} ms)"))))));
    }
    if reload {
        return wait_reload(d, ctx, c, s);
    }
    let timeout = parse_ms(c.flags.get("timeout"), 10_000)?;
    s.insert("timeout".into(), json!(timeout));
    Ok(Out::Text(out(d, ctx, "wait", s, timeout as f64 / 1000.0 + 10.0, SNAP_LINES)?))
}

fn found(d: &Daemon, ctx: &mut Ctx, s: &Map<String, Value>, tab: Option<String>) -> Result<bool, Fail> {
    match call_on(d, ctx, "expect", s.clone(), CALL_TIMEOUT, tab) {
        Ok(_) => Ok(true),
        Err(e) if e.msg.starts_with("fail:") => Ok(false),
        Err(e) => Err(e),
    }
}

fn wait_reload(d: &Daemon, ctx: &mut Ctx, c: &Call, s: Map<String, Value>) -> Result<Out, Fail> {
    if s.contains_key("ms") {
        return Err(Fail::hint(
            "--reload waits for \"text\", url:, css: or a ref",
            "wisply wait \"All checks have passed\" --reload 15s",
        ));
    }
    let every = (parse_ms(c.flags.get("reload"), 15_000)? as f64 / 1000.0).max(3.0);
    let total = parse_ms(c.flags.get("timeout"), 600_000)? as f64 / 1000.0;
    let place = call(d, ctx, "where", Map::new(), CALL_TIMEOUT)?;
    let tab = place.get("where").and_then(|w| w.get("tab")).and_then(Value::as_str).filter(|t| !t.is_empty());
    let tab = tab.map(String::from);
    let t0 = Instant::now();
    if found(d, ctx, &s, tab.clone())? {
        return Ok(Out::Text("ok (already there)".into()));
    }
    let (mut reloads, mut next) = (0u32, t0 + secs(every));
    Ok(Out::Wait(Pending::new(secs(total), Duration::from_secs(1), move |d, ctx, last| {
        if found(d, ctx, &s, tab.clone())? {
            let word = if reloads == 1 { "reload" } else { "reloads" };
            return Ok(Some(format!("ok after {reloads} {word} ({} s)", t0.elapsed().as_secs())));
        }
        if last {
            return Err(Fail::hint(
                format!("not there after {} s ({reloads} reloads)", total as u64),
                format!("wisply wait ... --reload {}s --timeout {}m", every as u64, ((total / 30.0) as u64).max(1)),
            ));
        }
        if Instant::now() >= next {
            call_on(d, ctx, "reload", Map::new(), LOAD_TIMEOUT + 10.0, tab.clone())?;
            reloads += 1;
            next = Instant::now() + secs(every);
        }
        Ok(None)
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lt_core::parse;

    fn call_of(words: &[&str]) -> Call {
        parse(&words.iter().map(|w| (*w).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn wait_specs_and_times_read_like_2x() {
        assert_eq!(spec(&call_of(&["wait", "url:/done"])).unwrap()["url"], "/done");
        assert_eq!(spec(&call_of(&["wait", "css:.ok"])).unwrap()["css"], ".ok");
        assert_eq!(spec(&call_of(&["wait", "e12"])).unwrap()["ref"], "e12");
        assert_eq!(spec(&call_of(&["wait", "1500"])).unwrap()["ms"], 1500);
        let s = spec(&call_of(&["wait", "All", "checks", "--gone"])).unwrap();
        assert_eq!((s["text"].as_str(), s["gone"].as_bool()), (Some("All checks"), Some(true)));
        assert!(spec(&call_of(&["wait"])).is_err());
        let val = |v: &str| Flag::Val(v.into());
        assert_eq!(parse_ms(None, 10_000).unwrap(), 10_000);
        assert_eq!(parse_ms(Some(&Flag::On), 7).unwrap(), 7);
        assert_eq!(parse_ms(Some(&val("90s")), 0).unwrap(), 90_000);
        assert_eq!(parse_ms(Some(&val(" 1.5 M ")), 0).unwrap(), 90_000);
        assert_eq!(parse_ms(Some(&val("250")), 0).unwrap(), 250);
        assert_eq!(parse_ms(Some(&val("2h")), 0).unwrap(), 7_200_000);
        let bad = parse_ms(Some(&val("soon")), 0).unwrap_err().text();
        assert_eq!(bad, "err: cannot read the time 'soon' -> try: use 90s, 10m or milliseconds");
    }
}
