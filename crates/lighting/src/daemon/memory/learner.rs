use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use lt_memory::{
    Due, NewNote, Record, Store, WIKI_KINDS, chunks, condense, now, oneline, points, project_of_files, render_page,
    scrub, similar, template, titled, write_draft, write_session,
};
use serde_json::{Value, json};

use super::super::{config, game};
use super::chat::{self, ChatError, Provider};
use super::index::vault;

const KINDS: &[&str] = &["decision", "preference", "bugfix", "feature", "fact", "discovery", "change"];
const SYSTEM: &str = "You keep the long-term memory of a coding assistant. From the session log, keep only what helps in a FUTURE \
session: decisions with their reason, the user's preferences and rules, bugs fixed with the root cause, \
non-obvious facts (paths, ports, commands, versions, accounts; never passwords, keys or tokens). Skip routine \
steps, file listings, what anyone sees in the code, progress and status (counts, timings, test or CI runs, \
what is running right now) and plans; what is still open goes into next, never into a note. Each note stands \
alone: its title starts with the project or tool, like 'Lighting: hints need words and meaning'. Write \
English, short and concrete. At most 6 notes; fewer is fine, none is fine. For the session record write only \
what THIS part of the log adds, one short line per point, what and why, never how: summary = what changed \
and where, why = the reasons behind the decisions, rejected = what was tried and dropped and why, next = what \
is left open at the end of this part. lessons = at most 2 things worth a permanent page in the user's \
knowledge base: mistake = an error with its root cause and the fix, rule = a fixed rule the user gave, \
pattern = a reusable way of doing something; none is fine.";
const DRAFT_SYSTEM: &str = "Write one page for the user's knowledge base from the note and the session context. Fill each section with \
1 to 4 short lines in the language of the section headings; leave a section empty when the note says nothing \
for it. Concrete names, versions and commands, never passwords, keys or tokens. The title says the lesson in \
a few words.";
const NOTES_PER_CHUNK: usize = 6;
const LESSONS_PER_CHUNK: usize = 2;
const WIKI_PER_SESSION: usize = 2;
const MIN_CHARS: usize = 600;
const QUIET_S: f64 = 600.0;
const RETRY_S: f64 = 1800.0;
const POLL_S: f64 = 60.0;
const BACKOFF_STEPS: u32 = 4;
const DUP_SIM: f64 = 0.7;
const DRAFT_TITLE: usize = 100;
const INBOX: &str = "Inbox";

#[derive(Default, Clone)]
pub struct State {
    pub last: String,
    pub error: String,
    pub learned: usize,
}

static STATE: Mutex<State> = Mutex::new(State { last: String::new(), error: String::new(), learned: 0 });

pub fn state() -> State {
    STATE.lock().map(|s| s.clone()).unwrap_or_default()
}

fn set(f: impl FnOnce(&mut State)) {
    if let Ok(mut s) = STATE.lock() {
        f(&mut s);
    }
}

fn schema() -> Value {
    let text = json!({"type": "string"});
    let note = json!({"type": "object", "required": ["kind", "title", "body"], "properties": {
        "kind": {"type": "string", "enum": KINDS}, "title": text, "body": text,
        "files": {"type": "array", "items": text}}});
    let lesson = json!({"type": "object", "required": ["kind", "title", "body"], "properties": {
        "kind": {"type": "string", "enum": WIKI_KINDS}, "title": text, "body": text}});
    json!({"type": "object", "required": ["title", "summary", "why", "rejected", "next", "notes", "lessons"],
        "properties": {"title": text, "summary": text, "why": text, "rejected": text, "next": text,
            "notes": {"type": "array", "maxItems": NOTES_PER_CHUNK, "items": note},
            "lessons": {"type": "array", "maxItems": LESSONS_PER_CHUNK, "items": lesson}}})
}

pub fn parse(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok().or_else(|| {
        let (a, b) = (text.find('{')?, text.rfind('}')?);
        (a < b).then(|| serde_json::from_str(&text[a..=b]).ok()).flatten()
    })
}

fn field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

static WAITS: AtomicU32 = AtomicU32::new(0);

fn backoff() -> f64 {
    let n = WAITS.fetch_add(1, Ordering::Relaxed).min(BACKOFF_STEPS);
    (POLL_S * f64::from(1u32 << n)).min(QUIET_S)
}

