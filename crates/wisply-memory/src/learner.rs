use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};

use super::store::{NewNote, Store, now};
use super::text::{oneline, scrub, similar, title_words};

const MAX_TRIES: i64 = 3;
const DUP_SIM: f64 = 0.7;
const TITLE_CHARS: usize = 200;
const RECORD_LINES: usize = 24;
const FILES_JSON: usize = 2000;
const FILE_SOURCES: &[&str] = &["vault", "claude"];
const EMPTY: &[&str] = &["none", "nothing", "n/a", "na", "keine", "keiner", "nichts", "-", "--"];
const STILL_OPEN: &str = "case when last > ?2 then 'open' else 'done' end";

#[derive(Debug, Clone, Default)]
pub struct Due {
    pub id: String,
    pub project: String,
    pub transcript: String,
    pub offset: u64,
    pub started: f64,
    pub title: String,
    pub summary: String,
    pub why: String,
    pub rejected: String,
    pub next: String,
}

#[derive(Debug, Clone, Default)]
pub struct Record {
    pub title: String,
    pub summary: String,
    pub why: String,
    pub rejected: String,
    pub next: String,
}

fn opt(r: &rusqlite::Row, i: usize) -> rusqlite::Result<String> {
    Ok(r.get::<_, Option<String>>(i)?.unwrap_or_default())
}

pub fn titled(project: &str, title: &str) -> String {
    let title = title.trim();
    if project.is_empty() || title.to_lowercase().starts_with(&project.to_lowercase()) {
        title.to_string()
    } else {
        format!("{project}: {title}")
    }
}

fn point(line: &str) -> String {
    line.trim().trim_start_matches(['-', '*', ' ']).trim().to_string()
}

pub fn points(old: &str, new: &str) -> String {
    let mut lines: Vec<String> = old.lines().filter(|x| !x.trim().is_empty()).map(point).collect();
    let mut seen: HashSet<Vec<String>> = lines.iter().map(|x| key(x)).collect();
    for raw in new.lines() {
        let x = scrub(&point(raw));
        let k = key(&x);
        if !x.is_empty() && !EMPTY.contains(&x.trim_end_matches(['.', '!']).to_lowercase().as_str()) && seen.insert(k) {
            lines.push(x);
        }
    }
    let skip = lines.len().saturating_sub(RECORD_LINES);
    lines[skip..].join("\n")
}

fn key(text: &str) -> Vec<String> {
    let mut words: Vec<String> = title_words(text).into_iter().collect();
    words.sort();
    words
}

