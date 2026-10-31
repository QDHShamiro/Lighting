use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, UNIX_EPOCH};

use regex::Regex;
use serde_json::{Value, json};
use wisply_core::{Call, Fail, local_parts};
use wisply_memory::{Note, links};

use super::super::ctx::Ctx;
use super::super::{Daemon, config};
use super::{QUERY_BUDGET, embed, index, with_store};

const USAGE: &str = "wisply board home | pulse | project <name> | note <id> | save <path> --mtime <ms> | todo <id> <status> | \
                     todo-add <project> <task> | search <words> | graph [project] | live | lessons | unlearn <sig> | settings";
const NOTES_SHOWN: usize = 400;
const RUNS_SHOWN: usize = 60;
const BACKLINKS: usize = 80;
const HITS: usize = 40;
const FILE_MAX: u64 = 4_000_000;
const LOG_TAIL: u64 = 256 * 1024;
const LIVE_S: f64 = 900.0;
const WORKING_S: f64 = 60.0;
const STATUSES: &[&str] = &["todo", "doing", "done", "dropped"];

static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)#([\p{L}\d_/-]*\p{L}[\p{L}\d_/-]*)").expect("tag pattern"));

fn mtime(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

fn note_json(n: &Note) -> Value {
    json!({"id": n.id, "title": n.title, "project": n.project, "kind": n.kind, "source": n.source,
           "updated": n.updated, "path": n.path().unwrap_or("")})
}

fn tags(body: &str) -> Vec<String> {
    let mut out: Vec<String> = TAG.captures_iter(body).map(|c| c[1].to_string()).collect();
    out.sort();
    out.dedup();
    out
}

fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let parent = path.parent()?.canonicalize().ok()?;
    let full = parent.join(path.file_name()?);
    (full.starts_with(&root) && full.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))).then_some(full)
}

fn pulse_view(d: &Daemon) -> Result<Value, Fail> {
    let (notes, queued, drafts) = with_store(d, |s| Ok((s.note_count()?, s.queued()?, s.drafts()?)))?;
    let live = live_rows(&d.home);
    let working = live.iter().filter(|r| r["age"].as_f64().is_some_and(|a| a <= WORKING_S)).count();
    Ok(json!({"pid": d.pid, "up": d.started.elapsed().as_secs(), "working": working, "queued": queued,
              "drafts": drafts, "notes": notes, "problem": index::problem()}))
}

fn home_view(d: &Daemon) -> Result<Value, Fail> {
    let cards = with_store(d, |s| s.cards())?;
    let mut out = pulse_view(d)?;
    out["projects"] = cards
        .iter()
        .map(|c| json!({"name": c.name, "notes": c.notes, "todos": c.todos, "sessions": c.sessions, "last": c.last, "title": c.title}))
        .collect();
    Ok(out)
}

fn project_view(d: &Daemon, name: &str) -> Result<Value, Fail> {
    let (notes, todos, runs, about) = with_store(d, |s| {
        Ok((
            s.notes_of(name, NOTES_SHOWN)?,
            s.todos(Some(name), false)?,
            s.runs(name, RUNS_SHOWN)?,
            s.project_about(name)?,
        ))
    })?;
    let todos: Vec<Value> = todos
        .iter()
        .map(|t| json!({"id": t.id, "task": t.task, "status": t.status, "list": t.list, "note": t.note, "updated": t.updated}))
        .collect();
    let runs: Vec<Value> = runs
        .iter()
        .map(|r| json!({"id": r.id, "title": r.title, "summary": r.summary, "next": r.next, "last": r.last, "status": r.status}))
        .collect();
    Ok(
        json!({"name": name, "about": about, "notes": notes.iter().map(note_json).collect::<Vec<_>>(), "todos": todos, "sessions": runs}),
    )
}

fn note_view(d: &Daemon, c: &Call) -> Result<Value, Fail> {
    let which = c.pos.get(1).map(String::as_str).unwrap_or("");
    let note = match (c.val("title"), which.parse::<i64>()) {
        (Some(t), _) => with_store(d, |s| s.note_titled(t))?,
        (None, Ok(id)) => with_store(d, |s| s.note(id))?,
        _ => return Err(Fail::hint("which note?", USAGE)),
    };
    let n = note.ok_or_else(|| Fail::hint("no such note", USAGE))?;
    let path = n.path().map(PathBuf::from).filter(|p| p.is_file());
    let (body, stamp) = match &path {
        Some(p) if fs::metadata(p).is_ok_and(|m| m.len() <= FILE_MAX) => {
            (fs::read_to_string(p).unwrap_or_else(|_| n.body.clone()), mtime(p))
        }
        _ => (n.body.clone(), 0),
    };
    let back = with_store(d, |s| s.backlinks(&n.title, BACKLINKS))?;
    let back: Vec<Value> =
        back.into_iter().filter(|(id, _)| *id != n.id).map(|(id, title)| json!({"id": id, "title": title})).collect();
    let editable = path.as_ref().is_some_and(|p| inside(&index::vault(&d.home), p).is_some());
    let mut out = note_json(&n);
    out["body"] = json!(body);
    out["mtime"] = json!(stamp);
    out["editable"] = json!(editable);
    out["links"] = json!(links(&body));
    out["tags"] = json!(tags(&body));
    out["backlinks"] = json!(back);
    Ok(out)
}

