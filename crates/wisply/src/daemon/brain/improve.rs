use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};
use wisply_core::{Call, Fail, local_parts, stamp_secs};

use super::super::ctx::now;
use super::super::{Daemon, Out};
use super::with;

const DAYS: u64 = 7;
const DAY_S: f64 = 86_400.0;
const BURST_S: i64 = 600;
const BURST_SAID: usize = 10;
const SHOWN: usize = 5;
const ERROR_CHARS: usize = 70;
const TIP_CHARS: usize = 60;
const CMDS_SHOWN: usize = 4;
const EVERY_S: f64 = 7.0 * DAY_S;
const MIN_FAILS: usize = 10;
const ADVICE: &[(&str, &str)] = &[
    (r"no tab yet|has no tab", "open a page first (wisply open <url>); a script must stop when its tab is gone"),
    (r"is gone|older snapshot", "the page changed under the ref: snap again (the brain now retries by name)"),
    (
        r"nothing matches|nothing called|no field called|no field for",
        "the name was guessed: look first with snap -f <word> (the brain learns real names)",
    ),
    (r"mouse or keyboard", "the user was busy; Wisply waits up to 10 s for a pause now"),
    (r"no window chosen", "name the app: wisply snap app:<name> (no windows call needed)"),
    (r"unknown command", "check the name in wisply help"),
    (r"did not answer|disconnected|not connected", "the browser was busy or restarted: wisply status"),
    (
        r"not there after|still there after|timed out|timeout after",
        "the page was slower than the wait: raise --timeout or wait for other text",
    ),
    (r"matches several", "use a longer name or --first"),
    (
        r"runs as administrator",
        "an elevated window takes no clicks or keys from normal apps: wisply read / shot, or an admin terminal",
    ),
    (r"needs --yes|irreversible", "a risky step: only after the user agreed, then --yes"),
    (
        r"no frame matching",
        "the frame was not loaded yet or its name was cut: Wisply waits 3 s and lists the frames now",
    ),
    (r"needs |need a ", "the command was called without what it needs: see its line in wisply help"),
];

static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""[^"]*""#).expect("quoted"));
static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:f\d+\.)?[edwtfo]\d+\b").expect("ref"));
static NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").expect("num"));
static RULES: LazyLock<Vec<(Regex, &str)>> =
    LazyLock::new(|| ADVICE.iter().map(|(p, a)| (Regex::new(p).expect("advice"), *a)).collect());

struct Run {
    ts: i64,
    cmd: String,
    ok: bool,
    err: String,
}

pub struct Pattern {
    pub n: usize,
    pub error: String,
    cmds: Vec<String>,
    burst: usize,
    advice: &'static str,
}

pub struct Summary {
    pub runs: usize,
    pub failed: usize,
    prev_runs: usize,
    prev_failed: usize,
    pub patterns: Vec<Pattern>,
}

