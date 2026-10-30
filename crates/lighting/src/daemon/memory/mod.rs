pub mod chat;
mod code;
mod embed;
mod hook;
pub mod index;
pub mod learn;
mod learner;
mod status;
mod watch;
mod wiki;

use std::cell::RefCell;
use std::thread;
use std::time::Duration;

use lt_core::{Call, Fail};
use lt_memory::{DbResult, NewNote, Note, Store, Todo, date, day, oneline, project_of, scrub, short};

use super::Daemon;
use super::ctx::Ctx;
use super::web::number;

const RECALL_MAX: usize = 8;
const LINE: usize = 110;
const BODY_LINE: usize = 70;
const RECALL_COST: usize = 150;
const TITLE_MAX: usize = 160;
const SESSION_CHARS: usize = 8;
const FILES_SHOWN: usize = 8;
const TASK_SHORT: usize = 80;
const QUERY_BUDGET: Duration = Duration::from_secs(4);
const EMBED_BUDGET: Duration = Duration::from_secs(10);
const TODO_HELP: &str =
    "lighting todo | todo add \"task\" [--list name] | todo done|doing|drop|todo <id> | todo note <id> \"text\"";

thread_local! {
    static STORE: RefCell<Option<Store>> = const { RefCell::new(None) };
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    match c.name.as_str() {
        "recall" => recall(d, ctx, c),
        "remember" => remember(d, ctx, c),
        "forget" => forget(d, c),
        "todo" => todo(d, ctx, c),
        "hook" => hook::hook(d, ctx, c),
        "suggest" => hook::suggest(d, ctx, c),
        "memory" => status::memory(d, c),
        "wiki" => wiki::wiki(d, c),
        "code" => code::code(d, ctx, c),
        name => Err(super::route::later(name)),
    }
}

fn with_store<T>(d: &Daemon, f: impl FnOnce(&mut Store) -> DbResult<T>) -> Result<T, Fail> {
    STORE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(Store::open(&d.home).map_err(|e| Fail::new(format!("cannot open memory.db ({e})")))?);
        }
        let Some(store) = slot.as_mut() else { return Err(Fail::new("memory.db is not open")) };
        f(store).map_err(|e| Fail::new(format!("memory.db: {e}")))
    })
}

fn project(ctx: &Ctx, c: &Call) -> String {
    if c.on("global") {
        return String::new();
    }
    match c.val("project").filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => project_of(&ctx.cwd),
    }
}

fn is_ident(text: &str) -> bool {
    let digits = text.trim().trim_start_matches('#');
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && text.trim().matches('#').count() <= 1
}

fn ident(text: &str) -> Result<i64, Fail> {
    let fail = || Fail::hint(format!("'{text}' is not a note or to-do number"), "the number after # in recall or todo");
    if !is_ident(text) {
        return Err(fail());
    }
    text.trim().trim_start_matches('#').parse().map_err(|_| fail())
}

fn line(n: &Note, other: bool) -> String {
    let body = if !n.body.is_empty() && n.kind != "session" {
        oneline(n.body.split('\n').next().unwrap_or(""), BODY_LINE)
    } else {
        String::new()
    };
    let tag = match n.source.as_str() {
        "vault" => " (vault)",
        "claude" => " (claude memory)",
        _ => "",
    };
    let place = if other && !n.project.is_empty() { format!(" [{}]", n.project) } else { String::new() };
    let mut out = format!("#{} {} {}{place}{tag}: {}", n.id, day(n.updated), n.kind, oneline(&n.title, LINE));
    if !body.is_empty() && !out.to_lowercase().contains(&body.to_lowercase()) {
        out.push_str(&format!(" | {body}"));
    }
    let full = (n.title.chars().count() + n.body.chars().count()) / 4;
    if full >= RECALL_COST {
        out.push_str(&format!(" ~{}t", short(full)));
    }
    out
}

fn full_note(n: &Note) -> String {
    let files = n.path().map(String::from).unwrap_or_else(|| {
        let list: Vec<String> = serde_json::from_str(&n.files).unwrap_or_default();
        list.into_iter().take(FILES_SHOWN).collect::<Vec<_>>().join(", ")
    });
    let session: String = n.session.chars().take(SESSION_CHARS).collect();
    format!(
        "#{} {} {} [{}, {}, {}]\n{}{}{}",
        n.id,
        date(n.updated),
        n.kind,
        if n.project.is_empty() { "global" } else { &n.project },
        n.source,
        if session.is_empty() { "-" } else { &session },
        n.title,
        if n.body.is_empty() { String::new() } else { format!("\n{}", n.body) },
        if files.is_empty() { String::new() } else { format!("\nfiles: {files}") },
    )
}

