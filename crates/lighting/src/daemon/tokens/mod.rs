mod claude;
mod num;
mod transcript;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use lt_core::{Call, Fail, local_now};
use serde_json::Value;

use super::{Daemon, brain, routines};
use num::{commas, date_of, day_no, pyround, short, weekday};

const DAYS: i64 = 30;
const PAGE: i64 = 15_850;
const SCREENSHOT: i64 = 2_350;
const VIDEO: i64 = 14_100;
const IMAGE: &[(&str, i64)] = &[("shot", 700), ("frames", 518)];
const BAR: f64 = 24.0;
const WEEKS: usize = 12;
const DAILY_MAX: usize = 14;
const TOP: usize = 5;

const GROUPS: &[(&str, &[&str])] = &[
    (
        "look at pages and apps",
        &["open", "snap", "back", "forward", "reload", "tab", "search", "shot", "frames", "inbox", "unread"],
    ),
    (
        "clicks and typing",
        &[
            "click", "type", "press", "fill", "select", "check", "scroll", "hover", "drag", "dismiss", "focus",
            "upload", "launch", "wait", "expect", "reply", "dialog",
        ],
    ),
    ("sequences and routines", &["do", "run"]),
    ("page text and data", &["text", "read", "table", "fetch", "js", "net", "console", "listen", "watch", "clip"]),
    ("memory and code", &["recall", "remember", "forget", "todo", "wiki", "memory", "code", "hook", "grep", "squeeze"]),
];
const CLAIMED: &[&str] = &["look at pages and apps", "clicks and typing", "sequences and routines"];
const SKIP: &[&str] = &[
    "selftest",
    "bench",
    "demo",
    "ext-reload",
    "done",
    "ping",
    "status",
    "log",
    "tokens",
    "suggest",
    "version",
    "help",
    "access-prompt",
    "stop",
    "autoload",
];
const OTHER: &str = "other";

fn group_of(cmd: &str) -> &'static str {
    GROUPS.iter().find(|(_, cmds)| cmds.contains(&cmd)).map_or(OTHER, |(name, _)| name)
}

fn without(cmd: &str) -> Option<i64> {
    match cmd {
        "open" => Some(PAGE),
        "frames" => Some(VIDEO),
        _ => CLAIMED.contains(&group_of(cmd)).then_some(SCREENSHOT),
    }
}

fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(routines::truthy)
}

fn int(v: Option<&Value>) -> i64 {
    v.and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0)
}

