use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use regex::Regex;
use rusqlite::params;

use super::store::Store;
use super::text::{oneline, scrub};

const BODY_CHARS: usize = 12_000;
const TITLE_CHARS: usize = 200;
const SKIP: &[&str] = &[".obsidian", ".obsidian-mcp", ".trash", "templates", ".git"];
const KIND_DIRS: &[(&str, &str)] = &[
    ("knowledge/mistakes", "mistake"),
    ("knowledge/patterns", "pattern"),
    ("rules", "rule"),
    ("projects", "project"),
    ("sessions", "session"),
    ("eingang", "draft"),
    ("resources", "reference"),
    ("archive", "archive"),
    ("notes", "note"),
];

static FRONT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\A---\s*\n(.*?)\n---\s*\n").expect("front pattern"));
static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#\s+(.+)$").expect("head pattern"));

pub struct Page {
    pub project: String,
    pub kind: String,
    pub title: String,
    pub body: String,
}

fn posix(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn front(text: &str) -> (HashMap<String, String>, &str) {
    let Some(m) = FRONT.captures(text) else { return (HashMap::new(), text) };
    let mut meta = HashMap::new();
    for line in m[1].lines() {
        if line.starts_with([' ', '\t']) {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            meta.insert(k.trim().to_lowercase(), v.trim().trim_matches(['"', '\'']).to_string());
        }
    }
    let end = m.get(0).map_or(0, |all| all.end());
    (meta, &text[end..])
}

fn kind_of(rel: &str, inbox: &str) -> String {
    let low = rel.to_lowercase();
    if low.starts_with(&format!("{inbox}/")) {
        return "draft".into();
    }
    KIND_DIRS.iter().find(|(prefix, _)| low.starts_with(&format!("{prefix}/"))).map_or("note", |(_, k)| *k).into()
}

fn project_of_rel(rel: &str, meta: &HashMap<String, String>) -> String {
    if let Some(p) = meta.get("project").filter(|p| !p.is_empty()) {
        return p.trim_matches(['[', ']']).rsplit('/').next().unwrap_or("").to_string();
    }
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() >= 2 && ["projects", "sessions"].contains(&parts[0].to_lowercase().as_str()) {
        let name = if parts.len() == 2 {
            Path::new(parts[1]).file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned())
        } else {
            parts[1].to_string()
        };
        return if name == "general" { String::new() } else { name };
    }
    String::new()
}

pub fn parse(text: &str, rel: &str, claude: bool, inbox: &str) -> Page {
    let (meta, body) = front(text);
    let stem = Path::new(rel).file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let given = |k: &str| meta.get(k).filter(|t| !t.is_empty()).cloned();
    let title = given("title")
        .or_else(|| given("name"))
        .or_else(|| HEAD.captures(body).map(|h| h[1].trim().to_string()))
        .unwrap_or(stem);
    if claude {
        let kind = meta.get("type").map(|t| t.trim()).filter(|t| !t.is_empty()).unwrap_or("rule").to_string();
        let body = match meta.get("description").filter(|d| !d.is_empty()) {
            Some(d) => format!("{d}\n{body}"),
            None => body.to_string(),
        };
        return Page { project: String::new(), kind, title, body };
    }
    Page { project: project_of_rel(rel, &meta), kind: kind_of(rel, inbox), title, body: body.to_string() }
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let sub = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            if !SKIP.contains(&name.to_lowercase().as_str()) {
                walk(&e.path(), &sub, out);
            }
        } else if name.to_lowercase().ends_with(".md") {
            out.push((e.path(), sub));
        }
    }
}

fn files(base: &Path, claude: bool) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    if claude {
        if let Ok(entries) = fs::read_dir(base) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".md") && name != "MEMORY.md" && e.path().is_file() {
                    out.push((e.path(), name));
                }
            }
        }
    } else {
        walk(base, "", &mut out);
    }
    out
}

fn mtime(p: &Path) -> Option<f64> {
    fs::metadata(p).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs_f64())
}

impl Store {
    fn index_source(&mut self, source: &str, bases: &[PathBuf], claude: bool, inbox: &str) -> rusqlite::Result<usize> {
        let mut have: HashMap<String, (i64, f64)> = HashMap::new();
        {
            let mut stmt = self.db.prepare("select id, origin, updated from notes where source = ?1")?;
            let rows = stmt.query_map([source], |r| {
                Ok((
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    (r.get(0)?, r.get::<_, Option<f64>>(2)?.unwrap_or(0.0)),
                ))
            })?;
            for row in rows {
                let (origin, v) = row?;
                have.insert(origin, v);
            }
        }
        let mut seen = HashSet::new();
        let mut scanned = false;
        let mut changed = 0;
        let tx = self.db.transaction()?;
        for base in bases.iter().filter(|b| b.is_dir()) {
            scanned = true;
            for (p, rel) in files(base, claude) {
                let origin = format!("{source}:{}", posix(&p));
                seen.insert(origin.clone());
                let Some(t) = mtime(&p) else { continue };
                let old = have.get(&origin).copied();
                if old.is_some_and(|(_, u)| (u - t).abs() < 1.0) {
                    continue;
                }
                let Ok(raw) = fs::read(&p) else { continue };
                let page = parse(&String::from_utf8_lossy(&raw), &rel, claude, inbox);
                let title = oneline(&scrub(&page.title), TITLE_CHARS);
                let body: String = scrub(page.body.trim()).chars().take(BODY_CHARS).collect();
                match old {
                    Some((id, _)) => tx.execute(
                        "update notes set project = ?1, kind = ?2, title = ?3, body = ?4, updated = ?5 where id = ?6",
                        params![page.project, page.kind, title, body, t, id],
                    )?,
                    None => tx.execute(
                        "insert into notes(project, kind, title, body, files, source, created, updated, origin) \
                         values (?1, ?2, ?3, ?4, '[]', ?5, ?6, ?6, ?7)",
                        params![page.project, page.kind, title, body, source, t, origin],
                    )?,
                };
                changed += 1;
            }
        }
        if scanned {
            for (origin, (id, _)) in &have {
                if !seen.contains(origin) {
                    tx.execute("delete from notes where id = ?1", [id])?;
                    changed += 1;
                }
            }
        }
        tx.commit()?;
        if changed > 0 {
            self.vecs = None;
        }
        Ok(changed)
    }

