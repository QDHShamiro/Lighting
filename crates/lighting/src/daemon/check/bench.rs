use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lt_core::{Call, Fail};
use serde_json::{Map, Value, json};

use super::super::ctx::Ctx;
use super::super::{Daemon, Out, Pending};
use super::serve::{Server, Site};
use super::{argv, once};

const REAL_SITES: &[&str] = &[
    "github.com/QDHShamiro/Lighting",
    "en.wikipedia.org/wiki/Minecraft",
    "www.youtube.com",
    "modrinth.com/plugins",
    "huggingface.co/models",
    "www.tiktok.com/foryou",
];
const RAW_JS: &str = "document.body.innerText.length";
const MCP_CALLS: usize = 20;
const MCP_WAIT: Duration = Duration::from_secs(60);
const MCP_POLL: Duration = Duration::from_millis(200);
const MAX_RUNS: usize = 9;
const MIN_RAW: usize = 50;

type Probe = Result<(u128, f64, f64), String>;

fn tokens(text: &str) -> usize {
    text.chars().count() / 4 + 1
}

fn timed(d: &Daemon, ctx: &mut Ctx, words: &[&str]) -> (u128, String) {
    let t = Instant::now();
    let (out, _) = once(d, ctx, &argv(words));
    (t.elapsed().as_millis(), out)
}

fn median<T: Copy + Ord>(mut v: Vec<T>) -> T {
    v.sort_unstable();
    v[v.len() / 2]
}

fn raw_tokens(d: &Daemon, ctx: &mut Ctx) -> usize {
    let (out, err) = once(d, ctx, &argv(&["js", RAW_JS]));
    if err { 0 } else { out.trim().parse::<usize>().map_or(0, |n| n / 4 + 1) }
}

fn plan_rows(d: &Daemon, ctx: &mut Ctx) -> Result<Vec<String>, Fail> {
    let srv = Server::start(Site::Fixture).map_err(|e| Fail::new(format!("test server: {e}")))?;
    let base = srv.base.as_str();
    let plan: [&[&str]; 9] = [
        &["open", base, "--new"],
        &["snap", "--force"],
        &["snap", "-f", "email"],
        &["click", "Open modal"],
        &["click", "Close modal"],
        &["text"],
        &["read", base],
        &["tabs"],
        &["close"],
    ];
    let mut rows = vec!["command                     ms   chars  ~tokens".to_string()];
    for words in plan {
        let (ms, out) = timed(d, ctx, words);
        let shown: String = words.join(" ").replace(base, "<fixture>").chars().take(26).collect();
        rows.push(format!("{shown:<26} {ms:>5} {:>6} {:>7}", out.chars().count(), tokens(&out)));
    }
    Ok(rows)
}

