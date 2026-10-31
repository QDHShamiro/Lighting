use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

const DESCRIPTION_MAX: usize = 1024;
const LISTING_MAX: usize = 1536;
const LINES_MAX: usize = 500;
const TOC_FROM: usize = 100;
const KNOWN: &[&str] = &[
    "name",
    "description",
    "when_to_use",
    "argument-hint",
    "arguments",
    "disable-model-invocation",
    "user-invocable",
    "allowed-tools",
    "disallowed-tools",
    "model",
    "effort",
    "context",
    "agent",
    "background",
    "hooks",
    "paths",
    "shell",
    "metadata",
    "license",
    "compatibility",
];

static REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`((?:references|scripts)/[^`\s*]+)`|\]\(([^)#\s]+)\)").expect("ref pattern"));
static MD_TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([\w./-]+\.md)`").expect("md pattern"));
static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9-]{1,64}$").expect("name pattern"));

fn front(text: &str) -> Option<(Vec<(String, String)>, &str)> {
    let rest = text.strip_prefix("---")?.trim_start_matches('\r').strip_prefix('\n')?;
    let end = rest.find("\n---")?;
    let (head, body) = (&rest[..end], &rest[end + 4..]);
    let body = body.split_once('\n').map_or("", |(_, b)| b);
    let mut keys: Vec<(String, String)> = Vec::new();
    for line in head.lines() {
        if !line.starts_with([' ', '\t'])
            && let Some((k, v)) = line.split_once(':')
        {
            keys.push((k.trim().to_string(), v.trim().trim_matches(['"', '\'']).to_string()));
        } else if let Some(last) = keys.last_mut() {
            last.1.push(' ');
            last.1.push_str(line.trim());
        }
    }
    for k in &mut keys {
        if k.1.starts_with(['>', '|']) {
            k.1 = k.1.trim_start_matches(['>', '|', '-']).trim().to_string();
        }
    }
    Some((keys, body))
}

pub fn lint(root: &Path) -> (Vec<String>, Vec<String>) {
    let (mut errs, mut warns) = (Vec::new(), Vec::new());
    let Ok(text) = fs::read_to_string(root.join("SKILL.md")) else {
        return (vec![format!("SKILL.md missing in {}", root.display())], warns);
    };
    let Some((keys, body)) = front(&text) else {
        return (vec!["frontmatter missing or broken (--- above and below)".into()], warns);
    };
    let get = |k: &str| keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let name = Some(get("name"))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    if !NAME.is_match(&name) {
        errs.push(format!("name '{name}': only a-z, 0-9, dash, max 64"));
    }
    for bad in ["anthropic", "claude"] {
        if name.contains(bad) {
            warns.push(format!("name contains '{bad}' (refused when uploading to claude.ai)"));
        }
    }
    let (desc, when) = (get("description"), get("when_to_use"));
    if desc.is_empty() {
        errs.push("description missing".into());
    }
    if desc.chars().count() > DESCRIPTION_MAX {
        errs.push(format!("description {} chars > {DESCRIPTION_MAX}", desc.chars().count()));
    }
    if desc.chars().count() + when.chars().count() > LISTING_MAX {
        warns.push(format!("description + when_to_use over {LISTING_MAX} chars (cut in the listing)"));
    }
    for (k, _) in &keys {
        if !KNOWN.contains(&k.as_str()) {
            warns.push(format!("unknown frontmatter field '{k}' (ignored)"));
        }
    }
    let n = body.lines().count();
    if n > LINES_MAX {
        errs.push(format!("SKILL.md text {n} lines > {LINES_MAX}"));
    }
    let mut named = Vec::new();
    for c in REF.captures_iter(body) {
        let r = c.get(1).or(c.get(2)).map(|m| m.as_str()).unwrap_or("");
        if r.starts_with("http") || r.is_empty() {
            continue;
        }
        named.push(r.to_string());
        if !root.join(r).exists() {
            errs.push(format!("reference goes nowhere: {r}"));
        }
    }
    let refs = root.join("references");
    if let Ok(list) = fs::read_dir(&refs) {
        let mut list: Vec<_> =
            list.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md")).collect();
        list.sort();
        for f in list {
            let fname = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if !named.contains(&format!("references/{fname}")) {
                warns.push(format!("orphan: references/{fname} (SKILL.md never names it)"));
            }
            let t = fs::read_to_string(&f).unwrap_or_default();
            for c in MD_TOKEN.captures_iter(&t) {
                if !refs.join(&c[1]).exists() && !root.join(&c[1]).exists() {
                    warns.push(format!("dead reference in references/{fname}: {}", &c[1]));
                }
            }
            let lines = t.lines().count();
            if lines > LINES_MAX {
                errs.push(format!("references/{fname}: {lines} lines > {LINES_MAX}"));
            }
            if lines > TOC_FROM && !t.lines().any(|l| l.starts_with("## Contents") || l.starts_with("## Inhalt")) {
                warns.push(format!("references/{fname}: {lines} lines without a contents list on top"));
            }
        }
    }
    if text.contains(":\\") && text.split(":\\").skip(1).any(|r| r.starts_with(|c: char| c.is_ascii_alphabetic())) {
        warns.push("SKILL.md has Windows paths with \\ (prefer /)".into());
    }
    (errs, warns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_are_linted() {
        let dir = std::env::temp_dir().join(format!("wisply-think-skill-{}", std::process::id()));
        fs::create_dir_all(dir.join("references")).unwrap();
        fs::write(dir.join("references/a.md"), "see `b.md`").unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: Bad Name\ndescription: >\n  does things\nfoo: 1\n---\n\nUse `references/x.md`.\n",
        )
        .unwrap();
        let (errs, warns) = lint(&dir);
        assert_eq!(errs, ["name 'Bad Name': only a-z, 0-9, dash, max 64", "reference goes nowhere: references/x.md"]);
        assert_eq!(
            warns,
            [
                "unknown frontmatter field 'foo' (ignored)",
                "orphan: references/a.md (SKILL.md never names it)",
                "dead reference in references/a.md: b.md"
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
