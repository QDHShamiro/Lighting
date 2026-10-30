use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusqlite::{Connection, params};
use tree_sitter::Parser;
use tree_sitter_language_pack::{PackConfig, configure, download, get_language};

use super::extract::extract;

const LANGS: &[(&str, &str)] = &[
    ("java", "java"),
    ("py", "python"),
    ("rs", "rust"),
    ("ts", "typescript"),
    ("tsx", "tsx"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsx", "javascript"),
    ("cs", "csharp"),
    ("go", "go"),
    ("kt", "kotlin"),
    ("kts", "kotlin"),
    ("cpp", "cpp"),
    ("cc", "cpp"),
    ("hpp", "cpp"),
    ("h", "c"),
    ("c", "c"),
    ("php", "php"),
    ("rb", "ruby"),
    ("lua", "lua"),
    ("swift", "swift"),
    ("scala", "scala"),
    ("dart", "dart"),
];
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "out",
    "dist",
    ".gradle",
    ".idea",
    "venv",
    ".venv",
    "__pycache__",
    ".next",
    ".graphify",
    "vendor",
    "bin",
    "obj",
];
const OUTPUT_DIRS: &[&str] = &["target", "build", "out", "dist", "bin", "obj"];
const MAX_BYTES: u64 = 600_000;
const RETRY: Duration = Duration::from_secs(600);
const SCHEMA: &str = "
create table if not exists files(path text primary key, mtime real, lang text, lines integer);
create table if not exists symbols(id integer primary key, path text, name text, low text, kind text, line integer,
  end_line integer, parent text, sig text);
create index if not exists symbols_low on symbols(low);
create index if not exists symbols_path on symbols(path);
create table if not exists refs(path text, name text, low text, line integer, caller text);
create index if not exists refs_low on refs(low);
create index if not exists refs_path on refs(path);
create table if not exists vecs(id integer primary key, v blob);
";

static FAILED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Index {
    pub root: PathBuf,
    pub db: Connection,
    pub left: usize,
}

pub fn lang_of(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_lowercase();
    LANGS.iter().find(|(e, _)| *e == ext).map(|(_, l)| *l)
}

fn slug(root: &Path) -> String {
    let name: String = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    let key = root.to_string_lossy().to_lowercase();
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{name}-{:08x}", hash >> 32)
}

impl Index {
    pub fn open(home: &Path, root: &Path) -> rusqlite::Result<Index> {
        let dir = home.join("code");
        let _ = fs::create_dir_all(&dir);
        let db = Connection::open(dir.join(format!("{}.db", slug(root))))?;
        db.busy_timeout(Duration::from_secs(15))?;
        db.pragma_update(None, "journal_mode", "wal")?;
        db.execute_batch(SCHEMA)?;
        let _ = configure(&PackConfig { cache_dir: Some(dir.join("parsers")), ..PackConfig::default() });
        Ok(Index { root: root.to_path_buf(), db, left: 0 })
    }