fn mcp_probe(home: &std::path::Path) -> Probe {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg("mcp").env("LIGHTING_HOME", home).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    crate::v2::hide(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let (Some(mut input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("no pipes to lighting mcp".into());
    };
    let mut lines = BufReader::new(output).lines();
    let mut ask = |id: usize, method: &str, params: Value| -> Result<String, String> {
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(input, "{msg}").and_then(|()| input.flush()).map_err(|e| e.to_string())?;
        lines.next().ok_or("lighting mcp closed")?.map_err(|e| e.to_string())
    };
    let t0 = Instant::now();
    let info = json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "bench", "version": super::super::VERSION}});
    ask(1, "initialize", info)?;
    let start = t0.elapsed().as_millis();
    let mut times = Vec::with_capacity(MCP_CALLS);
    for i in 0..MCP_CALLS {
        let t = Instant::now();
        let answer = ask(10 + i, "tools/call", json!({"name": "lighting", "arguments": {"cmd": "ping"}}))?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        if !answer.contains("pong") {
            return Err(format!("unexpected answer {}", answer.chars().take(80).collect::<String>()));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    times.sort_by(f64::total_cmp);
    Ok((start, times[times.len() / 2], times[times.len() - 1]))
}

fn with_mcp(d: &Daemon, rows: Vec<String>) -> Pending {
    let box_: Arc<Mutex<Option<Probe>>> = Arc::new(Mutex::new(None));
    let (slot, home) = (Arc::clone(&box_), d.home.clone());
    thread::spawn(move || {
        let got = mcp_probe(&home);
        *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(got);
    });
    Pending::new(MCP_WAIT, MCP_POLL, move |_, _, last| {
        let got = box_.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let line = match got {
            Some(Ok((start, med, max))) => format!(
                "mcp ping through lighting mcp: start {start} ms, then median {med:.1} ms, max {max:.1} ms ({MCP_CALLS} calls)"
            ),
            Some(Err(e)) => format!("mcp: {e}"),
            None if last => "mcp: no answer within 60 s".into(),
            None => return Ok(None),
        };
        Ok(Some(format!("{}\n{line}", rows.join("\n"))))
    })
}

fn real(d: &Daemon, ctx: &mut Ctx, c: &Call) -> String {
    let named: Vec<&str> = c.pos.iter().map(String::as_str).filter(|p| *p != "real").collect();
    let sites = if named.is_empty() { REAL_SITES.to_vec() } else { named };
    let path = d.home.join("bench-real.json");
    let before: Map<String, Value> =
        fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let runs = c.val("runs").and_then(|r| r.parse::<usize>().ok()).unwrap_or(3).clamp(1, MAX_RUNS);
    let suffix = if runs > 1 { format!("   (median of {runs} runs)") } else { String::new() };
    let mut rows = vec![format!(
        "site                          open ms  tok   snap ms  tok   text ms   tok  raw page tok  saved  vs last{suffix}"
    )];
    let (mut now, mut total) = (Map::new(), (0usize, 0usize));
    for (i, site) in sites.iter().enumerate() {
        let mut got: [Vec<(u128, usize)>; 3] = Default::default();
        for n in 0..runs {
            let open: Vec<&str> = if i == 0 && n == 0 { vec!["open", site, "--new"] } else { vec!["open", site] };
            for (k, words) in [open, vec!["snap", "--force"], vec!["text"]].iter().enumerate() {
                let (ms, out) = timed(d, ctx, words);
                got[k].push((ms, tokens(&out)));
            }
        }
        let res: Vec<(u128, usize)> = got
            .into_iter()
            .map(|v| (median(v.iter().map(|x| x.0).collect()), median(v.iter().map(|x| x.1).collect())))
            .collect();
        let raw = raw_tokens(d, ctx);
        now.insert(
            (*site).to_string(),
            json!({"open": [res[0].0, res[0].1], "snap": [res[1].0, res[1].1], "text": [res[2].0, res[2].1], "raw": [0, raw]}),
        );
        if raw >= MIN_RAW {
            total.0 += res[2].1;
            total.1 += raw;
        }
        let delta = before.get(*site).map_or(String::new(), |old| {
            let a: u64 = ["open", "snap", "text"].iter().filter_map(|k| old[*k][1].as_u64()).sum();
            let b: usize = res.iter().map(|x| x.1).sum();
            format!("{:+}% tok", ((b as f64 - a as f64) * 100.0 / a.max(1) as f64).round())
        });
        let saved = if raw >= MIN_RAW {
            format!("{}%", (100.0 - res[2].1 as f64 * 100.0 / raw as f64).round())
        } else {
            "-".into()
        };
        let name: String = site.chars().take(29).collect();
        rows.push(format!(
            "{name:<29} {:>6} {:>5} {:>8} {:>5} {:>8} {:>6} {raw:>13} {saved:>6}  {delta}",
            res[0].0, res[0].1, res[1].0, res[1].1, res[2].0, res[2].1
        ));
    }
    if total.1 > 0 {
        rows.push(format!(
            "total: text {} tokens vs raw page text {} tokens = {}% saved",
            total.0,
            total.1,
            (100.0 - total.0 as f64 * 100.0 / total.1 as f64).round()
        ));
    }
    once(d, ctx, &argv(&["close"]));
    let _ = fs::write(&path, serde_json::to_string_pretty(&Value::Object(now)).unwrap_or_default());
    rows.join("\n")
}

pub fn bench(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    if c.on("real") || c.pos.first().is_some_and(|p| p == "real") {
        return Ok(Out::Text(real(d, ctx, c)));
    }
    if !super::super::web::any_browser(d) {
        return Err(Fail::hint("bench needs the browser", "lighting setup"));
    }
    let rows = plan_rows(d, ctx)?;
    if !ctx.run_stack.is_empty() || c.on("no-mcp") {
        return Ok(Out::Text(rows.join("\n")));
    }
    Ok(Out::Wait(with_mcp(d, rows)))
}
