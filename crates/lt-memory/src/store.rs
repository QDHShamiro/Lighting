use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use half::f16;
use lt_core::local_parts;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use super::text::{oneline, scrub};

pub const VEC_DIM: usize = 256;
const RRF_K: f64 = 60.0;
const POOL: usize = 60;
const PROJECT_BOOST: f64 = 1.3;
const USER_BOOST: f64 = 1.2;
const FRESH_BOOST: f64 = 0.25;
const HALF_LIFE_S: f64 = 90.0 * 86_400.0;
const BUSY: Duration = Duration::from_secs(15);
const TITLE_CHARS: usize = 200;
const TASK_CHARS: usize = 300;
const FILE_SOURCES: &[&str] = &["vault", "claude"];
const SCHEMA: &str = "
create table if not exists vecs(id integer primary key, v blob);
create table if not exists notes(id integer primary key, project text, kind text, title text, body text,
  files text default '', source text, session text default '', created real, updated real, hits integer default 0,
  origin text default '');
create index if not exists notes_project on notes(project, updated);
create index if not exists notes_origin on notes(origin);
create virtual table if not exists notes_fts using fts5(title, body, content='notes', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2', prefix='3');
create trigger if not exists notes_ai after insert on notes begin
  insert into notes_fts(rowid, title, body) values (new.id, new.title, new.body); end;
create trigger if not exists notes_ad after delete on notes begin
  insert into notes_fts(notes_fts, rowid, title, body) values ('delete', old.id, old.title, old.body);
  delete from vecs where id = old.id; end;
create trigger if not exists notes_au after update of title, body on notes begin
  insert into notes_fts(notes_fts, rowid, title, body) values ('delete', old.id, old.title, old.body);
  insert into notes_fts(rowid, title, body) values (new.id, new.title, new.body);
  delete from vecs where id = old.id; end;
create table if not exists sessions(id text primary key, project text, cwd text, transcript text default '',
  started real, last real, ended integer default 0, offset integer default 0, learned real default 0,
  title text default '', summary text default '', next text default '', status text default 'open',
  tries integer default 0, retry_at real default 0, source text default 'live', why text default '',
  rejected text default '');
create index if not exists sessions_project on sessions(project, last);
create table if not exists todos(id integer primary key, project text, list text default '', task text,
  status text default 'todo', note text default '', created real, updated real, origin text default '');
create index if not exists todos_project on todos(project, status);
create table if not exists meta(k text primary key, v text);
";
const MIGRATE: &[&str] = &[
    "alter table sessions add column why text default ''",
    "alter table sessions add column rejected text default ''",
];

#[derive(Debug, Clone, Default)]
pub struct Note {
    pub id: i64,
    pub project: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub files: String,
    pub source: String,
    pub session: String,
    pub created: f64,
    pub updated: f64,
    pub hits: i64,
    pub origin: String,
}

impl Note {
    pub fn path(&self) -> Option<&str> {
        if FILE_SOURCES.contains(&self.source.as_str()) { self.origin.split_once(':').map(|(_, p)| p) } else { None }
    }
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub note: Note,
    pub score: f64,
    pub sim: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct Todo {
    pub id: i64,
    pub project: String,
    pub list: String,
    pub task: String,
    pub status: String,
    pub note: String,
    pub created: f64,
    pub updated: f64,
}

pub struct NewNote<'a> {
    pub project: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub source: &'a str,
    pub session: &'a str,
}

pub struct Store {
    db: Connection,
    vecs: Option<(i64, Vec<i64>, Vec<f32>)>,
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

pub fn day(t: f64) -> String {
    let [_, m, d, ..] = local_parts(t);
    format!("{m:02}-{d:02}")
}

pub fn date(t: f64) -> String {
    let [y, m, d, ..] = local_parts(t);
    format!("{y:04}-{m:02}-{d:02}")
}

fn text(row: &Row, key: &str) -> String {
    row.get::<_, Option<String>>(key).ok().flatten().unwrap_or_default()
}

fn real(row: &Row, key: &str) -> f64 {
    row.get::<_, Option<f64>>(key).ok().flatten().unwrap_or(0.0)
}

fn note_of(row: &Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get("id")?,
        project: text(row, "project"),
        kind: text(row, "kind"),
        title: text(row, "title"),
        body: text(row, "body"),
        files: text(row, "files"),
        source: text(row, "source"),
        session: text(row, "session"),
        created: real(row, "created"),
        updated: real(row, "updated"),
        hits: row.get::<_, Option<i64>>("hits")?.unwrap_or(0),
        origin: text(row, "origin"),
    })
}

