mod deps;
mod scan;

use std::path::{Path, PathBuf};
use std::time::Duration;

use lt_core::{Call, Fail};

use super::ctx::Ctx;
use super::{Daemon, Out, game, media, web};
use scan::Finding;

const WAIT: Duration = Duration::from_secs(600);
const CHARS: usize = 8000;

fn block(lines: &mut Vec<String>, title: &str, found: &[Finding]) {
    if found.is_empty() {
        return;
    }
    lines.push(format!("{title} ({}):", found.len()));
    for f in found {
        let at = if f.line > 0 { format!("{}:{}", f.file, f.line) } else { f.file.clone() };
        lines.push(format!("  {at} {} -> {}", f.why, f.fix));
    }
}

fn report(root: &Path, history: bool, check_deps: bool) -> String {
    let names = scan::files(root);
    let (secrets, risky) = scan::scan_files(root, &names);
    let mut lines = vec![format!("[secure] {} ({} files)", root.to_string_lossy().replace('\\', "/"), names.len())];
    block(&mut lines, "secrets", &secrets);
    let mut checked = String::new();
    if check_deps {
        let (all, notes) = deps::collect(root, &names);
        match deps::check(root, &all) {
            Ok(found) => {
                block(&mut lines, &format!("vulnerable dependencies, {} checked on osv.dev", all.len()), &found);
                checked = format!("{} dependencies", all.len());
            }
            Err(why) => lines.push(format!("! dependencies not checked: {why}")),
        }
        lines.extend(notes.into_iter().map(|n| format!("! {n}")));
    }
    block(&mut lines, "risky spots to look at", &risky);
    if history {
        block(&mut lines, "secrets in the git history", &scan::scan_history(root));
    }
    if lines.len() == 1 {
        let what = if checked.is_empty() { String::new() } else { format!(", {checked}") };
        lines.push(format!("no findings (secrets, risky code{what}{})", if history { ", history" } else { "" }));
    }
    lines.join("\n")
}

pub fn run(_: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let playing = game::running();
    if !playing.is_empty() {
        return Err(Fail::hint(format!("{playing} is running"), "wisply secure runs when the game is closed"));
    }
    let start = match c.pos.first() {
        Some(p) => web::local_file(ctx, p),
        None => PathBuf::from(if ctx.cwd.is_empty() { "." } else { ctx.cwd.as_str() }),
    };
    if !start.is_dir() {
        return Err(Fail::hint(format!("no folder {}", start.display()), "wisply secure <project folder>"));
    }
    let root = lt_memory::root_of(&start).unwrap_or(start);
    let (history, check_deps) = (c.on("history"), !c.on("no-deps"));
    let pending = media::background(
        "secure",
        WAIT,
        "wisply secure --no-deps (skips the online check)",
        move || Ok(report(&root, history, check_deps)),
        |d, ctx, text: String| Ok(web::capped(d, ctx, &text, "secure", None, CHARS)),
    );
    Ok(Out::Wait(pending))
}
