use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use rusqlite::{Connection, Row, params};

use super::index::Index;

const TOP: i64 = 12;
const FOLDERS: usize = 10;
const LIMIT: i64 = 10;
const CALLERS: i64 = 12;
const ALL_CALLERS: i64 = 60;
const OUTLINE_MAX: usize = 80;
const SIG: usize = 110;
const BODY_MAX: i64 = 150;
const EMBED_LINES: i64 = 30;
pub const MEANT: &[&str] =
    &["function", "method", "constructor", "class", "struct", "enum", "trait", "interface", "record", "macro"];
const CODE_STOP: &[&str] = &[
    "function",
    "funktion",
    "method",
    "methode",
    "class",
    "klasse",
    "code",
    "file",
    "datei",
    "does",
    "called",
    "aufgerufen",
    "passiert",
    "happens",
    "happen",
    "wird",
    "werden",
    "gets",
];

#[derive(Debug, Clone)]
pub struct Sym {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub low: String,
    pub kind: String,
    pub line: i64,
    pub end: i64,
    pub parent: String,
    pub sig: String,
}

fn sym(r: &Row) -> rusqlite::Result<Sym> {
    Ok(Sym {
        id: r.get("id")?,
        path: r.get("path")?,
        name: r.get("name")?,
        low: r.get("low")?,
        kind: r.get("kind")?,
        line: r.get("line")?,
        end: r.get("end_line")?,
        parent: r.get::<_, Option<String>>("parent")?.unwrap_or_default(),
        sig: r.get::<_, Option<String>>("sig")?.unwrap_or_default(),
    })
}

