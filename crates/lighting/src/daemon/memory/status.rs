use lt_core::{Call, Fail};
use lt_memory::{now, oneline};

use super::super::route::later;
use super::super::{Daemon, config, game};
use super::{chat, embed, index, learner, with_store};

const MEMORY_HELP: &str = "wisply memory [learn | on | off | lessons | unlearn <n>]";
const FIX_CHARS: usize = 80;

pub fn memory(d: &Daemon, c: &Call) -> Result<String, Fail> {
    let sub = c.pos.first().map(|s| s.to_lowercase()).unwrap_or_default();
    match sub.as_str() {
        "" => status(d),
        "on" | "off" => {
            config::command(&d.home, &["memory".into(), sub.clone()])?;
            Ok(format!("memory {sub}"))
        }
        "lessons" => lessons(d),
        "unlearn" => unlearn(d, c),
        "learn" => learn(d),
        "import" | "model" => Err(later(&format!("memory {sub}"))),
        _ => Err(Fail::hint(format!("unknown memory command '{sub}'"), MEMORY_HELP)),
    }
}

fn status(d: &Daemon) -> Result<String, Fail> {
    let on = config::on(&config::load(&d.home), "memory");
    let (c, indexed, learned) = with_store(d, |s| Ok((s.counts()?, s.meta("vault_indexed")?, s.lessons()?.len())))?;
    let mut lines = vec![format!(
        "memory {}: {} notes ({} with vectors), {} open to-dos, {} session summaries, {} sessions to learn",
        if on { "on" } else { "off" },
        c.notes,
        c.vecs,
        c.todos,
        c.summaries,
        c.queued
    )];
    lines.push(format!("model: {}", embed::status(&d.home)));
    let teacher =
        chat::provider(&d.home).map_or_else(|| "none (GEMINI_API_KEY or memory_api_model)".into(), |p| p.name());
    lines.push(format!("learns with: {teacher}"));
    let st = learner::state();
    if !st.last.is_empty() {
        lines.push(format!("last: {} ({} notes learned since the start)", st.last, st.learned));
    }
    if !st.error.is_empty() {
        lines.push(format!("learner problem: {}", st.error));
    }
    let game = if c.vecs < c.notes { game::running() } else { String::new() };
    if !game.is_empty() {
        lines.push(format!("vectors wait: {game} runs"));
    }
    if let Some(t) = indexed.and_then(|v| v.parse::<f64>().ok()) {
        lines.push(format!("vault and Claude memory indexed {} ago", lt_core::ago((now() - t).max(0.0) as u64)));
    }
    let problem = index::problem();
    if !problem.is_empty() {
        lines.push(format!("problem: {problem}"));
    }
    if learned > 0 {
        lines.push(format!("{learned} learned fixes (wisply memory lessons)"));
    }
    let db = d.home.join("memory").join("memory.db");
    lines.push(format!("kept on this PC only ({}) | {MEMORY_HELP}", db.display().to_string().replace('\\', "/")));
    Ok(lines.join("\n"))
}

fn learn(d: &Daemon) -> Result<String, Fail> {
    with_store(d, |s| s.sessions_retry_all())?;
    let queued = with_store(d, |s| s.counts())?.queued;
    index::learn_now();
    Ok(format!("learning {queued} sessions in the background (wisply memory shows the state)"))
}

fn lessons(d: &Daemon) -> Result<String, Fail> {
    let all = with_store(d, |s| s.lessons())?;
    if all.is_empty() {
        return Ok("no learned fixes yet (one is kept when a command fails and another works right after)".into());
    }
    let mut lines = vec![format!("{} learned fixes (wisply memory unlearn <n> drops one):", all.len())];
    for (i, l) in all.iter().enumerate() {
        let place = if l.target.is_empty() { String::new() } else { format!(" on {}", l.target) };
        let times = if l.ups > 1 { format!(", {}x", l.ups) } else { String::new() };
        let fix = oneline(&l.fix, FIX_CHARS);
        lines.push(format!("  {} {}{place}: {} -> `wisply {fix}`{times}", i + 1, l.cmd, l.kind));
    }
    Ok(lines.join("\n"))
}

fn unlearn(d: &Daemon, c: &Call) -> Result<String, Fail> {
    let all = with_store(d, |s| s.lessons())?;
    let pick = c.pos.get(1).and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=all.len()).contains(n));
    let Some(n) = pick else {
        let which: String =
            if all.is_empty() { "no learned fixes to drop".into() } else { format!("which fix? 1 to {}", all.len()) };
        return Err(Fail::hint(which, "wisply memory lessons"));
    };
    let l = &all[n - 1];
    with_store(d, |s| s.unlearn(&l.sig))?;
    Ok(format!("dropped fix {n}: {} {} -> `wisply {}`", l.cmd, l.kind, oneline(&l.fix, FIX_CHARS)))
}