fn todo_of(row: &Row) -> rusqlite::Result<Todo> {
    Ok(Todo {
        id: row.get("id")?,
        project: text(row, "project"),
        list: text(row, "list"),
        task: text(row, "task"),
        status: text(row, "status"),
        note: text(row, "note"),
        created: real(row, "created"),
        updated: real(row, "updated"),
    })
}

pub fn halves(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| f16::from_f32(*x).to_le_bytes()).collect()
}

fn floats(raw: &[u8]) -> impl Iterator<Item = f32> + '_ {
    raw.as_chunks::<2>().0.iter().map(|b| f16::from_le_bytes(*b).to_f32())
}

impl Store {
    pub fn open(home: &Path) -> rusqlite::Result<Store> {
        let dir = home.join("memory");
        let _ = fs::create_dir_all(&dir);
        let db = Connection::open(dir.join("memory.db"))?;
        db.busy_timeout(BUSY)?;
        db.pragma_update(None, "journal_mode", "wal")?;
        db.pragma_update(None, "synchronous", "normal")?;
        db.execute_batch(SCHEMA)?;
        for sql in MIGRATE {
            let _ = db.execute(sql, []);
        }
        Ok(Store { db, vecs: None })
    }

    pub fn note(&self, id: i64) -> rusqlite::Result<Option<Note>> {
        self.db.query_row("select * from notes where id = ?1", [id], note_of).optional()
    }

    pub fn add_note(&mut self, n: &NewNote) -> rusqlite::Result<Option<i64>> {
        let title = oneline(&scrub(n.title), TITLE_CHARS);
        if title.is_empty() {
            return Ok(None);
        }
        let body = scrub(n.body).trim().to_string();
        let t = now();
        self.db.execute(
            "insert into notes(project, kind, title, body, files, source, session, created, updated, origin) \
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, '')",
            params![
                n.project,
                if n.kind.is_empty() { "note" } else { n.kind },
                title,
                body,
                "[]",
                n.source,
                n.session,
                t
            ],
        )?;
        Ok(Some(self.db.last_insert_rowid()))
    }

    pub fn forget(&mut self, id: i64) -> rusqlite::Result<Result<usize, String>> {
        if let Some(n) = self.note(id)?
            && let Some(path) = n.path()
        {
            return Ok(Err(format!("#{id} is the file {path}: edit or delete the file itself")));
        }
        let gone = self.db.execute("delete from notes where id = ?1", [id])?;
        self.vecs = None;
        Ok(Ok(gone))
    }

    pub fn touch(&self, ids: &[i64]) -> rusqlite::Result<()> {
        let mut stmt = self.db.prepare_cached("update notes set hits = hits + 1 where id = ?1")?;
        for id in ids {
            stmt.execute([id])?;
        }
        Ok(())
    }

    pub fn save_vecs(&mut self, pairs: &[(i64, Vec<f32>)]) -> rusqlite::Result<()> {
        let tx = self.db.transaction()?;
        for (id, v) in pairs {
            tx.execute("insert or replace into vecs(id, v) values (?1, ?2)", params![id, halves(v)])?;
        }
        tx.commit()?;
        self.vecs = None;
        Ok(())
    }

