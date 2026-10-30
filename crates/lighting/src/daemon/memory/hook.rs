use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

use lt_core::{Call, Fail};
use lt_memory::{DbResult, Store, day, now, oneline, project_of, stop_word};
use regex::Regex;
use serde_json::Value;

use super::super::ctx::Ctx;
use super::super::{Daemon, config};
use super::{LINE, embed, keep, todo_list, with_store};

const START_TODOS: usize = 12;
const START_SESSIONS: usize = 3;
const START_COMMITS: usize = 5;
const START_RULES: usize = 5;
const START_NOTES: usize = 8;
const ABOUT_LINES: usize = 3;
const ABOUT_CHARS: usize = 220;
const GIT_WAIT: Duration = Duration::from_secs(3);
const GIT_POLL: Duration = Duration::from_millis(20);
const TYPED_PREFIX: usize = 400;
const HINT_BUDGET: Duration = Duration::from_millis(800);
const HINT_SIM: f32 = 0.6;
const HINT_POOL: usize = 8;
const HINTS: usize = 2;
const HINT_TERMS: usize = 2;
const HINT_SHARE: f64 = 0.4;
const SAVED_MIN: usize = 6;
const STEM: usize = 5;

static SAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?is)^\s*(?:bitte\s+|please\s+)?(?:merk(?:e)?\s+dir|notier(?:e)?\s+dir|remember)\b[\s,:;-]*(?:(?:bitte|please)\s+)?(?:dass\s+|that\s+)?(.+)",
    )
    .expect("say pattern")
});
static NOT_TYPED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\[SYSTEM NOTIFICATION|<task-notification>|^\s*<(?:system-reminder|local-command-\w+|command-name)>|^\s*This session is being continued from a previous conversation",
    )
    .expect("typed pattern")
});
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[@#]|[^\W\d_][\w-]*").expect("word pattern"));
static PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:[A-Za-z]:|~|\.{1,2})?[\\/][^\s"'<>|]*"#).expect("path pattern"));

fn data_text<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn hook(d: &Daemon, ctx: &Ctx, c: &Call) -> Result<String, Fail> {
    if !config::on(&config::load(&d.home), "memory") {
        return Ok(String::new());
    }
    let event = c.pos.first().map(|e| e.to_lowercase()).unwrap_or_default();
    let data: Value = c.pos.get(1).and_then(|raw| serde_json::from_str(raw).ok()).unwrap_or(Value::Null);
    let cwd = Some(data_text(&data, "cwd")).filter(|c| !c.is_empty()).unwrap_or(ctx.cwd.as_str()).to_string();
    let here = project_of(&cwd);
    let sid = Some(data_text(&data, "session_id")).filter(|s| !s.is_empty()).unwrap_or(ctx.sid.as_str()).to_string();
    let transcript = data_text(&data, "transcript_path");
    with_store(d, |s| s.session_seen(&sid, &here, &cwd, transcript, event == "end"))?;
    if event != "start" {
        return Ok(String::new());
    }
    if data_text(&data, "source") == "compact" {
        return with_store(d, |s| compact_text(s, &here));
    }
    let commits = if here.is_empty() || cwd.is_empty() { Vec::new() } else { commits(&cwd) };
    with_store(d, |s| start_text(s, &here, commits))
}

fn listed(head: &str, rows: Vec<(i64, String)>) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut out = vec![head.to_string()];
    out.extend(rows.into_iter().map(|(id, title)| format!("  #{id} {}", oneline(&title, LINE))));
    out
}

fn compact_text(s: &mut Store, here: &str) -> DbResult<String> {
    let mut lines = vec![format!(
        "Lighting after the summary ({}): lighting recall \"<words>\" searches your notes, lighting todo tracks work.",
        if here.is_empty() { "no project" } else { here }
    )];
    lines.extend(listed("Project rules:", s.titles(here, true, START_RULES)?));
    lines.extend(listed("Remembered:", s.titles(here, false, START_NOTES)?));
    Ok(lines.join("\n"))
}

fn start_text(s: &mut Store, here: &str, commits: Vec<String>) -> DbResult<String> {
    let mut lines = vec![format!(
        "Lighting memory ({}): lighting recall \"<words>\" searches {} notes (vault, session notes and Claude memory \
         included), lighting remember \"<fact>\" keeps one, lighting todo tracks work.",
        if here.is_empty() { "no project" } else { here },
        s.note_count()?
    )];
    let about: Vec<String> = s
        .project_about(here)?
        .lines()
        .filter(|x| !x.trim().is_empty() && !x.starts_with('#') && !x.starts_with("---"))
        .map(|x| x.trim().to_string())
        .take(ABOUT_LINES)
        .collect();
    if !about.is_empty() {
        lines.push(format!("About: {}", oneline(&about.join(" "), ABOUT_CHARS)));
    }
    let scope = if here.is_empty() { None } else { Some(here) };
    let todos: Vec<_> = s.todos(scope, true)?.into_iter().take(START_TODOS).collect();
    if !todos.is_empty() {
        lines.push("Open to-dos:".into());
        lines.extend(todo_list(&todos, true).lines().map(|l| format!("  {l}")));
    }
    let recent = s.sessions(here, START_SESSIONS)?;
    if !recent.is_empty() {
        lines.push("Last sessions:".into());
        for r in recent {
            let next = if r.next.is_empty() { String::new() } else { format!(" Open: {}", oneline(&r.next, 90)) };
            let title = if r.title.is_empty() { "-" } else { r.title.as_str() };
            lines.push(format!("  {} {}: {}{next}", day(r.last), oneline(title, 50), oneline(&r.summary, 100)));
        }
    }
    lines.extend(commits);
    lines.extend(listed("Project rules:", s.titles(here, true, START_RULES)?));
    lines.extend(listed("Remembered:", s.titles(here, false, START_NOTES)?));
    let drafts = s.drafts()?;
    if drafts > 0 {
        lines.push(format!("{drafts} wiki drafts wait for a yes or no: lighting wiki"));
    }
    Ok(lines.join("\n"))
}

