use std::env;
use std::fs;
use std::path::Path;

use lt_core::{Fail, local_now, stamp_secs};
use serde_json::Value;

use super::num::{commas, day_no, pyround, short};
use super::transcript::{Req, claude_home, load};

const DAYS: i64 = 30;
const W_READ: f64 = 0.1;
const W_WRITE: f64 = 2.0;
const W_OUT: f64 = 5.0;
const COMPACT_AT: i64 = 400_000;
const SUMMARY: i64 = 50_000;
const REBUILD: i64 = 50_000;
const BREAK_S: i64 = 3600;
const START_BIG: f64 = 30_000.0;
const START_SESSIONS: usize = 20;
const TIP_PCT: f64 = 10.0;
const MAX_RATIO: f64 = 1.5;
const PARTS: [(&str, &str); 6] = [
    ("thinking", "earlier thinking"),
    ("start", "start context"),
    ("bash", "Bash output"),
    ("read", "file reads"),
    ("said", "Claude's text and tool calls"),
    ("summary", "summaries after compact"),
];
const THINKING: usize = 0;
const START: usize = 1;
const BASH: usize = 2;
const READ: usize = 3;
const SAID: usize = 4;
const SUMMARY_AT: usize = 5;

fn compacted(sessions: &[Vec<Req>], window: i64) -> (i64, i64) {
    let (mut total, mut count) = (0, 0);
    for reqs in sessions {
        let (mut offset, mut prev): (i64, Option<i64>) = (0, None);
        for r in reqs {
            if prev.is_some_and(|p| (r.ctx as f64) < p as f64 * 0.5) {
                offset = 0;
            }
            let mut sim = r.ctx - offset;
            if sim > window {
                total += sim;
                count += 1;
                offset += sim - SUMMARY;
                sim = SUMMARY;
            }
            total += sim;
            prev = Some(r.ctx);
        }
    }
    (total, count)
}