enum Stop {
    Wait(String),
    Fail(String),
}

impl From<ChatError> for Stop {
    fn from(e: ChatError) -> Stop {
        match e {
            ChatError::NotReady(why) => Stop::Wait(why),
            ChatError::Failed(why) => Stop::Fail(why),
        }
    }
}

fn inbox(home: &Path) -> std::path::PathBuf {
    let name = config::text(&config::load(home), "vault_inbox");
    vault(home).join(if name.trim().is_empty() { INBOX } else { name.trim() })
}

fn draft(home: &Path, store: &mut Store, p: &Provider, lesson: &Value, project: &str, sid: &str, r: &Record) {
    let (kind, title, body) = (field(lesson, "kind"), field(lesson, "title"), field(lesson, "body"));
    let known = store.search(&title, project, 5, None, 0.0).unwrap_or_default();
    if known.iter().any(|h| h.note.source == "vault" && similar(&h.note.title, &title) >= DUP_SIM) {
        return;
    }
    let root = vault(home);
    let heads = template(&root, &kind);
    let props: serde_json::Map<String, Value> = heads.iter().map(|h| (h.clone(), json!({"type": "string"}))).collect();
    let schema = json!({"type": "object", "required": ["title", "sections"], "properties": {
        "title": {"type": "string"}, "sections": {"type": "object", "required": heads, "properties": props}}});
    let shown = if project.is_empty() { "-" } else { project };
    let user =
        format!("Project: {shown}\nKind: {kind}\nLesson: {title}\n{body}\n\nSession: {}\n{}", r.title, r.summary);
    let Ok(answer) = chat::chat(p, DRAFT_SYSTEM, &user, &schema) else { return };
    let Some(out) = parse(&answer) else { return };
    let sections: Vec<(String, String)> = out
        .get("sections")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(h, v)| (h.clone(), scrub(v.as_str().unwrap_or("")))).collect())
        .unwrap_or_default();
    let named = Some(field(&out, "title")).filter(|t| !t.trim().is_empty()).unwrap_or(title);
    let named = oneline(&scrub(&named), DRAFT_TITLE);
    let page = render_page(&root, &kind, &named, &sections, project, sid);
    write_draft(&inbox(home), &kind, &named, &page);
}