    pub fn refresh(&mut self, budget: Option<Duration>) -> rusqlite::Result<usize> {
        let have: HashMap<String, f64> = {
            let mut q = self.db.prepare("select path, mtime from files")?;
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
        };
        let files = listing(&self.root);
        let mut todo = Vec::new();
        let mut seen = HashSet::new();
        for rel in files {
            let Some(lang) = lang_of(&rel) else { continue };
            seen.insert(rel.clone());
            let Ok(meta) = fs::metadata(self.root.join(&rel)) else { continue };
            let mtime =
                meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64());
            if have.get(&rel) == Some(&mtime) || meta.len() > MAX_BYTES {
                continue;
            }
            todo.push((rel, lang, mtime));
        }
        let wanted: Vec<&str> = todo
            .iter()
            .map(|(_, l, _)| *l)
            .filter(|l| !lock(&FAILED).get(*l).is_some_and(|at| at.elapsed() < RETRY))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if !wanted.is_empty() {
            let _ = download(&wanted);
        }
        let mut parsers: HashMap<&str, Option<Parser>> = HashMap::new();
        let (start, mut changed) = (Instant::now(), 0);
        self.left = 0;
        let tx = self.db.transaction()?;
        for (rel, lang, mtime) in &todo {
            if budget.is_some_and(|b| start.elapsed() > b) {
                self.left += 1;
                continue;
            }
            let Ok(data) = fs::read(self.root.join(rel)) else { continue };
            let parser = parsers.entry(*lang).or_insert_with(|| {
                if lock(&FAILED).get(*lang).is_some_and(|at| at.elapsed() < RETRY) {
                    return None;
                }
                let Ok(language) = get_language(lang) else {
                    lock(&FAILED).insert(lang.to_string(), Instant::now());
                    return None;
                };
                let mut p = Parser::new();
                p.set_language(&language).ok()?;
                Some(p)
            });
            let (defs, refs) = parser.as_mut().map(|p| extract(p, &data)).unwrap_or_default();
            tx.execute("delete from vecs where id in (select id from symbols where path = ?1)", [rel])?;
            tx.execute("delete from symbols where path = ?1", [rel])?;
            tx.execute("delete from refs where path = ?1", [rel])?;
            for d in &defs {
                tx.execute(
                    "insert into symbols(path, name, low, kind, line, end_line, parent, sig) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![rel, d.name, d.name.to_lowercase(), d.kind, d.line, d.end, d.parent, d.sig],
                )?;
            }
            for r in &refs {
                tx.execute(
                    "insert into refs(path, name, low, line, caller) values (?1, ?2, ?3, ?4, ?5)",
                    params![rel, r.name, r.name.to_lowercase(), r.line, r.caller],
                )?;
            }
            let lines = data.iter().filter(|b| **b == b'\n').count() as i64 + 1;
            tx.execute(
                "insert or replace into files(path, mtime, lang, lines) values (?1, ?2, ?3, ?4)",
                params![rel, mtime, lang, lines],
            )?;
            changed += 1;
        }
        let gone: Vec<&String> = have.keys().filter(|p| !seen.contains(*p)).collect();
        for p in &gone {
            tx.execute("delete from vecs where id in (select id from symbols where path = ?1)", [p])?;
            for table in ["files", "symbols", "refs"] {
                tx.execute(&format!("delete from {table} where path = ?1"), [p])?;
            }
        }
        tx.commit()?;
        Ok(changed + gone.len())
    }
}

fn git_files(root: &Path, args: &[&str], skip: &[&str]) -> Option<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotePath=false", "-C"]).arg(root).args(["ls-files", "-z"]).args(args).arg(".");
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let names = String::from_utf8_lossy(&out.stdout).into_owned();
    Some(
        names
            .split('\0')
            .filter(|n| !n.is_empty() && lang_of(n).is_some())
            .filter(|n| {
                let parts: Vec<&str> = n.split('/').collect();
                !parts[..parts.len() - 1].iter().any(|p| skip.contains(p))
            })
            .map(String::from)
            .collect(),
    )
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                walk(root, &path, found);
            }
        } else if lang_of(&name).is_some()
            && let Ok(rel) = path.strip_prefix(root)
        {
            found.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

pub fn listing(root: &Path) -> Vec<String> {
    let kept: Vec<&str> = SKIP_DIRS.iter().filter(|d| !OUTPUT_DIRS.contains(d)).copied().collect();
    let tracked = git_files(root, &["-c"], &kept);
    let loose = git_files(root, &["-o", "--exclude-standard"], SKIP_DIRS);
    if let (Some(mut a), Some(b)) = (tracked, loose) {
        a.extend(b);
        return a;
    }
    let mut found = Vec::new();
    walk(root, root, &mut found);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_slugs_read_like_2x() {
        assert_eq!(lang_of("src/Main.java"), Some("java"));
        assert_eq!(lang_of("a/b.TSX"), Some("tsx"));
        assert_eq!(lang_of("README.md"), None);
        let s = slug(Path::new("E:/plugins/My Plugin"));
        assert!(s.starts_with("My_Plugin-") && s.len() == "My_Plugin-".len() + 8, "{s}");
    }
}