fn window_set() -> Option<i64> {
    let val = env::var("CLAUDE_CODE_AUTO_COMPACT_WINDOW").ok().filter(|v| !v.is_empty()).or_else(|| {
        let raw = fs::read_to_string(claude_home().join("settings.json")).ok()?;
        let v: Value = serde_json::from_str(&raw).ok()?;
        match &v["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"] {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    })?;
    val.bytes().all(|b| b.is_ascii_digit()).then(|| val.parse().ok()).flatten()
}

#[derive(Default)]
struct Sum {
    sessions: usize,
    requests: i64,
    ctx: i64,
    write: i64,
    out: i64,
    think: i64,
    effort: Vec<(String, [i64; 2])>,
    rebuilds: i64,
    breaks: i64,
    rebuilt: i64,
    ctxs: Vec<i64>,
    starts: Vec<(String, i64, i64)>,
    parts: [f64; 6],
    held: Vec<(String, f64)>,
}

fn add<K: PartialEq + Clone, V: Default + Copy>(list: &mut Vec<(K, V)>, key: &K, f: impl FnOnce(&mut V)) {
    let at = list.iter().position(|(k, _)| k == key).unwrap_or_else(|| {
        list.push((key.clone(), V::default()));
        list.len() - 1
    });
    f(&mut list[at].1);
}

fn summarize(sessions: &[Vec<Req>]) -> Sum {
    let mut s = Sum { sessions: sessions.len(), ..Sum::default() };
    for reqs in sessions {
        let base = reqs.iter().map(|r| r.ctx).min().unwrap_or(0);
        s.starts.push((reqs[0].t.clone(), base, i64::try_from(reqs.len()).unwrap_or(0)));
        let (mut prev, mut run, mut held): (Option<&Req>, [f64; 6], Vec<(String, f64)>) = (None, [0.0; 6], Vec::new());
        for r in reqs {
            let think = (r.out - r.vis.div_euclid(4)).max(0);
            match prev {
                Some(p) if (r.ctx as f64) >= p.ctx as f64 * 0.5 => {
                    let gone = (p.out - p.vis.div_euclid(4)).max(0);
                    let added = (r.ctx - p.ctx - p.out).max(0) as f64;
                    let chars = r.got.iter().sum::<i64>();
                    let effort = if p.effort.is_empty() { "?".to_string() } else { p.effort.clone() };
                    add(&mut held, &effort, |v| *v += gone as f64);
                    run[SAID] += (p.out - gone) as f64;
                    if chars > 0 {
                        run[BASH] += added * r.got[0] as f64 / chars as f64;
                        run[READ] += added * r.got[1] as f64 / chars as f64;
                    }
                }
                _ => {
                    run = [0.0; 6];
                    held.clear();
                    run[START] = base as f64;
                    run[SUMMARY_AT] = (r.ctx - base) as f64;
                }
            }
            for (k, v) in run.iter().enumerate() {
                s.parts[k] += v;
            }
            for (k, v) in &held {
                add(&mut s.held, k, |h| *h += v);
            }
            s.requests += 1;
            s.ctx += r.ctx;
            s.write += r.write;
            s.out += r.out;
            s.think += think;
            s.ctxs.push(r.ctx);
            let effort = if r.effort.is_empty() { "?".to_string() } else { r.effort.clone() };
            add(&mut s.effort, &effort, |e| *e = [e[0] + 1, e[1] + think]);
            if let Some(p) = prev.filter(|p| r.write > REBUILD && (r.ctx as f64) >= p.ctx as f64 * 0.5) {
                s.rebuilds += 1;
                let gap = stamp_secs(&r.t.replace('T', " "))
                    .zip(stamp_secs(&p.t.replace('T', " ")))
                    .map_or(0, |(a, b)| a - b);
                s.breaks += i64::from(gap > BREAK_S);
                s.rebuilt += r.write;
            }
            prev = Some(r);
        }
    }
    s.parts[THINKING] = s.held.iter().map(|(_, v)| v).sum();
    s
}

fn held_of(s: &Sum, k: &str) -> f64 {
    s.held.iter().find(|(e, _)| e == k).map_or(0.0, |(_, v)| *v)
}

fn median(mut v: Vec<i64>) -> f64 {
    v.sort_unstable();
    let n = v.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        v[n / 2] as f64
    } else {
        (v[n / 2 - 1] + v[n / 2]) as f64 / 2.0
    }
}