    fn matrix(&mut self) -> rusqlite::Result<(&[i64], &[f32])> {
        let version: i64 = self.db.query_row("pragma data_version", [], |r| r.get(0))?;
        let stale = self.vecs.as_ref().is_none_or(|(v, ..)| *v != version);
        if stale {
            let mut ids = Vec::new();
            let mut mat = Vec::new();
            let mut stmt = self.db.prepare("select id, v from vecs")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let raw: Vec<u8> = row.get::<_, Option<Vec<u8>>>(1)?.unwrap_or_default();
                if raw.len() >= VEC_DIM * 2 {
                    ids.push(row.get(0)?);
                    mat.extend(floats(&raw[..VEC_DIM * 2]));
                }
            }
            self.vecs = Some((version, ids, mat));
        }
        Ok(self.vecs.as_ref().map_or((&[][..], &[][..]), |(_, ids, mat)| (ids.as_slice(), mat.as_slice())))
    }

    pub fn search(
        &mut self,
        query: &str,
        project: &str,
        limit: usize,
        vec: Option<&[f32]>,
    ) -> rusqlite::Result<Vec<Hit>> {
        let mut score: Vec<(i64, f64)> = Vec::new();
        let mut add = |id: i64, rank: usize| match score.iter_mut().find(|(i, _)| *i == id) {
            Some((_, s)) => *s += 1.0 / (RRF_K + rank as f64 + 1.0),
            None => score.push((id, 1.0 / (RRF_K + rank as f64 + 1.0))),
        };
        let fq = super::text::fts_query(query);
        if !fq.is_empty()
            && let Ok(mut stmt) = self.db.prepare(
                "select rowid from notes_fts where notes_fts match ?1 order by bm25(notes_fts, 3.0, 1.0) limit 60",
            )
            && let Ok(ids) =
                stmt.query_map([&fq], |r| r.get::<_, i64>(0)).and_then(Iterator::collect::<Result<Vec<_>, _>>)
        {
            for (rank, id) in ids.into_iter().enumerate() {
                add(id, rank);
            }
        }
        let mut sims: Vec<(i64, f32)> = Vec::new();
        if let Some(q) = vec.filter(|q| q.len() == VEC_DIM) {
            let (ids, mat) = self.matrix()?;
            let mut all: Vec<(f32, i64)> = mat
                .as_chunks::<VEC_DIM>()
                .0
                .iter()
                .zip(ids)
                .map(|(row, id)| (row.iter().zip(q).map(|(a, b)| a * b).sum(), *id))
                .collect();
            all.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
            for (rank, (sim, id)) in all.into_iter().take(POOL).take_while(|(sim, _)| *sim >= 0.0).enumerate() {
                sims.push((id, sim));
                add(id, rank);
            }
        }
        if score.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; score.len()].join(",");
        let mut stmt = self.db.prepare(&format!("select * from notes where id in ({marks})"))?;
        let notes = stmt.query_map(params_from_iter(score.iter().map(|(id, _)| id)), note_of)?;
        let t = now();
        let mut out = Vec::new();
        for note in notes {
            let note = note?;
            let mut s = score.iter().find(|(id, _)| *id == note.id).map_or(0.0, |(_, s)| *s);
            if !project.is_empty() && note.project == project {
                s *= PROJECT_BOOST;
            }
            if note.source == "user" {
                s *= USER_BOOST;
            }
            s *= 1.0 + FRESH_BOOST * 0.5f64.powf((t - note.updated).max(0.0) / HALF_LIFE_S);
            let sim = sims.iter().find(|(id, _)| *id == note.id).map(|(_, v)| *v);
            out.push(Hit { note, score: s, sim });
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(limit);
        Ok(out)
    }

    pub fn todo(&self, id: i64) -> rusqlite::Result<Option<Todo>> {
        self.db.query_row("select * from todos where id = ?1", [id], todo_of).optional()
    }

    pub fn todo_add(&self, project: &str, task: &str, list: &str, status: &str) -> rusqlite::Result<i64> {
        let t = now();
        self.db.execute(
            "insert into todos(project, list, task, status, note, created, updated, origin) \
             values (?1, ?2, ?3, ?4, '', ?5, ?5, '')",
            params![project, list, oneline(task, TASK_CHARS), status, t],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    pub fn todo_set(&self, id: i64, status: Option<&str>, note: Option<&str>) -> rusqlite::Result<Option<Todo>> {
        let Some(old) = self.todo(id)? else { return Ok(None) };
        let mut text = old.note.clone();
        if let Some(add) = note.map(str::trim).filter(|n| !n.is_empty()) {
            let [_, m, d, h, mi, _] = local_parts(now());
            let words = add.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("{m:02}-{d:02} {h:02}:{mi:02} {words}"));
        }
        self.db.execute(
            "update todos set status = ?1, note = ?2, updated = ?3 where id = ?4",
            params![status.unwrap_or(&old.status), text, now(), id],
        )?;
        self.todo(id)
    }

    pub fn todos(&self, project: Option<&str>, open_only: bool) -> rusqlite::Result<Vec<Todo>> {
        let mut sql = String::from("select * from todos where 1 = 1");
        if project.is_some() {
            sql.push_str(" and project = ?1");
        }
        if open_only {
            sql.push_str(" and status in ('todo', 'doing')");
        }
        sql.push_str(" order by list, status = 'doing' desc, id");
        let mut stmt = self.db.prepare(&sql)?;
        let rows = match project {
            Some(p) => stmt.query_map([p], todo_of)?.collect::<Result<Vec<_>, _>>()?,
            None => stmt.query_map([], todo_of)?.collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ltm-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn unit(seed: usize) -> Vec<f32> {
        let mut v = vec![0.0; VEC_DIM];
        v[seed] = 1.0;
        v
    }

    #[test]
    fn notes_are_found_by_words_and_vectors() {
        let dir = home("notes");
        let mut s = Store::open(&dir).unwrap();
        let new = |title: &'static str, project: &'static str| NewNote {
            project,
            kind: "fact",
            title,
            body: "",
            source: "user",
            session: "s1",
        };
        let a = s.add_note(&new("Brave is the only browser", "lighting")).unwrap().unwrap();
        let b = s.add_note(&new("Deploy with ship.sh and token: abcdefgh1", "other")).unwrap().unwrap();
        assert_eq!(s.add_note(&new("   ", "x")).unwrap(), None);
        assert_eq!(s.note(b).unwrap().unwrap().title, "Deploy with ship.sh and token: [secret]");
        let hits = s.search("which browser", "lighting", 8, None).unwrap();
        assert_eq!(hits.iter().map(|h| h.note.id).collect::<Vec<_>>(), [a]);
        s.save_vecs(&[(a, unit(1)), (b, unit(2))]).unwrap();
        let hits = s.search("zzz", "", 8, Some(&unit(2))).unwrap();
        assert_eq!(hits[0].note.id, b);
        assert!(hits[0].sim.is_some_and(|v| (v - 1.0).abs() < 1e-3));
        s.touch(&[a]).unwrap();
        assert_eq!(s.note(a).unwrap().unwrap().hits, 1);
        assert_eq!(s.forget(a).unwrap(), Ok(1));
        assert!(s.note(a).unwrap().is_none() && s.search("browser", "", 8, None).unwrap().is_empty());
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn todos_keep_status_and_notes() {
        let dir = home("todos");
        let s = Store::open(&dir).unwrap();
        let one = s.todo_add("p", "write the spec", "", "todo").unwrap();
        let two = s.todo_add("p", "ship it", "release", "doing").unwrap();
        s.todo_add("q", "elsewhere", "", "todo").unwrap();
        let open: Vec<i64> = s.todos(Some("p"), true).unwrap().iter().map(|t| t.id).collect();
        assert_eq!(open, [one, two]);
        let done = s.todo_set(one, Some("done"), Some("  all   good ")).unwrap().unwrap();
        assert!(done.status == "done" && done.note.ends_with(" all good"), "{done:?}");
        assert_eq!(s.todos(Some("p"), true).unwrap().len(), 1);
        assert_eq!(s.todos(None, false).unwrap().len(), 3);
        assert!(s.todo_set(99, Some("done"), None).unwrap().is_none());
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }
}