fn commits(cwd: &str) -> Vec<String> {
    let mut cmd = Command::new("git");
    cmd.args(["-C", cwd, "log", &format!("-{START_COMMITS}"), "--format=%h %ad %s", "--date=format:%m-%d"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(mut child) = cmd.spawn() else { return Vec::new() };
    let started = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() < GIT_WAIT => thread::sleep(GIT_POLL),
            _ => {
                let _ = child.kill();
                break false;
            }
        }
    };
    let mut out = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut pipe, &mut out);
    }
    let _ = child.wait();
    let rows: Vec<String> = out.lines().filter(|l| !l.is_empty()).map(|l| format!("  {}", oneline(l, 100))).collect();
    if !ok || rows.is_empty() {
        return Vec::new();
    }
    let mut lines = vec!["Last commits:".to_string()];
    lines.extend(rows);
    lines
}

fn words(text: &str) -> Vec<String> {
    WORD.find_iter(&text.to_lowercase())
        .map(|m| m.as_str().to_string())
        .filter(|t| t == "@" || t == "#" || (t.chars().count() > 2 && !stop_word(t)))
        .collect()
}

fn without_paths(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut at = 0;
    while let Some(m) = PATH.find_at(text, at) {
        let before = text[..m.start()].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '/' | '.' | ':')) {
            at = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        out.push_str(&text[last..m.start()]);
        out.push(' ');
        last = m.end();
        at = m.end().max(m.start() + 1);
        if at > text.len() {
            break;
        }
    }
    out.push_str(&text[last..]);
    out
}

fn stem(w: &str) -> String {
    w.chars().take(STEM).collect()
}

fn stems(text: &str) -> HashSet<String> {
    words(text).iter().map(|w| stem(w)).collect()
}

pub fn suggest(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let raw = c.pos.join(" ");
    let prompt = serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| v.get("prompt").and_then(Value::as_str).map(String::from))
        .unwrap_or(raw);
    let head: String = prompt.chars().take(TYPED_PREFIX).collect();
    if prompt.is_empty() || NOT_TYPED.is_match(&head) || !config::on(&config::load(&d.home), "memory") {
        return Ok(String::new());
    }
    if let Some(m) = SAY.captures(&prompt)
        && m[1].trim().chars().count() >= SAVED_MIN
    {
        let said = m[1].trim().trim_end_matches('.').to_string();
        let id = keep(d, ctx, said, "preference", &project_of(&ctx.cwd))?;
        return Ok(format!(
            "Lighting memory saved #{id}: \"{}\" (lighting forget {id} if that was not meant to be kept)",
            oneline(&m[1], 90)
        ));
    }
    let asked = stems(&without_paths(&prompt));
    if asked.len() < 3 {
        return Ok(String::new());
    }
    let here = project_of(&ctx.cwd);
    let Ok(mut vecs) = embed::embed(&d.home, std::slice::from_ref(&prompt), true, HINT_BUDGET) else {
        return Ok(String::new());
    };
    let vec = vecs.pop();
    let hits = with_store(d, |s| s.search(&prompt, &here, HINT_POOL, vec.as_deref(), HINT_SIM))?;
    let mut out = Vec::new();
    for h in hits {
        let n = &h.note;
        let key = format!("m:{}", n.id);
        if ctx.told.contains_key(&key)
            || n.kind == "session"
            || n.kind == "draft"
            || (n.project != here && !n.project.is_empty())
        {
            continue;
        }
        let have = stems(&format!("{} {}", n.title, n.body));
        let hit = asked.intersection(&have).count();
        let words_ok = hit >= HINT_TERMS && hit as f64 / asked.len() as f64 >= HINT_SHARE;
        let titled = !asked.is_disjoint(&stems(&n.title));
        if words_ok && titled && h.sim.unwrap_or(0.0) >= HINT_SIM {
            ctx.told.insert(key, now());
            out.push(format!("#{} {}", n.id, oneline(&n.title, 100)));
        }
        if out.len() >= HINTS {
            break;
        }
    }
    Ok(if out.is_empty() {
        String::new()
    } else {
        format!("Lighting memory may help (lighting recall <id>): {}", out.join(" | "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_read_like_2x() {
        assert_eq!(words("Fix the Login-Page in @web #42"), ["fix", "login-page", "@", "web", "#"]);
        assert_eq!(without_paths("open C:/x/y.txt and a/b or ~/c"), "open   and a/b or  ");
        assert_eq!(stem("deployment"), "deplo");
        assert!(NOT_TYPED.is_match("  <system-reminder>y") && !NOT_TYPED.is_match("fix the bug"));
        let m = SAY.captures("Bitte merk dir, dass Brave der einzige Browser ist.").unwrap();
        assert_eq!(&m[1], "Brave der einzige Browser ist.");
        assert!(SAY.captures("remembering things").is_none());
    }
}