fn tips(s: &Sum, sessions: &[Vec<Req>], weighted: f64) -> Vec<String> {
    let mut out: Vec<(f64, String)> = Vec::new();
    let ctx = s.ctx.max(1) as f64;
    let window = window_set();
    let (sim, count) = compacted(sessions, COMPACT_AT);
    let cut = 100.0 * (s.ctx - sim) as f64 / ctx;
    if cut >= TIP_PCT {
        let done = match window.filter(|w| *w <= COMPACT_AT && *w > 0) {
            Some(w) => format!(" (you set {}; older sessions in this window ran without it)", short(w as f64)),
            None => String::new(),
        };
        out.push((cut, format!(
            "auto-compact at {}: ~{}% fewer context tokens, +{count} compactions. Set CLAUDE_CODE_AUTO_COMPACT_WINDOW={COMPACT_AT} in ~/.claude/settings.json env{done}",
            short(COMPACT_AT as f64), cut as i64
        )));
    }
    let find = |k: &str| s.effort.iter().find(|(e, _)| e == k).map(|(_, v)| *v);
    if let (Some(mx), Some(hi)) = (find("max"), find("high"))
        && hi[1] != 0
    {
        let share = 100.0 * mx[0] as f64 / s.requests as f64;
        let ratio = (mx[1] as f64 / mx[0] as f64) / (hi[1] as f64 / hi[0] as f64);
        if share >= TIP_PCT && ratio >= MAX_RATIO {
            out.push((share, format!(
                "effort: max ran {}% of requests and thinks {ratio:.1}x as much as high; its thinking fills ~{}% of all context. /effort high for routine work, max for hard problems",
                share as i64, pyround(100.0 * held_of(s, "max") / s.ctx as f64)
            )));
        }
    }
    let fixed: i64 = s.starts.iter().map(|(_, c, n)| c * n).sum();
    let start = fixed as f64 / s.requests.max(1) as f64;
    if start > START_BIG {
        let mut sorted = s.starts.clone();
        sorted.sort();
        let last: Vec<i64> = sorted.iter().rev().take(START_SESSIONS).map(|(_, c, _)| *c).collect();
        let latest = median(last);
        let part = 100.0 * fixed as f64 / ctx;
        out.push((part, format!(
            "start context ~{} on every request ({}% of all context, last {START_SESSIONS} sessions ~{}): /context shows it; deny tools you never use, skillOverrides \"name-only\" for rare skills, turn off unused MCP servers",
            short(start.trunc()), part as i64, short(latest.trunc())
        )));
    }
    let share = |k: usize| 100.0 * s.parts[k] / ctx;
    if share(BASH) + share(READ) >= TIP_PCT {
        out.push((share(BASH) + share(READ), format!(
            "tool output: Bash ~{}% and file reads ~{}% of all context, kept until the next compact. Outline a file first and read one function or a line range (wisply code outline <file>, wisply code body <Name>, sed -n 40,90p), search with wisply grep, run long commands through wisply squeeze -- <cmd>",
            pyround(share(BASH)), pyround(share(READ))
        )));
    }
    let rebuilt = 100.0 * s.rebuilt as f64 * W_WRITE / weighted.max(1.0);
    if rebuilt >= TIP_PCT / 2.0 {
        out.push((rebuilt, format!(
            "cache rebuilds: {} full rewrites ({} after a break over {} min), {}% of the weighted total. /compact before long breaks",
            s.rebuilds, s.breaks, BREAK_S / 60, rebuilt as i64
        )));
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    out.into_iter().map(|(_, t)| t).collect()
}

fn report(sessions: &[Vec<Req>], days: i64) -> String {
    let mut s = summarize(sessions);
    let title = format!("Claude Code tokens, last {days} days");
    if s.requests == 0 {
        return format!("{title}: no sessions found in {}", claude_home().join("projects").display());
    }
    let read = (s.ctx - s.write) as f64;
    let weighted = read * W_READ + s.write as f64 * W_WRITE + s.out as f64 * W_OUT;
    let pct = |part: f64| (100.0 * part / weighted.max(1.0)) as i64;
    s.ctxs.sort_unstable();
    let ctx = s.ctx as f64;
    let mut lines = vec![
        format!("{title}: {} requests in {} sessions (subagents not counted)", commas(s.requests), s.sessions),
        format!(
            "context read: {} tokens, ~{} per request (half of all requests above {})",
            short(ctx),
            short(s.ctx.div_euclid(s.requests) as f64),
            short(s.ctxs[s.ctxs.len() / 2] as f64)
        ),
        format!(
            "weighted like API prices: cache reads {}%, cache writes {}%, output {}%",
            pct(read * W_READ),
            pct(s.write as f64 * W_WRITE),
            pct(s.out as f64 * W_OUT)
        ),
        format!(
            "output {} tokens: ~{}% thinking. Claude Code keeps thinking in the context, so later requests read it again",
            short(s.out as f64),
            (100 * s.think).div_euclid(s.out.max(1))
        ),
    ];
    let part = |n: f64| pyround(100.0 * n / ctx);
    let mut order: Vec<usize> = (0..PARTS.len()).collect();
    order.sort_by(|a, b| s.parts[*b].total_cmp(&s.parts[*a]));
    let mut shown: Vec<String> = order
        .iter()
        .filter(|k| part(s.parts[**k]) != 0)
        .map(|k| format!("{} ~{}%", PARTS[*k].1, part(s.parts[*k])))
        .collect();
    shown.push(format!("rest ~{}%", part((ctx - s.parts.iter().sum::<f64>()).max(0.0))));
    lines.push(format!("context by content: {}", shown.join(", ")));
    let mut known: Vec<&(String, [i64; 2])> = s.effort.iter().filter(|(k, _)| k != "?").collect();
    known.sort_by_key(|(_, v)| -v[0]);
    if !known.is_empty() {
        lines.push(String::new());
        lines.push(format!("{:<8} {:>9} {:>22}", "effort", "requests", "thinking per request"));
        for (k, [n, t]) in known {
            lines.push(format!("{k:<8} {:>9} {:>22}", commas(*n), format!("~{}", commas(t.div_euclid(*n)))));
        }
    }
    let found = tips(&s, sessions, weighted);
    lines.push(String::new());
    if found.is_empty() {
        lines.push("no big fix found".into());
    } else {
        lines.push("fixes, biggest first:".into());
        lines.extend(found.into_iter().map(|t| format!("- {t}")));
    }
    lines.push(String::new());
    lines.push(format!(
        "read from your local Claude Code transcripts; weights: cache read {W_READ:.1}x, cache write {W_WRITE:.0}x (1 h cache), output {W_OUT:.0}x the input price. Thinking = output minus visible text and tool calls (characters / 4). Content = how much each request grew, split by the characters that came in since the one before."
    ));
    lines.join("\n")
}

pub fn run(home: &Path, pos: &[String]) -> Result<String, Fail> {
    let word = pos.first().cloned().unwrap_or_else(|| DAYS.to_string());
    let days = Some(&word)
        .filter(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|w| w.parse::<i64>().ok())
        .filter(|d| *d >= 1)
        .ok_or_else(|| Fail::hint("which days?", "wisply tokens claude | wisply tokens claude 7"))?;
    let today = day_no(&local_now()).unwrap_or(0);
    Ok(report(&load(home, today - days + 1), days))
}

#[cfg(test)]
mod tests {
    use super::super::transcript::{parse, py_len};
    use super::*;
    use serde_json::json;

    #[test]
    fn transcripts_are_read_like_2x() {
        let lines = [
            r#"{"type":"api-request","effort":"high"}"#,
            r#"{"type":"user","message":{"content":"hallo du"}}"#,
            r#"{"type":"assistant","timestamp":"2026-10-30T10:00:00.000Z","message":{"id":"m1","content":[{"type":"text","text":"ok"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls ä"}}],"usage":{"input_tokens":10,"cache_creation_input_tokens":60000,"cache_read_input_tokens":0,"output_tokens":40}}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"abcdef"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"id":"m9","usage":{"input_tokens":5}}}"#,
            r#"{"type":"assistant","timestamp":"2026-10-30T10:00:05.000Z","message":{"id":"m2","content":[{"type":"text","text":"done"}],"usage":{"input_tokens":1,"cache_creation_input_tokens":100,"cache_read_input_tokens":60010,"output_tokens":3}}}"#,
        ];
        let reqs = parse(&lines.join("\n"));
        assert_eq!(reqs.len(), 2);
        assert_eq!(
            (reqs[0].ctx, reqs[0].write, reqs[0].out, reqs[0].effort.as_str(), reqs[0].got),
            (60010, 60000, 40, "high", [0, 0, 8])
        );
        assert_eq!(reqs[0].vis, 2 + py_len(&json!({"command": "ls ä"})));
        assert_eq!(py_len(&json!({"command": "ls ä"})), 24);
        assert_eq!((reqs[1].ctx, reqs[1].got, reqs[1].t.as_str()), (60111, [6, 0, 0], "2026-10-30T10:00:05"));
        let s = summarize(std::slice::from_ref(&reqs));
        assert_eq!((s.requests, s.ctx, s.think, s.rebuilds), (2, 120_121, 40 - 6 + 2, 0));
        assert!((s.parts[BASH] - 61.0).abs() < 1e-9 && (s.parts[START] - 120_020.0).abs() < 1e-9);
        let back = Req::from(&reqs[1].json()).expect("row");
        assert_eq!((back.t, back.ctx, back.got), (reqs[1].t.clone(), 60111, [6, 0, 0]));
    }
}
