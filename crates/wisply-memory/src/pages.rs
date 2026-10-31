use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use super::learner::Record;
use super::store::{date, now};
use super::text::oneline;

pub const WIKI_KINDS: &[&str] = &["mistake", "rule", "pattern"];
const BUILTIN: &[(&str, &str)] = &[
    ("mistake", "## Problem\n## Cause\n## Fix\n## Rule\n"),
    ("rule", "## Rule\n## Why\n## When it applies\n"),
    ("pattern", "## Pattern\n## When to use\n## Example\n"),
];
const PARTS: &[(&str, &str)] =
    &[("summary", "Done"), ("why", "Why"), ("rejected", "Tried and rejected"), ("next", "Left open")];
const SESSIONS: &str = "Sessions";
const TEMPLATES: &str = "Templates";
const GENERAL: &str = "general";
const SID_CHARS: usize = 8;
const NAME_CHARS: usize = 60;
const FOLDER_CHARS: usize = 80;
const TITLE_CHARS: usize = 120;

static UNSAFE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f]+"#).expect("unsafe name pattern"));
static HEADS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^##\s+(.+)$").expect("heads pattern"));

pub fn safe_name(text: &str, n: usize) -> String {
    let s = UNSAFE.replace_all(text, " ");
    let s = s.trim_matches([' ', '.']).split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.chars().take(n).collect();
    let s = s.trim_matches([' ', '.']);
    if s.is_empty() { "session".into() } else { s.into() }
}

fn sid8(sid: &str) -> String {
    sid.chars().take(SID_CHARS).collect()
}

fn link(project: &str) -> Option<String> {
    (!project.is_empty()).then(|| format!("[[Projects/{project}]]"))
}

fn old_session(base: &Path, sid: &str) -> Option<PathBuf> {
    let tail = format!(" {}.md", sid8(sid));
    fs::read_dir(base).ok()?.flatten().filter(|d| d.path().is_dir()).find_map(|d| {
        fs::read_dir(d.path())
            .ok()?
            .flatten()
            .map(|f| f.path())
            .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&tail)))
    })
}

pub fn write_session(root: &Path, sid: &str, project: &str, r: &Record, started: f64) -> Option<PathBuf> {
    let base = root.join(SESSIONS);
    let folder = base.join(safe_name(if project.is_empty() { GENERAL } else { project }, FOLDER_CHARS));
    let old = old_session(&base, sid);
    let title = oneline(if r.title.is_empty() { "session" } else { &r.title }, TITLE_CHARS);
    let day = date(started);
    let shown = if project.is_empty() { "-" } else { project };
    let mut lines =
        vec![format!("# {title}"), format!("Session: {}   Date: {day}   Project: {shown}", sid8(sid)), String::new()];
    for (key, head) in PARTS {
        let text = match *key {
            "summary" => &r.summary,
            "why" => &r.why,
            "rejected" => &r.rejected,
            _ => &r.next,
        };
        let items: Vec<String> = text
            .lines()
            .filter(|x| !x.trim().is_empty())
            .map(|x| format!("- {}", x.trim().trim_start_matches(['-', '*', ' ']).trim()))
            .collect();
        if !items.is_empty() {
            lines.push(format!("## {head}"));
            lines.extend(items);
            lines.push(String::new());
        }
    }
    lines.extend(link(project));
    let path = folder.join(format!("{day} {} {}.md", safe_name(&title, NAME_CHARS), sid8(sid)));
    fs::create_dir_all(&folder).ok()?;
    fs::write(&path, lines.join("\n")).ok()?;
    if let Some(old) = old.filter(|o| *o != path) {
        let _ = fs::remove_file(old);
    }
    Some(path)
}

pub fn template(root: &Path, kind: &str) -> Vec<String> {
    let own = fs::read_to_string(root.join(TEMPLATES).join(format!("{kind}.md"))).ok();
    let builtin = BUILTIN.iter().find(|(k, _)| *k == kind).map_or("", |(_, t)| *t);
    let text = own.as_deref().unwrap_or(builtin);
    HEADS.captures_iter(text).map(|c| c[1].trim().to_string()).collect()
}

pub fn render(root: &Path, kind: &str, title: &str, sections: &[(String, String)], project: &str, sid: &str) -> String {
    let mut lines = vec![
        "---".to_string(),
        format!("type: {kind}"),
        format!("date: {}", date(now())),
        format!("project: {}", if project.is_empty() { "-" } else { project }),
        "draft: true".into(),
        format!("session: {}", sid8(sid)),
        "---".into(),
        format!("# {title}"),
        String::new(),
    ];
    for head in template(root, kind) {
        let value = sections.iter().find(|(h, _)| *h == head).map_or("", |(_, v)| v.trim());
        if !value.is_empty() {
            lines.extend([format!("## {head}"), value.to_string(), String::new()]);
        }
    }
    lines.extend(link(project));
    lines.join("\n") + "\n"
}

pub fn write_draft(inbox: &Path, kind: &str, title: &str, page: &str) -> Option<PathBuf> {
    let path = inbox.join(format!("{} {kind} {}.md", date(now()), safe_name(title, NAME_CHARS)));
    if path.exists() {
        return None;
    }
    fs::create_dir_all(inbox).ok()?;
    fs::write(&path, page).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_and_drafts_are_written_into_the_vault() {
        let root = std::env::temp_dir().join(format!("ltm-pages-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        assert_eq!(safe_name(" a/b: c?. ", 60), "a b c");
        assert_eq!(safe_name("...", 60), "session");
        let r = Record {
            title: "Cart fix".into(),
            summary: "- moved cart\n\n* added redis".into(),
            next: "tests".into(),
            ..Record::default()
        };
        let first = write_session(&root, "abcdef1234", "Shop", &r, now()).unwrap();
        let text = fs::read_to_string(&first).unwrap();
        assert!(text.starts_with("# Cart fix\nSession: abcdef12   Date: "), "{text}");
        assert!(
            text.contains("## Done\n- moved cart\n- added redis\n\n## Left open\n- tests\n")
                && text.ends_with("[[Projects/Shop]]")
        );
        let renamed = Record { title: "Cart done".into(), ..r };
        let second = write_session(&root, "abcdef1234", "Shop", &renamed, now()).unwrap();
        assert!(second != first && second.exists() && !first.exists());
        assert_eq!(template(&root, "rule"), ["Rule", "Why", "When it applies"]);
        let sections = [("Rule".to_string(), "Brave only".to_string()), ("Why".to_string(), String::new())];
        let page = render(&root, "rule", "Brave only", &sections, "", "s1");
        assert!(
            page.contains("type: rule\n")
                && page.contains("project: -\ndraft: true\nsession: s1\n---\n# Brave only\n\n## Rule\nBrave only\n")
        );
        assert!(!page.contains("## Why") && !page.contains("[[Projects"));
        let inbox = root.join("Inbox");
        let draft = write_draft(&inbox, "rule", "Brave only", &page).unwrap();
        assert!(draft.file_name().unwrap().to_string_lossy().ends_with(" rule Brave only.md"));
        assert!(write_draft(&inbox, "rule", "Brave only", &page).is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
