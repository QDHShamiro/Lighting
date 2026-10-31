use std::thread;
use std::time::{Duration, Instant};

use wisply_core::{Call, Fail};

use super::super::Daemon;
use super::super::ctx::Ctx;
use super::serve::{Server, Site};
use super::{argv, once};

const PLACE: &str = "<demo page>";
const PAUSE_S: f64 = 3.5;
const RAW_JS: &str = "document.body.innerText.length";

fn steps(base: &str) -> Vec<Vec<String>> {
    let plan: [&[&str]; 7] = [
        &["open", base, "--new"],
        &["snap", "-f", "name|email|plan|terms|trial"],
        &["fill", "Name=Ada Lovelace", "Email=ada@example.com"],
        &["select", "Plan", "Pro"],
        &["check", "I accept the terms"],
        &["click", "Start free trial", "-f", "Welcome"],
        &["table", "table"],
    ];
    plan.iter().map(|words| argv(words)).collect()
}

fn shown(words: &[String], base: &str) -> String {
    let parts: Vec<String> =
        words.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect();
    parts.join(" ").replace(base, PLACE)
}

pub fn demo(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    if !super::super::web::any_browser(d) {
        return Err(Fail::hint("the demo needs the browser", "wisply setup"));
    }
    let pause =
        if c.on("fast") { 0.0 } else { c.val("pause").and_then(|p| p.parse::<f64>().ok()).unwrap_or(PAUSE_S).max(0.0) };
    let srv = Server::start(Site::Demo).map_err(|e| Fail::new(format!("demo server: {e}")))?;
    let base = srv.base.as_str();
    let (mut blocks, mut work, mut chars, mut failed) = (Vec::new(), 0u128, 0usize, 0usize);
    let start = Instant::now();
    for (i, words) in steps(base).iter().enumerate() {
        if i > 0 && pause > 0.0 {
            thread::sleep(Duration::from_secs_f64(pause));
        }
        let t = Instant::now();
        let (out, err) = once(d, ctx, words);
        let ms = t.elapsed().as_millis();
        failed += usize::from(err);
        work += ms;
        let out = out.replace(base, PLACE).trim().to_string();
        let n = out.chars().count();
        chars += n;
        blocks.push(format!("$ wisply {}   ({ms} ms, ~{} tokens)\n{out}", shown(words, base), n / 4 + 1));
    }
    let (raw_out, raw_err) = once(d, ctx, &argv(&["js", RAW_JS]));
    let raw = if raw_err { 0 } else { raw_out.trim().parse::<usize>().map_or(0, |n| n / 4 + 1) };
    let with_pauses = if pause > 0.0 { " with pauses (--fast skips them)" } else { "" };
    let head = format!(
        "Wisply demo: {} commands on a made-up shop page, {:.1} s{with_pauses}",
        blocks.len(),
        start.elapsed().as_secs_f64()
    );
    let mut tail = format!(
        "{} commands, {:.1} s of work, ~{} tokens of answers in all",
        blocks.len(),
        work as f64 / 1000.0,
        chars / 4 + 1
    );
    if raw > 0 {
        tail.push_str(&format!("; the page itself is ~{raw} tokens of text"));
    }
    if failed > 0 {
        tail.push_str(&format!("; {failed} failed (wisply doctor)"));
    }
    let mut all = vec![head];
    all.extend(blocks);
    all.push(tail);
    Ok(all.join("\n\n"))
}