fn recall(d: &Daemon, ctx: &Ctx, c: &Call) -> Result<String, Fail> {
    if c.pos.is_empty() {
        return Err(Fail::hint(
            "recall needs words or a number",
            "lighting recall \"how did we deploy\" | lighting recall 42",
        ));
    }
    if let [one] = c.pos.as_slice()
        && is_ident(one)
    {
        let id = ident(one)?;
        let found = with_store(d, |s| {
            let n = s.note(id)?;
            if n.is_some() {
                s.touch(&[id])?;
            }
            Ok(n)
        })?;
        return found
            .map(|n| full_note(&n))
            .ok_or_else(|| Fail::hint(format!("no note #{id}"), "lighting recall \"<words>\""));
    }
    let text = c.pos.join(" ");
    let here = project(ctx, c);
    let (vec, why) = match embed::embed(&d.home, std::slice::from_ref(&text), true, QUERY_BUDGET) {
        Ok(mut v) => (v.pop(), String::new()),
        Err(why) => (None, why),
    };
    let limit = number(c, "max")?.filter(|n| *n > 0).map_or(RECALL_MAX, |n| usize::try_from(n).unwrap_or(RECALL_MAX));
    let scope = if c.on("all") { "" } else { here.as_str() };
    let found = with_store(d, |s| {
        let hits = s.search(&text, scope, limit, vec.as_deref(), 0.0)?;
        s.touch(&hits.iter().map(|h| h.note.id).collect::<Vec<_>>())?;
        Ok(hits)
    })?;
    let note = if why.is_empty() { String::new() } else { format!("words only, {why}") };
    if found.is_empty() {
        let tail = if note.is_empty() { String::new() } else { format!(" ({note})") };
        return Ok(format!("nothing remembered about \"{text}\"{tail}"));
    }
    let lines: Vec<String> = found.iter().map(|h| line(&h.note, !here.is_empty() && h.note.project != here)).collect();
    let tail = if note.is_empty() { String::new() } else { format!("; {note}") };
    Ok(format!("{}\n(lighting recall <id> shows one in full, ~Nt = its size{tail})", lines.join("\n")))
}

fn first_sentence(text: &str) -> &str {
    let mut prev = ' ';
    for (i, ch) in text.char_indices() {
        if ch.is_whitespace() && matches!(prev, '.' | '!' | '?') {
            return &text[..i];
        }
        prev = ch;
    }
    text
}

fn remember(d: &Daemon, ctx: &Ctx, c: &Call) -> Result<String, Fail> {
    let text = scrub(&c.pos.join(" ").split_whitespace().collect::<Vec<_>>().join(" "));
    if text.chars().count() < 4 {
        return Err(Fail::hint("what should Lighting remember?", "lighting remember \"Brave is the only browser\""));
    }
    let kind = c.val("kind").filter(|k| !k.is_empty()).unwrap_or("fact");
    let id = keep(d, ctx, text, kind, &project(ctx, c))?;
    Ok(format!("remembered #{id} (lighting forget {id} undoes it)"))
}

fn keep(d: &Daemon, ctx: &Ctx, text: String, kind: &str, here: &str) -> Result<i64, Fail> {
    let first = first_sentence(&text);
    let title = if first.chars().count() <= TITLE_MAX { first.to_string() } else { oneline(&text, TITLE_MAX) };
    let body = if text == title { "" } else { text.as_str() };
    let new = NewNote { project: here, kind, title: &title, body, source: "user", session: &ctx.sid };
    let id = with_store(d, |s| s.add_note(&new))?.ok_or_else(|| Fail::new("nothing to remember"))?;
    let home = d.home.clone();
    let _ = thread::Builder::new().name("embed".into()).spawn(move || {
        if let Ok(mut vecs) = embed::embed(&home, std::slice::from_ref(&text), false, EMBED_BUDGET)
            && let Some(v) = vecs.pop()
            && let Ok(mut s) = Store::open(&home)
        {
            let _ = s.save_vecs(&[(id, v)]);
        }
    });
    Ok(id)
}

fn forget(d: &Daemon, c: &Call) -> Result<String, Fail> {
    let Some(first) = c.pos.first() else { return Err(Fail::hint("forget needs a note number", "lighting forget 42")) };
    let id = ident(first)?;
    match with_store(d, |s| s.forget(id))? {
        Err(msg) => Err(Fail::new(msg)),
        Ok(0) => Ok(format!("no note #{id}")),
        Ok(_) => Ok(format!("forgot #{id}")),
    }
}

fn todo_line(t: &Todo, short: bool) -> String {
    let last = t.note.trim().split('\n').next_back().unwrap_or("");
    let doing = if t.status == "doing" { "[doing] " } else { "" };
    let note = if last.is_empty() || short { String::new() } else { format!(" ({})", oneline(last, TASK_SHORT)) };
    format!("#{} {doing}{}{note}", t.id, oneline(&t.task, if short { TASK_SHORT } else { LINE }))
}