fn save(d: &Daemon, ctx: &Ctx, c: &Call) -> Result<Value, Fail> {
    let raw = c.pos.get(1).ok_or_else(|| Fail::hint("which file?", USAGE))?;
    let body = ctx.secret.clone().ok_or_else(|| Fail::hint("no text to save", USAGE))?;
    let path = inside(&index::vault(&d.home), Path::new(raw))
        .ok_or_else(|| Fail::new(format!("{raw} is not a Markdown file in the vault")))?;
    let want: u64 = c.val("mtime").and_then(|m| m.parse().ok()).unwrap_or(0);
    let now = mtime(&path);
    if now != want {
        return Ok(json!({"conflict": true, "mtime": now}));
    }
    let tmp = path.with_extension("md.wisply-tmp");
    fs::write(&tmp, body.as_bytes())
        .and_then(|()| fs::rename(&tmp, &path))
        .map_err(|e| Fail::new(format!("cannot save {} ({e})", path.display())))?;
    index::nudge();
    Ok(json!({"ok": true, "mtime": mtime(&path)}))
}

fn todo(d: &Daemon, c: &Call) -> Result<Value, Fail> {
    let id: i64 = c.pos.get(1).and_then(|p| p.parse().ok()).ok_or_else(|| Fail::hint("which to-do?", USAGE))?;
    let status = c.pos.get(2).map(String::as_str).filter(|s| STATUSES.contains(s));
    let status = status.ok_or_else(|| Fail::hint(format!("status is one of {}", STATUSES.join(", ")), USAGE))?;
    let task = c.val("task").filter(|t| !t.trim().is_empty());
    let t = with_store(d, |s| {
        if let Some(task) = task {
            s.todo_rename(id, task)?;
        }
        s.todo_set(id, Some(status), None)
    })?;
    let t = t.ok_or_else(|| Fail::new(format!("no to-do #{id}")))?;
    Ok(json!({"id": t.id, "status": t.status, "task": t.task}))
}

fn todo_add(d: &Daemon, c: &Call) -> Result<Value, Fail> {
    let project = c.pos.get(1).ok_or_else(|| Fail::hint("which project?", USAGE))?;
    let task = c.val("task").map_or_else(|| c.pos[2..].join(" "), String::from);
    if task.trim().is_empty() {
        return Err(Fail::hint("what to do?", USAGE));
    }
    let id = with_store(d, |s| s.todo_add(project, task.trim(), "", "todo"))?;
    Ok(json!({"id": id}))
}

fn search(d: &Daemon, c: &Call) -> Result<Value, Fail> {
    let text = c.val("words").map_or_else(|| c.pos[1..].join(" "), String::from);
    if text.trim().is_empty() {
        return Ok(json!({"hits": []}));
    }
    let vec = embed::embed(&d.home, std::slice::from_ref(&text), true, QUERY_BUDGET).ok().and_then(|mut v| v.pop());
    let scope = c.val("project").unwrap_or("");
    let hits = with_store(d, |s| s.search(&text, scope, HITS, vec.as_deref(), 0.0))?;
    let hits: Vec<Value> = hits
        .iter()
        .map(|h| {
            let mut n = note_json(&h.note);
            n["score"] = json!(h.score);
            n["snippet"] = json!(wisply_memory::oneline(&h.note.body, 160));
            n
        })
        .collect();
    Ok(json!({"hits": hits}))
}

fn graph(d: &Daemon, c: &Call) -> Result<Value, Fail> {
    let project = c.pos.get(1).map(String::as_str).filter(|p| !p.is_empty());
    let g = with_store(d, |s| s.graph(project))?;
    let nodes: Vec<Value> = g.nodes.iter().map(|n| json!([n.id, n.label, n.kind, n.group, n.t.round()])).collect();
    let edges: Vec<Value> = g.edges.iter().map(|(a, b)| json!([a, b])).collect();
    Ok(json!({"nodes": nodes, "edges": edges}))
}