impl Store {
    pub fn due_sessions(&self, at: f64, quiet: f64) -> rusqlite::Result<Vec<Due>> {
        let mut stmt = self.db.prepare(
            "select id, project, transcript, offset, started, title, summary, why, rejected, next from sessions \
             where status = 'open' and transcript != '' and tries < ?1 and retry_at <= ?2 \
             and (ended = 1 or last < ?3) order by last desc",
        )?;
        let rows = stmt.query_map(params![MAX_TRIES, at, at - quiet], |r| {
            Ok(Due {
                id: r.get(0)?,
                project: opt(r, 1)?,
                transcript: opt(r, 2)?,
                offset: r.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) as u64,
                started: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                title: opt(r, 5)?,
                summary: opt(r, 6)?,
                why: opt(r, 7)?,
                rejected: opt(r, 8)?,
                next: opt(r, 9)?,
            })
        })?;
        rows.collect()
    }

    pub fn session_project(&self, sid: &str, project: &str) -> rusqlite::Result<()> {
        self.db.execute("update sessions set project = ?2 where id = ?1", params![sid, project]).map(|_| ())
    }

    pub fn session_gone(&self, sid: &str) -> rusqlite::Result<()> {
        self.db.execute("update sessions set status = 'done' where id = ?1", [sid]).map(|_| ())
    }

    pub fn session_short(&self, sid: &str, end: u64, began: f64, title: &str) -> rusqlite::Result<()> {
        let sql = format!(
            "update sessions set offset = ?3, learned = ?4, status = {STILL_OPEN}, \
             title = coalesce(nullif(?5, ''), title) where id = ?1"
        );
        self.db.execute(&sql, params![sid, began, end as i64, now(), title]).map(|_| ())
    }

    pub fn session_learned(&self, sid: &str, end: u64, open_since: f64, r: &Record) -> rusqlite::Result<()> {
        let sql = format!(
            "update sessions set offset = ?3, learned = ?4, status = {STILL_OPEN}, tries = 0, title = ?5, \
             summary = ?6, why = ?7, rejected = ?8, next = ?9 where id = ?1"
        );
        let title = oneline(&r.title, 120);
        let args = params![sid, open_since, end as i64, now(), title, r.summary, r.why, r.rejected, r.next];
        self.db.execute(&sql, args).map(|_| ())
    }

    pub fn session_retry(&self, sid: &str, at: f64, failed: bool) -> rusqlite::Result<()> {
        let sql = if failed {
            "update sessions set tries = tries + 1, retry_at = ?2 where id = ?1"
        } else {
            "update sessions set retry_at = ?2 where id = ?1"
        };
        self.db.execute(sql, params![sid, at]).map(|_| ())
    }

    pub fn sessions_retry_all(&self) -> rusqlite::Result<usize> {
        self.db.execute("update sessions set retry_at = 0, tries = 0 where status = 'open' and transcript != ''", [])
    }

    pub fn merge_note(&mut self, n: &NewNote, files: &[String]) -> rusqlite::Result<Option<(i64, bool)>> {
        for hit in self.search(n.title, n.project, 3, None, 0.0)? {
            let r = hit.note;
            if r.project == n.project
                && !FILE_SOURCES.contains(&r.source.as_str())
                && similar(&r.title, n.title) >= DUP_SIM
            {
                let body = if n.body.chars().count() > r.body.chars().count() { n.body } else { r.body.as_str() };
                let kind = if n.kind.is_empty() { r.kind.as_str() } else { n.kind };
                self.db.execute(
                    "update notes set title = ?2, body = ?3, kind = ?4, updated = ?5, session = ?6 where id = ?1",
                    params![r.id, oneline(&scrub(n.title), TITLE_CHARS), scrub(body), kind, now(), n.session],
                )?;
                self.vecs = None;
                return Ok(Some((r.id, false)));
            }
        }
        let Some(id) = self.add_note(n)? else { return Ok(None) };
        let json = serde_json::to_string(files).unwrap_or_else(|_| "[]".into());
        let json: String = json.chars().take(FILES_JSON).collect();
        self.db.execute("update notes set files = ?2 where id = ?1", params![id, json])?;
        Ok(Some((id, true)))
    }

    pub fn session_note(&mut self, sid: &str, project: &str, r: &Record) -> rusqlite::Result<Option<i64>> {
        let origin = format!("s:{sid}");
        let body = if r.next.is_empty() { r.summary.clone() } else { format!("{}\nNext: {}", r.summary, r.next) };
        let title = if r.title.is_empty() { "session" } else { r.title.as_str() };
        let old: Option<i64> =
            self.db.query_row("select id from notes where origin = ?1", [&origin], |row| row.get(0)).optional()?;
        if let Some(id) = old {
            self.db.execute(
                "update notes set title = ?2, body = ?3, updated = ?4 where id = ?1",
                params![id, oneline(title, TITLE_CHARS), scrub(&body), now()],
            )?;
            self.vecs = None;
            return Ok(Some(id));
        }
        let new = NewNote { project, kind: "session", title, body: &body, source: "session", session: sid };
        let Some(id) = self.add_note(&new)? else { return Ok(None) };
        self.db.execute("update notes set origin = ?2 where id = ?1", params![id, origin])?;
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn learned_sessions_merge_notes_and_keep_records() {
        let dir = std::env::temp_dir().join(format!("ltm-learner-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut s = Store::open(&dir).unwrap();
        s.session_seen("s1", "web", "C:/web", "C:/t.jsonl", true).unwrap();
        s.session_seen("s2", "web", "C:/web", "", true).unwrap();
        let due = s.due_sessions(now(), 600.0).unwrap();
        assert_eq!(due.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["s1"]);
        let note =
            |title, body| NewNote { project: "web", kind: "fact", title, body, source: "session", session: "s1" };
        let (a, new) = s.merge_note(&note("web: cart uses redis", "short"), &["C:/web/a.rs".into()]).unwrap().unwrap();
        assert!(new);
        let (b, new) = s.merge_note(&note("web: the cart uses redis", "a longer body"), &[]).unwrap().unwrap();
        assert_eq!((b, new), (a, false));
        let kept = s.note(a).unwrap().unwrap();
        assert_eq!((kept.body.as_str(), kept.files.as_str()), ("a longer body", r#"["C:/web/a.rs"]"#));
        let rec =
            Record { title: "Cart".into(), summary: "built it".into(), next: "tests".into(), ..Record::default() };
        let id = s.session_note("s1", "web", &rec).unwrap().unwrap();
        assert_eq!(s.session_note("s1", "web", &rec).unwrap(), Some(id));
        assert_eq!(s.note(id).unwrap().unwrap().body, "built it\nNext: tests");
        s.session_learned("s1", 42, now() + 1.0, &rec).unwrap();
        assert!(s.due_sessions(now(), 600.0).unwrap().is_empty());
        s.session_retry("s2", 0.0, true).unwrap();
        let got = points("- alpha one\n- beta two", "beta two\ngamma three\nnone\n* delta four");
        assert_eq!(got, "alpha one\nbeta two\ngamma three\ndelta four");
        assert_eq!(titled("web", "Cart"), "web: Cart");
        assert_eq!(titled("web", "Web: Cart"), "Web: Cart");
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }
}
