use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{Row, params};

use super::store::{Note, Store, note_of, real, text};

static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[([^\]|#\n]+)").expect("link pattern"));
const HUB: &str = "p:";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Card {
    pub name: String,
    pub notes: i64,
    pub todos: i64,
    pub sessions: i64,
    pub last: f64,
    pub title: String,
}

#[derive(Debug, Clone, Default)]
pub struct Run {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub next: String,
    pub last: f64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: &'static str,
    pub group: String,
    pub t: f64,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<(usize, usize)>,
}

pub fn links(body: &str) -> Vec<String> {
    LINK.captures_iter(body).map(|c| c[1].trim().to_string()).filter(|l| !l.is_empty()).collect()
}

fn run_of(row: &Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: text(row, "id"),
        title: text(row, "title"),
        summary: text(row, "summary"),
        next: text(row, "next"),
        last: real(row, "last"),
        status: text(row, "status"),
    })
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

impl Graph {
    fn add(&mut self, index: &mut HashMap<String, usize>, node: Node) -> usize {
        if let Some(i) = index.get(&node.id) {
            return *i;
        }
        index.insert(node.id.clone(), self.nodes.len());
        self.nodes.push(node);
        self.nodes.len() - 1
    }
}

impl Store {
    pub fn cards(&self) -> rusqlite::Result<Vec<Card>> {
        let mut stmt = self.db.prepare(
            "with p as (select project, count(*) n, 0 t, 0 s, max(updated) last from notes where project != '' group by project \
             union all select project, 0, count(*), 0, max(updated) from todos \
               where project != '' and status in ('todo', 'doing') group by project \
             union all select project, 0, 0, count(*), max(last) from sessions where project != '' group by project) \
             select project, sum(n), sum(t), sum(s), max(last) from p group by project order by max(last) desc",
        )?;
        let mut cards: Vec<Card> = stmt
            .query_map([], |r| {
                Ok(Card {
                    name: r.get(0)?,
                    notes: r.get(1)?,
                    todos: r.get(2)?,
                    sessions: r.get(3)?,
                    last: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                    title: String::new(),
                })
            })?
            .collect::<Result<_, _>>()?;
        let mut titles = self.db.prepare(
            "select project, title from sessions s where title != '' and last = \
             (select max(last) from sessions where project = s.project and title != '')",
        )?;
        let map: HashMap<String, String> =
            titles.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        for c in &mut cards {
            c.title = map.get(&c.name).cloned().unwrap_or_default();
        }
        Ok(cards)
    }

    pub fn notes_of(&self, project: &str, limit: usize) -> rusqlite::Result<Vec<Note>> {
        let mut stmt = self.db.prepare("select * from notes where project = ?1 order by updated desc limit ?2")?;
        stmt.query_map(params![project, limit as i64], note_of)?.collect()
    }

    pub fn runs(&self, project: &str, limit: usize) -> rusqlite::Result<Vec<Run>> {
        let mut stmt = self.db.prepare("select * from sessions where project = ?1 order by last desc limit ?2")?;
        stmt.query_map(params![project, limit as i64], run_of)?.collect()
    }

    pub fn note_titled(&self, title: &str) -> rusqlite::Result<Option<Note>> {
        let mut stmt = self.db.prepare(
            "select * from notes where lower(title) = lower(?1) order by source = 'vault' desc, updated desc limit 1",
        )?;
        stmt.query_map([title.trim()], note_of)?.next().transpose()
    }

    pub fn todo_rename(&self, id: i64, task: &str) -> rusqlite::Result<usize> {
        self.db.execute(
            "update todos set task = ?2, updated = ?3 where id = ?1",
            params![id, super::text::oneline(&super::text::scrub(task), super::store::TASK_CHARS), super::store::now()],
        )
    }

    pub fn run_titles(&self, ids: &[String]) -> rusqlite::Result<HashMap<String, (String, String)>> {
        let mut stmt = self.db.prepare("select project, title from sessions where id = ?1")?;
        let mut out = HashMap::new();
        for id in ids {
            if let Some(pair) = stmt.query_map([id], |r| Ok((text(r, "project"), text(r, "title"))))?.next() {
                out.insert(id.clone(), pair?);
            }
        }
        Ok(out)
    }

    pub fn backlinks(&self, title: &str, limit: usize) -> rusqlite::Result<Vec<(i64, String)>> {
        let pattern = format!("%[[{}%", like_escape(title.trim()));
        let mut stmt = self
            .db
            .prepare("select id, title from notes where body like ?1 escape '\\' order by updated desc limit ?2")?;
        stmt.query_map(params![pattern, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }

    pub fn graph(&self, project: Option<&str>) -> rusqlite::Result<Graph> {
        let mut g = Graph::default();
        let mut index = HashMap::new();
        let filter = if project.is_some() { " where project = ?1" } else { "" };
        let mut stmt =
            self.db.prepare(&format!("select id, project, title, body, session, updated from notes{filter}"))?;
        let map = |r: &Row| -> rusqlite::Result<(i64, String, String, String, String, f64)> {
            Ok((
                r.get(0)?,
                text(r, "project"),
                text(r, "title"),
                text(r, "body"),
                text(r, "session"),
                real(r, "updated"),
            ))
        };
        let rows: Vec<_> = match project {
            Some(p) => stmt.query_map([p], map)?.collect::<Result<_, _>>()?,
            None => stmt.query_map([], map)?.collect::<Result<_, _>>()?,
        };
        let mut titled: HashMap<String, usize> = HashMap::new();
        let mut bodies = Vec::with_capacity(rows.len());
        for (id, proj, title, body, session, t) in rows {
            let i = g.add(
                &mut index,
                Node { id: format!("n{id}"), label: title.clone(), kind: "note", group: proj.clone(), t },
            );
            titled.entry(title.to_lowercase()).or_insert(i);
            if !proj.is_empty() {
                let hub = g.add(
                    &mut index,
                    Node { id: format!("{HUB}{proj}"), label: proj.clone(), kind: "project", group: proj, t: 0.0 },
                );
                g.edges.push((i, hub));
            }
            if !session.is_empty() {
                let s = g.add(
                    &mut index,
                    Node {
                        id: format!("s{session}"),
                        label: String::new(),
                        kind: "session",
                        group: g.nodes[i].group.clone(),
                        t,
                    },
                );
                g.edges.push((i, s));
            }
            bodies.push((i, body));
        }
        for (i, body) in bodies {
            for l in links(&body) {
                if let Some(&j) = titled.get(&l.to_lowercase())
                    && j != i
                {
                    g.edges.push((i, j));
                }
            }
        }
        let mut stmt = self.db.prepare(&format!("select id, project, title, last from sessions{filter}"))?;
        let map = |r: &Row| -> rusqlite::Result<(String, String, String, f64)> {
            Ok((text(r, "id"), text(r, "project"), text(r, "title"), real(r, "last")))
        };
        let runs: Vec<_> = match project {
            Some(p) => stmt.query_map([p], map)?.collect::<Result<_, _>>()?,
            None => stmt.query_map([], map)?.collect::<Result<_, _>>()?,
        };
        for (id, proj, title, t) in runs {
            if let Some(&i) = index.get(&format!("s{id}")) {
                let n = &mut g.nodes[i];
                (n.label, n.t) = (title, t);
            } else if !title.is_empty() {
                let i = g.add(
                    &mut index,
                    Node { id: format!("s{id}"), label: title, kind: "session", group: proj.clone(), t },
                );
                if let Some(&hub) = index.get(&format!("{HUB}{proj}")) {
                    g.edges.push((i, hub));
                }
            }
        }
        g.edges.sort_unstable();
        g.edges.dedup();
        Ok(g)
    }
}

#[cfg(test)]
mod tests {
    use crate::{NewNote, Store};

    fn note<'a>(project: &'a str, title: &'a str, body: &'a str) -> NewNote<'a> {
        NewNote { project, kind: "note", title, body, source: "vault", session: "" }
    }

    #[test]
    fn board_lists_projects_links_and_a_graph() {
        let home = std::env::temp_dir().join(format!("lt-board-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let mut s = Store::open(&home).unwrap();
        let a = s.add_note(&note("Alpha", "Plan", "see [[Spec]] and [[Missing|x]]")).unwrap().unwrap();
        let b = s.add_note(&note("Alpha", "Spec", "the spec")).unwrap().unwrap();
        s.add_note(&note("Beta", "Other", "[[spec]] too")).unwrap();
        s.todo_add("Alpha", "ship", "", "todo").unwrap();
        s.session_seen("sid1", "Alpha", "/x", "", false).unwrap();
        let cards = s.cards().unwrap();
        let alpha = cards.iter().find(|c| c.name == "Alpha").unwrap();
        assert_eq!((alpha.notes, alpha.todos, alpha.sessions), (2, 1, 1));
        assert_eq!(s.notes_of("Alpha", 10).unwrap().len(), 2);
        assert_eq!(s.note_titled("spec").unwrap().map(|n| n.id), Some(b));
        let back: Vec<i64> = s.backlinks("Spec", 10).unwrap().into_iter().map(|(id, _)| id).collect();
        assert!(back.contains(&a) && back.len() == 2, "{back:?}");
        assert_eq!(super::links("a [[One]] b [[Two#h]] [[Three|t]]"), ["One", "Two", "Three"]);
        let g = s.graph(Some("Alpha")).unwrap();
        let kinds: Vec<&str> = g.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, ["note", "project", "note"]);
        assert!(g.edges.contains(&(0, 2)) && g.edges.contains(&(0, 1)), "{:?}", g.edges);
        assert_eq!(s.graph(None).unwrap().nodes.iter().filter(|n| n.kind == "project").count(), 2);
        let _ = std::fs::remove_dir_all(&home);
    }
}