pub(crate) fn syms(db: &Connection, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<Vec<Sym>> {
    let mut q = db.prepare(sql)?;
    q.query_map(args, sym)?.collect()
}

pub(crate) fn place(s: &Sym) -> String {
    let within = if s.parent.is_empty() { String::new() } else { format!(" in {}", s.parent) };
    format!("  {}:{} {} {}{within}", s.path, s.line, s.kind, s.name)
}

impl Index {
    fn name(&self) -> String {
        self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn overview(&self) -> rusqlite::Result<String> {
        let mut q = self.db.prepare("select lang, count(*), sum(lines) l from files group by lang order by l desc")?;
        let langs: Vec<(String, i64, i64)> = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<i64>>(2)?.unwrap_or(0))))?
            .collect::<Result<_, _>>()?;
        let shown = self.root.to_string_lossy().replace('\\', "/");
        if langs.is_empty() {
            return Ok(format!("no code files in {shown}"));
        }
        let kinds: Vec<String> = langs.iter().take(5).map(|(l, n, c)| format!("{n} {l} files ({c} lines)")).collect();
        let mut lines = vec![format!("{shown}: {}", kinds.join(", "))];
        let mut q = self.db.prepare("select path from files")?;
        let paths: Vec<Vec<String>> = q
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(Result::ok)
            .map(|p| {
                let mut parts: Vec<String> = p.split('/').map(String::from).collect();
                parts.pop();
                parts
            })
            .collect();
        let mut common = 0;
        while !paths.is_empty()
            && paths.iter().all(|p| p.len() > common)
            && paths.iter().map(|p| &p[common]).collect::<BTreeSet<_>>().len() == 1
        {
            common += 1;
        }
        let mut tops: BTreeMap<String, usize> = BTreeMap::new();
        for p in &paths {
            let key = p[common..p.len().min(common + 2)].join("/");
            *tops.entry(if key.is_empty() { ".".into() } else { key }).or_default() += 1;
        }
        let base = if common > 0 { format!("{}/", paths[0][..common].join("/")) } else { String::new() };
        let mut ranked: Vec<(String, usize)> = tops.into_iter().collect();
        ranked.sort_by_key(|r| std::cmp::Reverse(r.1));
        let folders: Vec<String> = ranked.iter().take(FOLDERS).map(|(k, n)| format!("{k} ({n})")).collect();
        let under = if base.is_empty() { String::new() } else { format!(" under {base}") };
        lines.push(format!("folders{under}: {}", folders.join(", ")));
        let mut q = self.db.prepare(
            "select s.name, s.path, s.line, count(*) n, (select count(*) from symbols x where x.low = s.low and x.kind = s.kind) same \
             from symbols s join refs r on r.low = s.low where s.kind in ('class', 'interface', 'struct', 'enum', 'trait', 'record', 'object') \
             and length(s.name) > 2 group by s.low order by n desc limit ?1",
        )?;
        let top: Vec<String> = q
            .query_map([TOP], |r| {
                let (name, path, line, same): (String, String, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(4)?);
                Ok(if same > 1 {
                    format!("{name} ({same} classes with this name)")
                } else {
                    format!("{name} ({path}:{line})")
                })
            })?
            .collect::<Result<_, _>>()?;
        if !top.is_empty() {
            lines.push(format!("most used types: {}", top.join(", ")));
        }
        let mains: Vec<String> = syms(&self.db, "select * from symbols where low = 'main' limit 5", [])?
            .iter()
            .map(|s| format!("{}:{}", s.path, s.line))
            .collect();
        if !mains.is_empty() {
            lines.push(format!("entry points: {}", mains.join(", ")));
        }
        Ok(lines.join("\n"))
    }

    fn callers_lines(&self, low: &str, limit: i64) -> rusqlite::Result<Vec<String>> {
        let mut q =
            self.db.prepare("select path, line, caller from refs where low = ?1 order by path, line limit ?2")?;
        let rows: Vec<(String, i64, String)> = q
            .query_map(params![low, limit + 1], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default()))
            })?
            .collect::<Result<_, _>>()?;
        let total: i64 = self.db.query_row("select count(*) from refs where low = ?1", [low], |r| r.get(0))?;
        if rows.is_empty() {
            return Ok(vec!["  no calls found".into()]);
        }
        let mut out = vec![format!("called from ({total}):")];
        out.extend(
            rows.iter()
                .take(limit as usize)
                .map(|(p, l, c)| format!("  {p}:{l}{}", if c.is_empty() { String::new() } else { format!(" in {c}") })),
        );
        if total > limit {
            out.push("  ... wisply code callers <name> lists all".into());
        }
        Ok(out)
    }

    pub fn lookup(&self, name: &str) -> rusqlite::Result<String> {
        let low = name.to_lowercase();
        let mut defs = syms(
            &self.db,
            "select * from symbols where low = ?1 order by kind != 'class', path limit ?2",
            params![low, LIMIT],
        )?;
        let mut fuzzy = false;
        if defs.is_empty() {
            let like = format!("%{}%", low.replace('%', ""));
            defs = syms(
                &self.db,
                "select * from symbols where low like ?1 order by length(name), path limit ?2",
                params![like, LIMIT],
            )?;
            fuzzy = !defs.is_empty();
        }
        if defs.is_empty() {
            return Ok(format!(
                "nothing named \"{name}\" in {} (wisply code \"<what it does>\" searches by meaning)",
                self.name()
            ));
        }
        let mut lines = vec![if fuzzy { "no exact match, close names:" } else { "defined:" }.to_string()];
        lines.extend(defs.iter().map(place));
        if !fuzzy {
            lines.extend(self.callers_lines(&low, CALLERS)?);
        }
        Ok(lines.join("\n"))
    }

    pub fn callers(&self, name: &str) -> rusqlite::Result<String> {
        Ok(self.callers_lines(&name.to_lowercase(), ALL_CALLERS)?.join("\n"))
    }

    fn find_file(&self, name: &str) -> rusqlite::Result<Vec<(String, String, i64)>> {
        let mut name = name.replace('\\', "/");
        if Path::new(&name).is_absolute() {
            let root = self.root.to_string_lossy().replace('\\', "/").to_lowercase();
            match name.to_lowercase().strip_prefix(&format!("{}/", root.trim_end_matches('/'))) {
                Some(_) => name = name[root.trim_end_matches('/').len() + 1..].to_string(),
                None => return Ok(Vec::new()),
            }
        }
        let name = name.strip_prefix("./").unwrap_or(&name).to_string();
        let get = |r: &Row| -> rusqlite::Result<(String, String, i64)> {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<i64>>(2)?.unwrap_or(0)))
        };
        let mut q = self.db.prepare("select path, lang, lines from files where path = ?1")?;
        let exact: Vec<_> = q.query_map([&name], get)?.collect::<Result<_, _>>()?;
        if !exact.is_empty() {
            return Ok(exact);
        }
        let mut q =
            self.db.prepare("select path, lang, lines from files where path like ?1 order by length(path) limit 6")?;
        q.query_map([format!("%/{}", name.replace('%', ""))], get)?.collect()
    }

    pub fn outline(&self, name: &str) -> rusqlite::Result<String> {
        let found = self.find_file(name)?;
        let Some((path, lang, count)) = found.first().cloned() else {
            return Ok(format!("no code file \"{name}\" in {}", self.name()));
        };
        if found.len() > 1 {
            let list: Vec<String> = found.iter().map(|(p, _, _)| format!("  {p}")).collect();
            return Ok(format!("several files match:\n{}", list.join("\n")));
        }
        let all = syms(&self.db, "select * from symbols where path = ?1 order by line, end_line desc", [&path])?;
        let mut lines = vec![format!("{path} ({count} lines, {lang}):")];
        for s in all.iter().take(OUTLINE_MAX) {
            let sig = if s.sig.is_empty() { &s.name } else { &s.sig };
            let sig: String = sig.trim_end_matches([' ', '{']).chars().take(SIG).collect();
            let pad = if s.parent.is_empty() { "  " } else { "    " };
            lines.push(format!("{pad}{}-{} {sig}", s.line, s.end));
        }
        if all.len() > OUTLINE_MAX {
            lines.push(format!("  ... {} more", all.len() - OUTLINE_MAX));
        }
        Ok(lines.join("\n"))
    }

    pub fn body(&self, name: &str) -> Result<String, String> {
        let (owner, short) = name.rsplit_once('.').unwrap_or(("", name));
        let q = owner.rsplit('.').next().unwrap_or("").to_lowercase();
        let found = if q.is_empty() {
            syms(&self.db, "select * from symbols where low = ?1 order by path, line limit 6", [short.to_lowercase()])
        } else {
            syms(
                &self.db,
                "select * from symbols where low = ?1 and (lower(parent) = ?2 or lower(path) like ?3 or lower(path) like ?4) order by path, line limit 6",
                params![short.to_lowercase(), q, format!("%/{q}.%"), format!("{q}.%")],
            )
        }
        .map_err(|e| e.to_string())?;
        let Some(s) = found.first() else {
            return Ok(format!("nothing named \"{name}\" in {} (wisply code {short} finds close names)", self.name()));
        };
        if found.len() > 1 {
            let list: Vec<String> = found.iter().map(place).collect();
            return Ok(format!(
                "several match, name the class or file (code body Class.{short} or file.{short}):\n{}",
                list.join("\n")
            ));
        }
        let text = fs::read_to_string(self.root.join(&s.path)).map_err(|e| format!("cannot read {}: {e}", s.path))?;
        let rows: Vec<&str> = text.lines().collect();
        let end = s.end.min(s.line - 1 + BODY_MAX);
        let mut out = vec![format!("{}:{}-{}", s.path, s.line, s.end)];
        out.extend(rows.iter().take(end as usize).skip((s.line - 1).max(0) as usize).map(|r| r.to_string()));
        if end < s.end {
            out.push(format!("... {} more lines (Read {} from line {})", s.end - end, s.path, end + 1));
        }
        Ok(out.join("\n"))
    }

    pub fn by_name(&self, text: &str, why: &str) -> rusqlite::Result<String> {
        let mut terms: Vec<String> = Vec::new();
        for w in text.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '_') {
            if w.chars().count() >= 3
                && !CODE_STOP.contains(&w)
                && !lt_memory::stop_word(w)
                && !terms.iter().any(|t| t == w)
            {
                terms.push(w.to_string());
            }
        }
        terms.truncate(8);
        let mut found: HashMap<i64, (Sym, BTreeSet<String>, BTreeSet<String>)> = HashMap::new();
        for t in &terms {
            let like = format!("%{t}%");
            for s in syms(&self.db, "select * from symbols where low like ?1 or lower(sig) like ?1 limit 400", [&like])?
            {
                let in_name = s.low.contains(t.as_str());
                let hit = found.entry(s.id).or_insert_with(|| (s, BTreeSet::new(), BTreeSet::new()));
                if in_name {
                    hit.1.insert(t.clone())
                } else {
                    hit.2.insert(t.clone())
                };
            }
        }
        let mut ranked: Vec<_> = found.into_values().collect();
        ranked.sort_by(|a, b| {
            let score = |h: &(Sym, BTreeSet<String>, BTreeSet<String>)| -(2 * h.1.len() as i64) - h.2.len() as i64;
            (score(a), !MEANT.contains(&a.0.kind.as_str()), a.0.name.len(), &a.0.path).cmp(&(
                score(b),
                !MEANT.contains(&b.0.kind.as_str()),
                b.0.name.len(),
                &b.0.path,
            ))
        });
        if ranked.is_empty() {
            return Ok(format!("{why}; no names contain those words either"));
        }
        let mut lines = vec![format!("{why}, names with those words:")];
        lines.extend(ranked.iter().take(LIMIT as usize).map(|h| place(&h.0)));
        Ok(lines.join("\n"))
    }

    pub fn meant_total(&self) -> rusqlite::Result<i64> {
        let kinds: Vec<String> = MEANT.iter().map(|k| format!("'{k}'")).collect();
        self.db
            .query_row(&format!("select count(*) from symbols where kind in ({})", kinds.join(", ")), [], |r| r.get(0))
    }

    pub fn unembedded(&self, limit: usize) -> rusqlite::Result<Vec<(i64, String)>> {
        let kinds: Vec<String> = MEANT.iter().map(|k| format!("'{k}'")).collect();
        let sql = format!(
            "select s.* from symbols s left join vecs v on v.id = s.id where v.id is null and s.kind in ({}) limit ?1",
            kinds.join(", ")
        );
        let todo = syms(&self.db, &sql, [limit as i64])?;
        Ok(todo.into_iter().map(|s| (s.id, self.source_text(&s))).collect())
    }

    fn source_text(&self, s: &Sym) -> String {
        let head =
            format!("{} {} in {} ({})", s.kind, s.name, if s.parent.is_empty() { "-" } else { &s.parent }, s.path);
        let Ok(text) = fs::read_to_string(self.root.join(&s.path)) else { return head };
        let end = s.end.min(s.line - 1 + EMBED_LINES);
        let body: Vec<&str> = text.lines().take(end as usize).skip((s.line - 1).max(0) as usize).collect();
        format!("{head}\n{}", body.join("\n"))
    }

    pub fn save_vecs(&mut self, pairs: &[(i64, Vec<f32>)]) -> rusqlite::Result<()> {
        let tx = self.db.transaction()?;
        for (id, v) in pairs {
            let alive: i64 = tx.query_row("select count(*) from symbols where id = ?1", [id], |r| r.get(0))?;
            if alive > 0 {
                tx.execute(
                    "insert or replace into vecs(id, v) values (?1, ?2)",
                    params![id, v.iter().flat_map(|x| half::f16::from_f32(*x).to_le_bytes()).collect::<Vec<u8>>()],
                )?;
            }
        }
        tx.commit()
    }

    pub fn meaning(&self, query: &[f32], why_none: &str) -> rusqlite::Result<String> {
        let mut q = self.db.prepare("select id, v from vecs")?;
        let rows: Vec<(i64, Vec<u8>)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        let total = self.meant_total()?;
        if rows.is_empty() {
            return Ok(why_none.to_string());
        }
        let mut scored: Vec<(f32, i64)> = rows
            .iter()
            .map(|(id, blob)| {
                let dot: f32 = blob
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(query)
                    .map(|(b, q)| half::f16::from_le_bytes(*b).to_f32() * q)
                    .sum();
                (dot, *id)
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(LIMIT as usize);
        let partial = if (rows.len() as i64) < total {
            format!(" ({} of {total} symbols indexed so far)", rows.len())
        } else {
            String::new()
        };
        let mut lines = vec![format!("closest by meaning:{partial}")];
        for (score, id) in scored {
            if let Some(s) = syms(&self.db, "select * from symbols where id = ?1", [id])?.first() {
                lines.push(format!("{} ({score:.2})", place(s)));
            }
        }
        Ok(lines.join("\n"))
    }
}