fn stamp(epoch: f64) -> String {
    let [y, mo, d, h, mi, s] = local_parts(epoch);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

fn read_log(home: &Path, since: &str) -> Vec<Run> {
    let mut rows = Vec::new();
    for name in ["log.old.jsonl", "log.jsonl"] {
        let Ok(bytes) = fs::read(home.join(name)) else { continue };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let Ok(r) = serde_json::from_str::<Value>(line) else { continue };
            let Some(t) = r["t"].as_str().filter(|t| *t >= since) else { continue };
            let Some(ts) = stamp_secs(t) else { continue };
            if r["x"].is_null() || r["x"] == false || r["x"] == 0 {
                rows.push(Run {
                    ts,
                    cmd: r["cmd"].as_str().unwrap_or("").to_string(),
                    ok: r["ok"] != false,
                    err: r["e"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }
    rows
}

fn shape(err: &str) -> String {
    let s = QUOTED.replace_all(err, "\"x\"");
    let s = REF.replace_all(&s, "ref");
    NUM.replace_all(&s, "n").trim().to_string()
}

pub fn summary(home: &Path, days: u64, at: f64) -> Summary {
    let span = days as f64 * DAY_S;
    let (t0, t1) = (stamp(at - span), stamp(at - 2.0 * span));
    let (s0, s1) = (stamp_secs(&t0).unwrap_or(0), stamp_secs(&t1).unwrap_or(0));
    let rows = read_log(home, &t1);
    let cur: Vec<&Run> = rows.iter().filter(|r| r.ts >= s0).collect();
    let prev: Vec<&Run> = rows.iter().filter(|r| r.ts >= s1 && r.ts < s0).collect();
    let mut groups: Vec<(String, Vec<&Run>)> = Vec::new();
    for r in cur.iter().filter(|r| !r.ok) {
        let k = shape(&r.err);
        match groups.iter_mut().find(|(g, _)| *g == k) {
            Some((_, rs)) => rs.push(r),
            None => groups.push((k, vec![r])),
        }
    }
    groups.sort_by_key(|(_, rs)| std::cmp::Reverse(rs.len()));
    let patterns = groups
        .into_iter()
        .map(|(k, rs)| {
            let mut ts: Vec<i64> = rs.iter().map(|r| r.ts).collect();
            ts.sort_unstable();
            let burst =
                (0..ts.len()).map(|i| ts[i..].iter().filter(|t| **t - ts[i] <= BURST_S).count()).max().unwrap_or(0);
            let advice =
                RULES.iter().find(|(p, _)| p.is_match(&k)).map_or("look at wisply log for the details", |(_, a)| a);
            let cmds: BTreeSet<String> = rs.iter().map(|r| r.cmd.clone()).collect();
            Pattern { n: rs.len(), error: k, cmds: cmds.into_iter().collect(), burst, advice }
        })
        .collect();
    let fails = |rs: &[&Run]| rs.iter().filter(|r| !r.ok).count();
    Summary { runs: cur.len(), failed: fails(&cur), prev_runs: prev.len(), prev_failed: fails(&prev), patterns }
}

pub fn pct(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 }
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

pub fn improve(d: &Daemon, c: &Call) -> Result<Out, Fail> {
    let from_pos = c.pos.first().filter(|p| !p.is_empty() && p.chars().all(|ch| ch.is_ascii_digit()));
    let days = c.val("days").or(from_pos.map(String::as_str)).map_or(Ok(DAYS), |v| {
        v.parse::<u64>().map_err(|_| Fail::hint("--days needs a number", "wisply improve --days 7"))
    })?;
    let s = summary(&d.home, days, now());
    if s.runs == 0 {
        return Ok(Out::Text(format!("no commands in the last {days} days")));
    }
    let before = if s.prev_runs > 0 {
        format!(" (the {days} days before: {:.1}%)", pct(s.prev_failed, s.prev_runs))
    } else {
        String::new()
    };
    let mut lines = vec![format!(
        "Wisply self-check, last {days} days: {} commands, {:.1}% failed{before}",
        s.runs,
        pct(s.failed, s.runs)
    )];
    for (i, p) in s.patterns.iter().take(SHOWN).enumerate() {
        let cmds: Vec<&str> = p.cmds.iter().take(CMDS_SHOWN).map(String::as_str).collect();
        lines.push(format!("{}. {}x {} ({}) -> {}", i + 1, p.n, cut(&p.error, ERROR_CHARS), cmds.join(", "), p.advice));
        if p.burst >= BURST_SAID {
            lines.push(format!("   {} of them within 10 min: a loop kept going after the error", p.burst));
        }
    }
    lines.push(with(&d.home, |st| {
        let data = &st.data;
        let known = data["pages"].as_object().map_or(0, |p| p.keys().filter(|k| k.contains('/') || k.starts_with("app:")).count());
        let fixes: usize = data["fixes"].as_object().into_iter().flatten().map(|(_, f)| f.as_object().map_or(0, |f| f.len())).sum();
        let n = |k: &str| data["stats"][k].as_u64().unwrap_or(0);
        format!(
            "brain: {known} pages known, {fixes} fixes learned, {} snapshots trimmed (-{} tokens), {} refs regained, {} fixes used",
            n("pruned"),
            n("chars") / 4,
            n("regained"),
            n("fixed")
        )
    }));
    Ok(Out::Text(lines.join("\n")))
}

pub fn weekly_tip(home: &Path) -> String {
    let at = now();
    let path = home.join("improve.json");
    let mut st: Value = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
    if !st.is_object() {
        st = json!({});
    }
    if at - st["shown"].as_f64().unwrap_or(0.0) < EVERY_S {
        return String::new();
    }
    st["shown"] = json!(at);
    let _ = fs::write(&path, st.to_string());
    let s = summary(home, DAYS, at);
    let Some(p) = s.patterns.first().filter(|_| s.failed >= MIN_FAILS) else { return String::new() };
    format!(
        "Wisply self-check (weekly): {} commands in 7 days, {:.0}% failed.\nMost common: {}x {} -> {}\nDetails: wisply improve",
        s.runs,
        pct(s.failed, s.runs),
        p.n,
        cut(&p.error, TIP_CHARS),
        p.advice
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_grouped_by_shape_with_advice() {
        assert_eq!(shape("nothing matches \"Save 2\" on e12 after 300 ms"), "nothing matches \"x\" on ref after n ms");
        let home = std::env::temp_dir().join(format!("lt-improve-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let t = stamp(now() - 60.0);
        let row = |ok: bool, e: &str| json!({"t": t, "cmd": "click", "ok": ok, "e": e}).to_string();
        let rows = [row(false, "nothing matches \"A\""), row(false, "nothing matches \"B\""), row(true, "")];
        fs::write(home.join("log.jsonl"), rows.join("\n")).unwrap();
        let s = summary(&home, 7, now());
        assert_eq!((s.runs, s.failed, s.patterns.len(), s.patterns[0].n), (3, 2, 1, 2));
        assert!(s.patterns[0].advice.starts_with("the name was guessed"));
        let _ = fs::remove_dir_all(&home);
        let advice = |e: &str| RULES.iter().find(|(p, _)| p.is_match(e)).map_or("", |(_, a)| *a);
        assert!(advice("no field for \"Email\"").starts_with("the name was guessed"));
        assert!(advice("wait: timeout after 5000 ms").starts_with("the page was slower"));
        assert!(advice("this window runs as administrator; Windows blocks control").starts_with("an elevated window"));
    }
}