fn todo_list(rows: &[Todo], short: bool) -> String {
    let mut out = Vec::new();
    let mut last: Option<&str> = None;
    for t in rows {
        if last != Some(t.list.as_str()) {
            out.push(format!("{}:", if t.list.is_empty() { "to-dos" } else { &t.list }));
            last = Some(t.list.as_str());
        }
        out.push(format!("  {}", todo_line(t, short)));
    }
    out.join("\n")
}

fn todo(d: &Daemon, ctx: &Ctx, c: &Call) -> Result<String, Fail> {
    let sub = c.pos.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let here = project(ctx, c);
    let list = c.val("list").unwrap_or("");
    match sub.as_str() {
        "add" => {
            let task = c.pos[1..].join(" ");
            if task.is_empty() {
                return Err(Fail::hint("todo add needs the task", TODO_HELP));
            }
            let status = if c.on("doing") { "doing" } else { "todo" };
            let id = with_store(d, |s| s.todo_add(&here, &task, list, status))?;
            let to = if list.is_empty() { String::new() } else { format!(" to {list}") };
            Ok(format!("to-do #{id} added{to}"))
        }
        "done" | "doing" | "drop" | "dropped" | "todo" | "open" => {
            let Some(which) = c.pos.get(1) else { return Err(Fail::hint("which to-do?", TODO_HELP)) };
            let status = match sub.as_str() {
                "drop" => "dropped",
                "open" => "todo",
                other => other,
            };
            let id = ident(which)?;
            let text = c.pos[2..].join(" ");
            let note = (!text.is_empty()).then_some(text.as_str());
            let t = with_store(d, |s| s.todo_set(id, Some(status), note))?;
            let t = t.ok_or_else(|| Fail::hint(format!("no to-do #{which}"), "lighting todo"))?;
            Ok(format!("#{} is {}: {}", t.id, t.status, oneline(&t.task, TASK_SHORT)))
        }
        "note" => {
            if c.pos.len() < 3 {
                return Err(Fail::hint("todo note needs a number and the text", TODO_HELP));
            }
            let id = ident(&c.pos[1])?;
            let text = c.pos[2..].join(" ");
            let t = with_store(d, |s| s.todo_set(id, None, Some(&text)))?;
            let t = t.ok_or_else(|| Fail::hint(format!("no to-do #{}", c.pos[1]), "lighting todo"))?;
            Ok(format!("#{} noted", t.id))
        }
        "" => {
            let scope = if c.on("all") { None } else { Some(here.as_str()) };
            let mut rows = with_store(d, |s| s.todos(scope, true))?;
            if !list.is_empty() {
                rows.retain(|t| t.list == list);
            }
            Ok(if rows.is_empty() {
                "no open to-dos (lighting todo add \"task\")".into()
            } else {
                todo_list(&rows, false)
            })
        }
        other => Err(Fail::hint(format!("unknown todo command '{other}'"), TODO_HELP)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_sentences_and_lines_read_like_2x() {
        assert_eq!(ident("#42").ok(), Some(42));
        assert_eq!(ident(" 7 ").ok(), Some(7));
        assert!(ident("4a").is_err() && ident("##4").is_err() && ident("").is_err());
        assert_eq!(first_sentence("Brave only. Never Chrome!"), "Brave only.");
        assert_eq!(first_sentence("no stop here"), "no stop here");
        let n = Note {
            id: 3,
            kind: "fact".into(),
            title: "Deploy with ship.sh".into(),
            body: "ship.sh does it all\nsecond line".into(),
            project: "web".into(),
            source: "vault".into(),
            updated: 0.0,
            ..Note::default()
        };
        let shown = line(&n, true);
        assert!(shown.starts_with("#3 ") && shown.contains(" fact [web] (vault): Deploy with ship.sh"), "{shown}");
        assert!(shown.ends_with(" | ship.sh does it all"), "{shown}");
        let t = Todo {
            id: 5,
            task: "ship it".into(),
            status: "doing".into(),
            note: "a\n10-30 12:00 halfway".into(),
            ..Todo::default()
        };
        assert_eq!(todo_line(&t, false), "#5 [doing] ship it (10-30 12:00 halfway)");
        assert_eq!(todo_line(&t, true), "#5 [doing] ship it");
        let rows = vec![Todo { list: String::new(), ..t.clone() }, Todo { id: 6, list: "release".into(), ..t }];
        let list = todo_list(&rows, false);
        assert!(list.starts_with("to-dos:\n  #5 ") && list.contains("\nrelease:\n  #6 "));
    }
}