fn read_rows(home: &Path, since: &str) -> Vec<Value> {
    ["log.old.jsonl", "log.jsonl"]
        .iter()
        .filter_map(|name| fs::read(home.join(name)).ok())
        .flat_map(|raw| {
            String::from_utf8_lossy(&raw)
                .lines()
                .filter(|l| l.get(7..17).unwrap_or("") >= since)
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[derive(Default)]
struct Sum {
    groups: Vec<(&'static str, [i64; 3])>,
    cmds: BTreeMap<String, i64>,
    days: BTreeMap<String, [i64; 2]>,
    trim: i64,
    trim_from: Option<String>,
    runs: i64,
}

fn summarize(rows: &[Value]) -> Sum {
    let mut s =
        Sum { groups: GROUPS.iter().map(|(n, _)| *n).chain([OTHER]).map(|n| (n, [0; 3])).collect(), ..Sum::default() };
    for r in rows {
        let cmd = r["cmd"].as_str().unwrap_or("");
        let t = r["t"].as_str().unwrap_or("");
        if truthy(r.get("x")) || SKIP.contains(&cmd) || t.is_empty() || r["sid"] == lt_proto::WINDOW_SID {
            continue;
        }
        let ok = r.get("ok").is_none_or(routines::truthy);
        let image = IMAGE.iter().find(|(c, _)| *c == cmd).map_or(0, |(_, n)| *n);
        let used = int(r.get("n")).div_euclid(4) + if ok { image } else { 0 };
        let alt = if ok { without(cmd) } else { None }.map_or(used, |b| b.max(used));
        let group = group_of(cmd);
        if let Some((_, g)) = s.groups.iter_mut().find(|(n, _)| *n == group) {
            *g = [g[0] + 1, g[1] + used, g[2] + alt];
        }
        *s.cmds.entry(cmd.to_string()).or_default() += alt - used;
        let day = s.days.entry(t.chars().take(10).collect()).or_default();
        *day = [day[0] + used, day[1] + alt - used];
        s.runs += 1;
        if truthy(r.get("c")) {
            s.trim += int(r.get("c")).div_euclid(4);
            s.trim_from.get_or_insert_with(|| t.chars().take(10).collect());
        }
    }
    s
}

fn buckets(days: &BTreeMap<String, [i64; 2]>, daily: bool) -> Vec<(i64, i64, i64)> {
    let mut out: BTreeMap<i64, [i64; 2]> = BTreeMap::new();
    for (day, [used, saved]) in days {
        let Some(d) = day_no(day) else { continue };
        let key = if daily { d } else { d - weekday(d) };
        let b = out.entry(key).or_default();
        *b = [b[0] + used, b[1] + saved];
    }
    let keep = if daily { DAILY_MAX } else { WEEKS };
    let skip = out.len().saturating_sub(keep);
    out.into_iter().skip(skip).map(|(k, [u, v])| (k, u, v)).collect()
}

fn extras(home: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    let all = routines::load_all(home);
    let field = |r: &Value, k: &str| int(r["stats"].get(k));
    let (runs, saved): (i64, i64) = all.iter().fold((0, 0), |(a, b), r| (a + field(r, "runs"), b + field(r, "saved")));
    if runs > 0 {
        lines.push(format!(
            "routines: {runs} runs, ~{} tokens saved against doing their steps by hand (all time)",
            short(saved as f64)
        ));
    }
    let st = brain::stats(home);
    let pruned = int(st.get("pruned"));
    if pruned > 0 {
        lines.push(format!(
            "brain filter: hid ~{} tokens of controls you never use, on {pruned} snaps (all time)",
            short(int(st.get("chars")).div_euclid(4) as f64)
        ));
    }
    lines
}

fn row(name: &str, runs: &str, used: &str, alt: &str, saved: &str) -> String {
    format!("{name:<24} {runs:>6} {used:>8} {alt:>17} {saved:>8}")
}

fn report(home: &Path, s: &Sum, title: &str, daily: bool) -> String {
    if s.runs == 0 {
        return format!("{title}: no Lighting commands logged");
    }
    let mut out = vec![
        format!("{title}: {} commands", commas(s.runs)),
        String::new(),
        row("", "runs", "used", "without Lighting", "saved"),
    ];
    let mut total = [0i64; 3];
    for (name, [runs, used, alt]) in &s.groups {
        if *runs == 0 {
            continue;
        }
        total = [total[0] + runs, total[1] + used, total[2] + alt];
        let claim = CLAIMED.contains(name) && alt > used;
        let pick = |v: String| if claim { v } else { "-".into() };
        let pct = if claim { format!("  {}%", (100 * (alt - used)).div_euclid(*alt)) } else { String::new() };
        out.push(
            row(
                name,
                &commas(*runs),
                &short(*used as f64),
                &pick(short(*alt as f64)),
                &pick(short((alt - used) as f64)),
            ) + &pct,
        );
    }
    let saved = total[2] - total[1];
    out.push(
        row("total", &commas(total[0]), &short(total[1] as f64), &short(total[2] as f64), &short(saved as f64))
            + &format!("  {}%", (100 * saved).div_euclid(total[2].max(1))),
    );
    let rows = buckets(&s.days, daily);
    let top = rows.iter().map(|r| r.2).chain([1]).max().unwrap_or(1);
    out.push(String::new());
    out.push(format!("{:<10} {:>8} {:>8}", if daily { "day" } else { "week" }, "used", "saved"));
    for (k, u, v) in rows {
        let bars = pyround(BAR * v as f64 / top as f64).max(i64::from(v != 0));
        let bar = "█".repeat(usize::try_from(bars).unwrap_or(0));
        out.push(format!("{:<10} {:>8} {:>8}  {bar}", &date_of(k)[5..], short(u as f64), short(v as f64)));
    }
    let mut best: Vec<(i64, &String)> = s.cmds.iter().filter(|(_, v)| **v > 0).map(|(c, v)| (*v, c)).collect();
    best.sort_by(|a, b| b.cmp(a));
    if !best.is_empty() {
        let shown: Vec<String> = best.iter().take(TOP).map(|(v, c)| format!("{c} {}", short(*v as f64))).collect();
        out.push(String::new());
        out.push(format!("most saved: {}", shown.join(", ")));
    }
    out.extend(extras(home));
    if s.trim > 0 {
        out.push(format!(
            "exact: Lighting cut ~{} tokens from its own answers since {} (long answers to a file, app deltas, brain filter, code outlines, squeeze, grep)",
            short(s.trim as f64),
            s.trim_from.as_deref().and_then(|d| d.get(5..)).unwrap_or("")
        ));
    }
    out.push(String::new());
    out.push("used = what your agent read from Lighting (characters / 4, images as measured).".into());
    out.push(format!(
        "without Lighting = the same steps by a screenshot agent, measured (README, The numbers): ~{} tokens",
        commas(PAGE)
    ));
    out.push(format!(
        "to open a page (screenshot + page text), ~{} per other look or action (one screenshot), ~{} per",
        commas(SCREENSHOT),
        commas(VIDEO)
    ));
    out.push("video. Page text, memory and code count only as used, nothing is claimed for them.".into());
    out.join("\n")
}

pub fn run(d: &Daemon, c: &Call) -> Result<String, Fail> {
    let word = c.pos.first().map(|w| w.to_lowercase()).filter(|w| !w.is_empty()).unwrap_or_else(|| DAYS.to_string());
    let today_s = local_now()[..10].to_string();
    let today = day_no(&today_s).unwrap_or(0);
    let (since, title, daily) = match word.as_str() {
        "claude" => return claude::run(&d.home, &c.pos[1..]),
        "all" => (String::new(), "Lighting tokens, all logged days".to_string(), false),
        "today" => (today_s, "Lighting tokens, today".to_string(), true),
        w if w.bytes().all(|b| b.is_ascii_digit()) && w.parse::<i64>().is_ok_and(|n| n > 0) => {
            let n: i64 = w.parse().unwrap_or(1);
            let first = date_of(today - n + 1);
            let title = format!("Lighting tokens, last {w} days ({} to {})", &first[5..], &date_of(today)[5..]);
            (first, title, n <= DAILY_MAX as i64)
        }
        _ => {
            return Err(Fail::hint(
                "which days?",
                "lighting tokens | lighting tokens 7 | lighting tokens today | lighting tokens all | lighting tokens claude",
            ));
        }
    };
    Ok(report(&d.home, &summarize(&read_rows(&d.home, &since)), &title, daily))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lighting_tokens_are_summed_like_2x() {
        let rows = [
            json!({"t": "2026-10-28 10:00:00", "cmd": "open", "ok": true, "n": 4000}),
            json!({"t": "2026-10-28 10:00:01", "cmd": "click", "ok": true, "n": 40, "c": 400}),
            json!({"t": "2026-10-29 10:00:00", "cmd": "shot", "ok": true, "n": 8}),
            json!({"t": "2026-10-29 10:00:00", "cmd": "click", "ok": false, "n": 80}),
            json!({"t": "2026-10-29 10:00:00", "cmd": "recall", "ok": true, "n": 400}),
            json!({"t": "2026-10-29 10:00:00", "cmd": "status", "ok": true, "n": 400}),
            json!({"t": "2026-10-30 10:00:00", "cmd": "snap", "ok": true, "n": 40, "x": 1}),
        ];
        let s = summarize(&rows);
        assert_eq!(s.runs, 5);
        assert_eq!(s.groups[0], ("look at pages and apps", [2, 1000 + 702, PAGE + SCREENSHOT]));
        assert_eq!(s.groups[1], ("clicks and typing", [2, 10 + 20, SCREENSHOT + 20]));
        assert_eq!(s.groups[4], ("memory and code", [1, 100, 100]));
        assert_eq!((s.trim, s.trim_from.as_deref()), (100, Some("2026-10-28")));
        assert_eq!(s.days["2026-10-28"], [1010, PAGE - 1000 + SCREENSHOT - 10]);
        let weeks = buckets(&s.days, false);
        assert_eq!(weeks.len(), 1);
        assert_eq!(date_of(weeks[0].0), "2026-10-26");
    }

    #[test]
    fn window_calls_are_not_model_tokens() {
        let rows = [
            json!({"t": "2026-10-31 01:05:02", "cmd": "board", "ok": true, "n": 42240, "sid": lt_proto::WINDOW_SID}),
            json!({"t": "2026-10-31 01:05:03", "cmd": "todo", "ok": true, "n": 400, "sid": lt_proto::WINDOW_SID}),
            json!({"t": "2026-10-31 01:05:04", "cmd": "todo", "ok": true, "n": 400, "sid": "a-session"}),
        ];
        let s = summarize(&rows);
        assert_eq!(s.runs, 1);
        assert_eq!(s.groups[4], ("memory and code", [1, 100, 100]));
        assert_eq!(s.groups[5], (OTHER, [0, 0, 0]));
    }
}
