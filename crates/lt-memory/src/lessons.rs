use rusqlite::{OptionalExtension, params};

use super::store::{Store, now};

const MISSES_OUT: i64 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Lesson {
    pub sig: String,
    pub cmd: String,
    pub target: String,
    pub kind: String,
    pub fix: String,
    pub ups: i64,
    pub downs: i64,
}

impl Store {
    pub fn lesson(&self, sig: &str) -> rusqlite::Result<Option<Lesson>> {
        self.db
            .query_row("select sig, cmd, target, kind, fix, ups, downs from lessons where sig = ?1", [sig], |r| {
                Ok(Lesson {
                    sig: r.get(0)?,
                    cmd: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    target: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    kind: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    fix: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    ups: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    downs: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                })
            })
            .optional()
    }

    pub fn learn(&self, sig: &str, cmd: &str, target: &str, kind: &str, fix: &str) -> rusqlite::Result<()> {
        let t = now();
        self.db.execute(
            "insert into lessons(sig, cmd, target, kind, fix, ups, downs, created, updated) \
             values (?1, ?2, ?3, ?4, ?5, 1, 0, ?6, ?6) on conflict(sig) do update set \
             ups = case when lessons.fix = excluded.fix then lessons.ups + 1 else 1 end, \
             downs = case when lessons.fix = excluded.fix then lessons.downs else 0 end, \
             fix = excluded.fix, updated = excluded.updated",
            params![sig, cmd, target, kind, fix, t],
        )?;
        Ok(())
    }

    pub fn vote(&self, sig: &str, good: bool) -> rusqlite::Result<bool> {
        let sql = if good {
            "update lessons set ups = ups + 1, downs = 0, updated = ?2 where sig = ?1"
        } else {
            "update lessons set downs = downs + 1, updated = ?2 where sig = ?1"
        };
        self.db.execute(sql, params![sig, now()])?;
        let gone = self.db.execute("delete from lessons where sig = ?1 and downs >= ?2", params![sig, MISSES_OUT])?;
        Ok(gone > 0)
    }

    pub fn lessons(&self) -> rusqlite::Result<Vec<Lesson>> {
        let mut stmt = self.db.prepare("select sig from lessons order by updated desc")?;
        let sigs: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(sigs.iter().filter_map(|s| self.lesson(s).ok().flatten()).collect())
    }

    pub fn unlearn(&self, sig: &str) -> rusqlite::Result<usize> {
        self.db.execute("delete from lessons where sig = ?1", [sig])
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn lessons_rise_and_drop_out_after_two_misses() {
        let dir = std::env::temp_dir().join(format!("ltm-lessons-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let s = Store::open(&dir).unwrap();
        let sig = "click|example.com|element e# not found";
        assert!(s.lesson(sig).unwrap().is_none());
        s.learn(sig, "click", "example.com", "element e# not found", "snap").unwrap();
        s.learn(sig, "click", "example.com", "element e# not found", "snap").unwrap();
        assert_eq!(s.lesson(sig).unwrap().map(|l| (l.fix, l.ups)), Some(("snap".into(), 2)));
        s.learn(sig, "click", "example.com", "element e# not found", "scroll down").unwrap();
        assert_eq!(s.lesson(sig).unwrap().map(|l| (l.fix, l.ups)), Some(("scroll down".into(), 1)));
        assert!(!s.vote(sig, false).unwrap());
        assert!(s.vote(sig, false).unwrap());
        assert!(s.lesson(sig).unwrap().is_none() && s.lessons().unwrap().is_empty());
        s.learn("a|b|c", "a", "b", "c", "x").unwrap();
        assert_eq!(s.lessons().unwrap().len(), 1);
        assert_eq!(s.unlearn("a|b|c").unwrap(), 1);
        drop(s);
        let _ = fs::remove_dir_all(&dir);
    }
}
