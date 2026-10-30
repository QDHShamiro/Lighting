use rusqlite::{OptionalExtension, params};

use super::store::{Store, now};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Counts {
    pub notes: i64,
    pub vecs: i64,
    pub todos: i64,
    pub summaries: i64,
    pub queued: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Session {
    pub last: f64,
    pub title: String,
    pub summary: String,
    pub next: String,
}

impl Store {
    pub fn session_seen(
        &self,
        sid: &str,
        project: &str,
        cwd: &str,
        transcript: &str,
        ended: bool,
    ) -> rusqlite::Result<()> {
        if sid.is_empty() {
            return Ok(());
        }
        let t = now();
        self.db.execute(
            "insert into sessions(id, project, cwd, transcript, started, last) values (?1, ?2, ?3, ?4, ?5, ?5) \
             on conflict(id) do update set last = excluded.last, transcript = coalesce(nullif(excluded.transcript, ''), \
             sessions.transcript), status = case when sessions.status = 'done' then 'open' else sessions.status end",
            params![sid, project, cwd, transcript, t],
        )?;
        if ended {
            self.db.execute("update sessions set ended = 1 where id = ?1", [sid])?;
        }
        Ok(())
    }

    pub fn sessions(&self, project: &str, limit: usize) -> rusqlite::Result<Vec<Session>> {
        let mut sql = String::from("select last, title, summary, next from sessions where summary != ''");
        if !project.is_empty() {
            sql.push_str(" and project = ?2");
        }
        sql.push_str(" order by last desc limit ?1");
        let mut stmt = self.db.prepare(&sql)?;
        let map = |r: &rusqlite::Row| -> rusqlite::Result<Session> {
            Ok(Session {
                last: r.get::<_, Option<f64>>(0)?.unwrap_or(0.0),
                title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                summary: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                next: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            })
        };
        let limit = limit as i64;
        if project.is_empty() {
            stmt.query_map(params![limit], map)?.collect()
        } else {
            stmt.query_map(params![limit, project], map)?.collect()
        }
    }

    pub fn note_count(&self) -> rusqlite::Result<i64> {
        self.db.query_row("select count(*) from notes", [], |r| r.get(0))
    }

    pub fn counts(&self) -> rusqlite::Result<Counts> {
        self.db.query_row(
            "select (select count(*) from notes), (select count(*) from vecs), \
             (select count(*) from todos where status in ('todo', 'doing')), \
             (select count(*) from sessions where summary != ''), \
             (select count(*) from sessions where status = 'open' and transcript != '')",
            [],
            |r| {
                Ok(Counts {
                    notes: r.get(0)?,
                    vecs: r.get(1)?,
                    todos: r.get(2)?,
                    summaries: r.get(3)?,
                    queued: r.get(4)?,
                })
            },
        )
    }

    pub fn meta(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.db.query_row("select v from meta where k = ?1", [key], |r| r.get(0)).optional()
    }

    pub fn drafts(&self) -> rusqlite::Result<i64> {
        self.db.query_row("select count(*) from notes where source = 'vault' and kind = 'draft'", [], |r| r.get(0))
    }

    pub fn project_about(&self, project: &str) -> rusqlite::Result<String> {
        if project.is_empty() {
            return Ok(String::new());
        }
        let body: Option<Option<String>> = self
            .db
            .query_row(
                "select body from notes where source = 'vault' and kind = 'project' and project = ?1 \
                 order by updated desc limit 1",
                [project],
                |r| r.get(0),
            )
            .optional()?;
        Ok(body.flatten().unwrap_or_default())
    }

    pub fn titles(&self, project: &str, rules: bool, limit: usize) -> rusqlite::Result<Vec<(i64, String)>> {
        let sql = if rules {
            "select id, title from notes where kind = 'rule' and source = 'vault' and project = ?1 \
             order by updated desc limit ?2"
        } else {
            "select id, title from notes where source = 'user' and project in (?1, '') order by updated desc limit ?2"
        };
        if rules && project.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.db.prepare(sql)?;
        stmt.query_map(params![project, limit as i64], |r| {
            Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default()))
        })?
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::store::NewNote;
    use super::*;

    #[test]
    fn sessions_and_start_lists_come_from_the_db() {
        let dir = std::env::temp_dir().join(format!("ltm-start-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut s = Store::open(&dir).unwrap();
        s.session_seen("s1", "web", "C:/web", "", false).unwrap();
        s.session_seen("s1", "web", "C:/web", "C:/t.jsonl", true).unwrap();
        s.session_seen("", "web", "", "", false).unwrap();
        s.db.execute(
            "update sessions set summary = 'built the cart', title = 'Cart', next = 'tests' where id = 's1'",
            [],
        )
        .unwrap();
        let got = s.sessions("web", 3).unwrap();
        assert_eq!((got.len(), got[0].title.as_str(), got[0].next.as_str()), (1, "Cart", "tests"));
        assert!(s.sessions("other", 3).unwrap().is_empty() && s.sessions("", 3).unwrap().len() == 1);
        let ended: i64 = s.db.query_row("select ended from sessions where id = 's1'", [], |r| r.get(0)).unwrap();
        assert_eq!(ended, 1);
        let note = |project, title| NewNote { project, kind: "fact", title, body: "", source: "user", session: "" };
        s.add_note(&note("web", "Use pnpm")).unwrap();
        s.add_note(&note("", "Brave only")).unwrap();
        s.add_note(&note("api", "Elsewhere")).unwrap();
        let mine: Vec<String> = s.titles("web", false, 8).unwrap().into_iter().map(|(_, t)| t).collect();
        assert_eq!(mine, ["Brave only", "Use pnpm"]);
        assert!(s.titles("", true, 5).unwrap().is_empty());
        assert_eq!((s.note_count().unwrap(), s.drafts().unwrap()), (3, 0));
        let c = s.counts().unwrap();
        assert_eq!((c.notes, c.vecs, c.todos, c.summaries, c.queued), (3, 0, 0, 1, 1));
        assert_eq!(s.meta("vault_indexed").unwrap(), None);
        assert_eq!(s.project_about("web").unwrap(), "");
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }
}
