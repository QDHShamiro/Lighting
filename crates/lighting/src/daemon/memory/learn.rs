use std::sync::LazyLock;

use lt_core::{APP_ONLY, CHAT, SHARED, WEB_ONLY};
use lt_memory::{Lesson, now, oneline};
use regex::Regex;

use super::super::Daemon;
use super::super::ctx::Ctx;
use super::with_store;

const WINDOW_S: f64 = 60.0;
const KIND_CHARS: usize = 80;
const NOT_LEARNED: &[&str] = &["tabs", "downloads", "console", "tools"];

static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""[^"]*"|'[^']*'"#).expect("quoted pattern"));
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").expect("digit pattern"));

pub struct Failed {
    sig: String,
    fix: String,
    target: String,
    at: f64,
}

pub struct Offered {
    sig: String,
    fix: String,
}

fn learnable(name: &str) -> bool {
    let action = SHARED.contains(&name) || WEB_ONLY.contains(&name) || APP_ONLY.contains(&name) || CHAT.contains(&name);
    action && !NOT_LEARNED.contains(&name)
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host.strip_prefix("www.").unwrap_or(host)
}

pub fn target(ctx: &Ctx) -> String {
    match ctx.target.as_ref() {
        Some((kind, _)) if kind == "web" => host(&ctx.url()).to_lowercase(),
        Some((kind, name)) => format!("{kind}:{}", name.to_lowercase()),
        None => String::new(),
    }
}

pub fn kind(out: &str) -> String {
    let first = out.lines().next().unwrap_or("");
    let first = first.strip_prefix("err: ").unwrap_or(first);
    let first = first.split(" -> try:").next().unwrap_or(first);
    let plain = QUOTED.replace_all(first, "…");
    let plain = DIGITS.replace_all(&plain, "#").to_lowercase();
    plain.trim().chars().take(KIND_CHARS).collect()
}

pub fn fix_text(argv: &[String]) -> String {
    let quote = |a: &String| {
        if a.is_empty() || a.chars().any(char::is_whitespace) {
            format!("\"{}\"", a.replace('"', "'"))
        } else {
            a.clone()
        }
    };
    argv.iter().map(quote).collect::<Vec<_>>().join(" ")
}

fn told(l: &Lesson) -> String {
    let times = if l.ups > 1 { format!(", {}x", l.ups) } else { String::new() };
    format!("\n! learned: after this error `lighting {}` worked here before{times}", oneline(&l.fix, 160))
}

pub fn after(d: &Daemon, ctx: &mut Ctx, name: &str, argv: &[String], code: i32, out: &mut String) {
    if !learnable(name) {
        return;
    }
    let (t, here, fix) = (now(), target(ctx), fix_text(argv));
    let offered = ctx.offered.take();
    if code != 0 {
        if let Some(o) = offered.filter(|o| o.fix == fix) {
            let _ = with_store(d, |s| s.vote(&o.sig, false));
        }
        let sig = format!("{name}|{here}|{}", kind(out));
        if let Ok(Some(l)) = with_store(d, |s| s.lesson(&sig))
            && l.fix != fix
        {
            out.push_str(&told(&l));
            ctx.offered = Some(Offered { sig: sig.clone(), fix: l.fix });
        }
        ctx.failed = Some(Failed { sig, fix, target: here, at: t });
        return;
    }
    if let Some(o) = offered.filter(|o| o.fix == fix) {
        let _ = with_store(d, |s| s.vote(&o.sig, true));
        ctx.failed = None;
        return;
    }
    if let Some(f) = ctx.failed.take()
        && t - f.at <= WINDOW_S
        && f.fix != fix
        && f.target == here
    {
        let (cmd, rest) = f.sig.split_once('|').unwrap_or((&f.sig, ""));
        let kind = rest.split_once('|').map_or("", |(_, k)| k);
        let _ = with_store(d, |s| s.learn(&f.sig, cmd, &f.target, kind, &fix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_get_stable_signatures() {
        assert_eq!(
            kind("err: element e12 not found (\"Log in\") -> try: lighting snap\n! STOP: again"),
            "element e# not found (…)"
        );
        assert_eq!(kind("err: browser did not answer 'click' within 20s"), "browser did not answer … within #s");
        assert_eq!(host("https://www.Example.com:8080/a?b"), "Example.com:8080");
        assert_eq!(fix_text(&["click".into(), "Log in".into(), "".into()]), "click \"Log in\" \"\"");
        assert!(learnable("click") && learnable("open") && !learnable("status") && !learnable("recall"));
        assert!(!learnable("tabs"));
    }
}