fn stamp(t: f64) -> String {
    let [y, mo, d, h, mi, s, ..] = local_parts(t);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

fn seconds_since(t: &str, now: f64) -> Option<f64> {
    Some(now - wisply_core::local_to_utc(wisply_core::stamp_secs(t)?) as f64)
}

fn log_tail(home: &Path) -> String {
    let Ok(mut f) = fs::File::open(home.join("log.jsonl")) else { return String::new() };
    let len = f.metadata().map_or(0, |m| m.len());
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL)));
    let mut buf = Vec::new();
    let _ = f.take(LOG_TAIL).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn live_rows(home: &Path) -> Vec<Value> {
    let now = wisply_memory::now();
    let since = stamp(now - LIVE_S);
    let mut by: BTreeMap<String, (Value, usize)> = BTreeMap::new();
    for line in log_tail(home).lines() {
        let Ok(row) = serde_json::from_str::<Value>(line) else { continue };
        let t = row["t"].as_str().unwrap_or("");
        let sid = row["sid"].as_str().unwrap_or("").to_string();
        if t < since.as_str()
            || sid == wisply_proto::WINDOW_SID
            || wisply_core::QUIET.contains(&row["cmd"].as_str().unwrap_or(""))
        {
            continue;
        }
        let age = seconds_since(t, now).unwrap_or(LIVE_S);
        let entry = by.entry(sid.clone()).or_insert((Value::Null, 0));
        entry.1 += 1;
        entry.0 = json!({"sid": sid, "cmd": row["cmd"], "ref": row["ref"], "ok": row["ok"], "error": row["e"], "t": t, "age": age});
    }
    let mut rows: Vec<Value> = by
        .into_values()
        .map(|(mut v, n)| {
            v["count"] = json!(n);
            v
        })
        .collect();
    rows.sort_by(|a, b| a["age"].as_f64().unwrap_or(0.0).total_cmp(&b["age"].as_f64().unwrap_or(0.0)));
    rows
}

fn live(d: &Daemon) -> Result<Value, Fail> {
    let mut rows = live_rows(&d.home);
    let ids: Vec<String> = rows.iter().filter_map(|r| r["sid"].as_str().map(String::from)).collect();
    let titles = with_store(d, |s| s.run_titles(&ids))?;
    for r in &mut rows {
        if let Some((project, title)) = r["sid"].as_str().and_then(|s| titles.get(s)) {
            r["project"] = json!(project);
            r["title"] = json!(title);
        }
    }
    Ok(json!({"sessions": rows, "working_s": WORKING_S, "poll_ms": Duration::from_secs(2).as_millis() as u64}))
}

fn lessons(d: &Daemon) -> Result<Value, Fail> {
    let all = with_store(d, |s| s.lessons())?;
    let rows: Vec<Value> = all
        .iter()
        .map(|l| json!({"sig": l.sig, "cmd": l.cmd, "target": l.target, "kind": l.kind, "fix": l.fix, "ups": l.ups, "downs": l.downs}))
        .collect();
    Ok(json!({"lessons": rows}))
}

fn unlearn(d: &Daemon, c: &Call) -> Result<Value, Fail> {
    let sig = c.pos[1..].join(" ");
    if sig.is_empty() {
        return Err(Fail::hint("which fix?", USAGE));
    }
    let dropped = with_store(d, |s| s.unlearn(&sig))?;
    Ok(json!({"dropped": dropped}))
}

pub fn board(d: &Daemon, ctx: &Ctx, c: &Call) -> Result<String, Fail> {
    let what = c.pos.first().map(String::as_str).unwrap_or("");
    let out = match what {
        "home" => home_view(d)?,
        "pulse" => pulse_view(d)?,
        "project" => project_view(d, &c.pos[1..].join(" "))?,
        "note" => note_view(d, c)?,
        "save" => save(d, ctx, c)?,
        "todo" => todo(d, c)?,
        "todo-add" => todo_add(d, c)?,
        "search" => search(d, c)?,
        "graph" => graph(d, c)?,
        "live" => live(d)?,
        "lessons" => lessons(d)?,
        "unlearn" => unlearn(d, c)?,
        "settings" => Value::Object(config::load(&d.home)),
        _ => return Err(Fail::hint("what should the board show?", USAGE)),
    };
    Ok(out.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_vault_paths_stay_safe() {
        assert_eq!(tags("a #rust and #2026 #plan/v3\n#rust"), ["plan/v3", "rust"]);
        let root = std::env::temp_dir().join(format!("lt-board-vault-{}", std::process::id()));
        fs::create_dir_all(root.join("sub")).unwrap();
        assert!(inside(&root, &root.join("sub/a.md")).is_some());
        assert!(inside(&root, &root.join("sub/a.txt")).is_none());
        assert!(inside(&root, &root.join("../evil.md")).is_none());
        assert!(inside(&root.join("sub"), &root.join("a.md")).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn log_times_turn_into_ages() {
        let now = wisply_memory::now();
        let age = seconds_since(&stamp(now - 30.0), now).unwrap();
        assert!((29.0..=31.0).contains(&age), "{age}");
        assert_eq!(seconds_since("garbage", now), None);
    }
}