    pub fn index_files(&mut self, vault: &Path, inbox: &str, claude: &[PathBuf]) -> rusqlite::Result<usize> {
        let inbox = inbox.trim_matches('/').to_lowercase();
        let inbox = if inbox.is_empty() { "inbox".to_string() } else { inbox };
        let t = super::store::now();
        let n = self.index_source("vault", &[vault.to_path_buf()], false, &inbox)?
            + self.index_source("claude", claude, true, &inbox)?;
        self.db.execute(
            "insert into meta(k, v) values ('vault_indexed', ?1) on conflict(k) do update set v = excluded.v",
            [t.to_string()],
        )?;
        Ok(n)
    }

    pub fn unembedded(&self, limit: usize) -> rusqlite::Result<Vec<(i64, String)>> {
        let mut stmt = self.db.prepare(
            "select n.id, n.title, n.body from notes n left join vecs v on v.id = n.id where v.id is null \
             order by n.source = 'user' desc, n.updated desc limit ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            let title: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
            let body: String = r.get::<_, Option<String>>(2)?.unwrap_or_default();
            Ok((r.get(0)?, format!("{title}\n{body}")))
        })?;
        rows.collect()
    }
}

pub fn claude_dirs(home: &Path) -> Vec<PathBuf> {
    let base = home.join(".claude").join("projects");
    let Ok(entries) = fs::read_dir(base) else { return Vec::new() };
    entries.flatten().map(|e| e.path().join("memory")).filter(|m| m.is_dir()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_and_claude_files_become_notes() {
        let base = std::env::temp_dir().join(format!("ltm-vault-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vault = base.join("Vault");
        let claude = base.join("memory");
        for dir in ["Rules", "Projects/Shop", "Inbox", ".obsidian"] {
            fs::create_dir_all(vault.join(dir)).unwrap();
        }
        fs::create_dir_all(&claude).unwrap();
        fs::write(vault.join("Rules/brave.md"), "---\ntitle: Brave only\n---\nNever Chrome.").unwrap();
        fs::write(vault.join("Projects/Shop/plan.md"), "# The plan\nShip it").unwrap();
        fs::write(vault.join("Inbox/idea.md"), "token: abcdefgh12 secret").unwrap();
        fs::write(vault.join(".obsidian/x.md"), "skip").unwrap();
        fs::write(
            claude.join("feedback_a.md"),
            "---\nname: Short answers\ndescription: keep it short\ntype: feedback\n---\nbody",
        )
        .unwrap();
        fs::write(claude.join("MEMORY.md"), "index").unwrap();
        let mut s = Store::open(&base).unwrap();
        assert_eq!(s.index_files(&vault, "Inbox", std::slice::from_ref(&claude)).unwrap(), 4);
        assert_eq!(s.index_files(&vault, "Inbox", std::slice::from_ref(&claude)).unwrap(), 0);
        let hits = s.search("brave", "", 8, None, 0.0).unwrap();
        let n = &hits[0].note;
        assert_eq!((n.kind.as_str(), n.title.as_str(), n.source.as_str()), ("rule", "Brave only", "vault"));
        assert_eq!(n.path(), Some(posix(&vault.join("Rules/brave.md"))).as_deref());
        let plan = &s.search("plan", "", 8, None, 0.0).unwrap()[0].note;
        assert_eq!((plan.project.as_str(), plan.kind.as_str(), plan.title.as_str()), ("Shop", "project", "The plan"));
        let idea = &s.search("idea secret", "", 8, None, 0.0).unwrap()[0].note;
        assert_eq!((idea.kind.as_str(), idea.title.as_str()), ("draft", "idea"));
        assert!(idea.body.contains("[secret]") && !idea.body.contains("abcdefgh12"));
        let mem = &s.search("short answers", "", 8, None, 0.0).unwrap()[0].note;
        assert_eq!(
            (mem.kind.as_str(), mem.source.as_str(), mem.body.as_str()),
            ("feedback", "claude", "keep it short\nbody")
        );
        assert_eq!(s.unembedded(10).unwrap().len(), 4);
        fs::remove_file(vault.join("Projects/Shop/plan.md")).unwrap();
        assert_eq!(s.index_files(&vault, "Inbox", std::slice::from_ref(&claude)).unwrap(), 1);
        assert!(s.search("plan", "", 8, None, 0.0).unwrap().is_empty());
        drop(s);
        let _ = fs::remove_dir_all(&base);
    }
}
