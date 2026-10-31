use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use super::vault::{HEAD, front, posix};

const HOMES: &[(&str, &str)] =
    &[("mistake", "Knowledge/Mistakes"), ("pattern", "Knowledge/Patterns"), ("rule", "Rules")];
const INDEXES: &[(&str, &str)] =
    &[("mistake", "Knowledge/INDEX.md"), ("pattern", "Knowledge/INDEX.md"), ("rule", "Rules/INDEX.md")];
const SECTIONS: &[(&str, &str)] = &[("mistake", "Mistakes"), ("pattern", "Patterns")];
const OTHER_HOME: &str = "Notes";
const OTHER_INDEX: &str = "Knowledge/INDEX.md";
const TRASH: &str = ".trash";

static DATED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2} \w+ ").expect("dated pattern"));
static DRAFT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^draft: true\n").expect("draft pattern"));

pub struct Draft {
    pub path: PathBuf,
    pub kind: String,
    pub project: String,
}

fn lookup(table: &[(&str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn stem(p: &Path) -> String {
    p.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned())
}

fn rel(root: &Path, p: &Path) -> String {
    posix(p.strip_prefix(root).unwrap_or(p))
}

fn read(p: &Path) -> Result<String, String> {
    fs::read(p).map(|raw| String::from_utf8_lossy(&raw).replace("\r\n", "\n")).map_err(|e| format!("{}: {e}", posix(p)))
}

pub fn drafts(inbox: &Path) -> Vec<Draft> {
    let Ok(entries) = fs::read_dir(inbox) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    paths
        .into_iter()
        .map(|path| {
            let text = read(&path).unwrap_or_default();
            let meta = front(&text).0;
            let get = |k: &str, or: &str| meta.get(k).filter(|v| !v.is_empty()).cloned().unwrap_or_else(|| or.into());
            Draft { kind: get("type", "note"), project: get("project", "-"), path }
        })
        .collect()
}

pub fn accept(root: &Path, path: &Path, folder: Option<&str>) -> Result<PathBuf, String> {
    let text = read(path)?;
    let meta = front(&text).0;
    let kind = meta.get("type").filter(|k| !k.is_empty()).cloned().unwrap_or_else(|| "note".into());
    let dir = root.join(folder.unwrap_or_else(|| lookup(HOMES, &kind).unwrap_or(OTHER_HOME)));
    let dest = dir.join(format!("{}.md", DATED.replace(&stem(path), "")));
    if dest.exists() {
        return Err(format!("{} already exists", rel(root, &dest)));
    }
    let io = |e: std::io::Error| format!("{}: {e}", posix(&dest));
    fs::create_dir_all(&dir).map_err(io)?;
    fs::write(&dest, DRAFT.replacen(&text, 1, "").as_bytes()).map_err(io)?;
    fs::remove_file(path).map_err(|e| format!("{}: {e}", posix(path)))?;
    register(root, &dest, &kind, meta.get("project").map_or("-", String::as_str));
    Ok(dest)
}

fn register(root: &Path, dest: &Path, kind: &str, project: &str) {
    let index = root.join(lookup(INDEXES, kind).unwrap_or(OTHER_INDEX));
    let (Ok(body), Ok(raw)) = (read(dest), fs::read(&index)) else { return };
    let head = HEAD.captures(&body).map_or_else(|| stem(dest), |h| h[1].trim().to_string());
    let tail = if project.is_empty() || project == "-" { String::new() } else { format!(", {project}") };
    let line = format!("- [[{}]] — {head} ({kind}{tail})", rel(root, &dest.with_extension("")));
    let raw = String::from_utf8_lossy(&raw);
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let text = raw.replace("\r\n", "\n");
    let section = lookup(SECTIONS, kind)
        .and_then(|s| Regex::new(&format!(r"(?m)^## {}\s*$", regex::escape(s))).ok())
        .and_then(|rx| rx.find(&text).map(|m| m.end()));
    let out = match section {
        Some(at) => {
            let end = text[at..].find("\n## ").map_or(text.len(), |i| at + i);
            format!("{}\n{line}\n{}", text[..end].trim_end_matches('\n'), &text[end..])
        }
        None => format!("{}\n{line}\n", text.trim_end_matches('\n')),
    };
    let _ = fs::write(&index, out.replace('\n', eol));
}

pub fn drop_draft(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let trash = root.join(TRASH);
    let to = trash.join(path.file_name().unwrap_or_default());
    fs::create_dir_all(&trash).and_then(|()| fs::rename(path, &to)).map_err(|e| format!("{}: {e}", posix(path)))?;
    Ok(to)
}

pub fn rel_path(root: &Path, p: &Path) -> String {
    rel(root, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_are_accepted_into_their_home_and_index() {
        let root = std::env::temp_dir().join(format!("ltm-wiki-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let inbox = root.join("Inbox");
        fs::create_dir_all(&inbox).unwrap();
        fs::create_dir_all(root.join("Knowledge")).unwrap();
        fs::write(root.join("Knowledge/INDEX.md"), "# Index\r\n## Mistakes\r\n- [[a]]\r\n\r\n## Patterns\r\n").unwrap();
        let draft = "---\ntype: mistake\nproject: Shop\ndraft: true\n---\n# Cart lost items\n## Fix\nsave first\n";
        fs::write(inbox.join("2026-10-30 mistake Cart lost items.md"), draft).unwrap();
        fs::write(inbox.join("2026-10-30 rule Brave.md"), "---\ntype: rule\n---\nBrave only\n").unwrap();
        let found = drafts(&inbox);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].kind.as_str(), found[0].project.as_str()), ("mistake", "Shop"));
        assert_eq!((found[1].kind.as_str(), found[1].project.as_str()), ("rule", "-"));
        let dest = accept(&root, &found[0].path, None).unwrap();
        assert_eq!(rel_path(&root, &dest), "Knowledge/Mistakes/Cart lost items.md");
        assert!(!fs::read_to_string(&dest).unwrap().contains("draft: true") && !found[0].path.exists());
        let index = fs::read_to_string(root.join("Knowledge/INDEX.md")).unwrap();
        let want = "# Index\r\n## Mistakes\r\n- [[a]]\r\n- [[Knowledge/Mistakes/Cart lost items]] — Cart lost items \
                    (mistake, Shop)\r\n\r\n## Patterns\r\n";
        assert_eq!(index, want);
        fs::write(inbox.join("2026-10-31 mistake Cart lost items.md"), draft).unwrap();
        let again = drafts(&inbox).into_iter().find(|d| d.kind == "mistake").unwrap();
        assert_eq!(
            accept(&root, &again.path, None),
            Err("Knowledge/Mistakes/Cart lost items.md already exists".into())
        );
        let gone = drop_draft(&root, &found[1].path).unwrap();
        assert!(gone.ends_with(".trash/2026-10-30 rule Brave.md") && !found[1].path.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