fn learn(home: &Path, store: &mut Store, p: &Provider, row: &Due) -> Result<usize, Stop> {
    let path = Path::new(&row.transcript);
    if !path.exists() {
        store.session_gone(&row.id).map_err(|e| Stop::Fail(e.to_string()))?;
        return Ok(0);
    }
    let began = now();
    let c = condense(path, row.offset, None).map_err(|e| Stop::Fail(format!("transcript: {e}")))?;
    let project = Some(project_of_files(&c.paths)).filter(|p| !p.is_empty()).unwrap_or_else(|| row.project.clone());
    let db = |e: lt_memory::DbError| Stop::Fail(format!("memory.db: {e}"));
    if project != row.project {
        store.session_project(&row.id, &project).map_err(db)?;
    }
    if c.text.chars().count() < MIN_CHARS {
        store.session_short(&row.id, c.end, began, &c.title).map_err(db)?;
        return Ok(0);
    }
    let mut r = Record {
        title: if c.title.is_empty() { row.title.clone() } else { c.title.clone() },
        summary: row.summary.clone(),
        why: row.why.clone(),
        rejected: row.rejected.clone(),
        next: row.next.clone(),
    };
    let (mut added, mut lessons) = (0, Vec::new());
    let schema = schema();
    for part in chunks(&c.text) {
        if p.local() && !game::running().is_empty() {
            return Err(Stop::Wait("a game started".into()));
        }
        let shown = if project.is_empty() { "-" } else { project.as_str() };
        let title = if r.title.is_empty() { "-" } else { r.title.as_str() };
        let user = format!("Project: {shown}\nSession title: {title}\n\nSession log:\n{part}");
        let answer = chat::chat(p, SYSTEM, &user, &schema)?;
        let out = parse(&answer).ok_or_else(|| Stop::Fail(format!("{} sent no JSON object", p.name())))?;
        for n in out.get("notes").and_then(Value::as_array).into_iter().flatten() {
            let title = field(n, "title");
            if title.trim().is_empty() {
                continue;
            }
            let kind = Some(field(n, "kind")).filter(|k| KINDS.contains(&k.as_str())).unwrap_or_else(|| "fact".into());
            let files: Vec<String> = n
                .get("files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|f| f.as_str().map(String::from))
                .collect();
            let full = titled(&project, &title);
            let body = field(n, "body");
            let note = NewNote {
                project: &project,
                kind: &kind,
                title: &full,
                body: &body,
                source: "session",
                session: &row.id,
            };
            if let Some((_, true)) = store.merge_note(&note, &files).map_err(db)? {
                added += 1;
            }
        }
        let found = out.get("lessons").and_then(Value::as_array).into_iter().flatten();
        lessons.extend(
            found.filter(|n| WIKI_KINDS.contains(&field(n, "kind").as_str()) && !field(n, "title").is_empty()).cloned(),
        );
        r.summary = points(&r.summary, &field(&out, "summary"));
        r.why = points(&r.why, &field(&out, "why"));
        r.rejected = points(&r.rejected, &field(&out, "rejected"));
        let next = points("", &field(&out, "next"));
        if !next.is_empty() {
            r.next = next;
        }
        if r.title.is_empty() {
            r.title = field(&out, "title");
        }
    }
    for lesson in lessons.iter().take(WIKI_PER_SESSION) {
        draft(home, store, p, lesson, &project, &row.id, &r);
    }
    let open_since = if c.more { 0.0 } else { began };
    store.session_learned(&row.id, c.end, open_since, &r).map_err(db)?;
    let started = if row.started > 0.0 { row.started } else { now() };
    if write_session(&vault(home), &row.id, &project, &r, started).is_none() {
        store.session_note(&row.id, &project, &r).map_err(db)?;
    }
    Ok(added)
}

pub fn tick(home: &Path, store: &mut Store, soon: bool) {
    let cfg = config::load(home);
    if !config::on(&cfg, "memory") || !(soon || config::on(&cfg, "memory_learn")) {
        return;
    }
    let t = now();
    let Ok(due) = store.due_sessions(if soon { t + QUIET_S } else { t }, QUIET_S) else { return };
    if due.is_empty() {
        return;
    }
    let Some(p) = chat::provider(home) else {
        set(|s| {
            s.error = "no model to learn with (GEMINI_API_KEY, or lighting config memory_api + memory_api_model)".into()
        });
        return;
    };
    if p.local() && !game::running().is_empty() {
        return;
    }
    for row in &due {
        match learn(home, store, &p, row) {
            Ok(n) => set(|s| {
                WAITS.store(0, Ordering::Relaxed);
                s.learned += n;
                s.last = format!(
                    "{} learned {}",
                    lt_core::local_now().get(11..16).unwrap_or(""),
                    row.id.chars().take(8).collect::<String>()
                );
                s.error.clear();
            }),
            Err(Stop::Wait(why)) => {
                let _ = store.session_retry(&row.id, now() + backoff(), false);
                set(|s| s.error = why);
                return;
            }
            Err(Stop::Fail(why)) => {
                let _ = store.session_retry(&row.id, now() + RETRY_S, true);
                set(|s| s.error = format!("{}: {why}", row.id.chars().take(8).collect::<String>()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_even_with_text_around() {
        assert_eq!(parse("sure: {\"a\": 1} done").unwrap()["a"], 1);
        assert!(parse("no json").is_none());
        let v = json!({"next": ["a", "b"], "why": "c"});
        assert_eq!((field(&v, "next").as_str(), field(&v, "why").as_str(), field(&v, "x").as_str()), ("a\nb", "c", ""));
        let s = schema();
        assert_eq!(s["properties"]["notes"]["maxItems"], 6);
        assert_eq!(s["properties"]["lessons"]["items"]["properties"]["kind"]["enum"][0], "mistake");
    }

    #[test]
    fn a_busy_model_is_asked_less_and_less_often() {
        let waits: Vec<f64> = (0..6).map(|_| backoff()).collect();
        assert_eq!(waits, [60.0, 120.0, 240.0, 480.0, 600.0, 600.0]);
        WAITS.store(0, Ordering::Relaxed);
        assert_eq!(backoff(), POLL_S);
    }
}
