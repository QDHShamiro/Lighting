use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::Finding;
use crate::files::Pack;

const SENTENCE_WORDS: usize = 35;

static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\(([^)\s]+)\)").expect("link pattern"));
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\W\d_][\w'-]*").expect("word pattern"));

pub fn repeats(line: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut last: Option<(usize, usize, String)> = None;
    for m in WORD.find_iter(line) {
        let w = m.as_str().to_lowercase();
        if let Some((s, e, prev)) = &last
            && *prev == w
            && w.chars().count() > 1
            && line[*e..m.start()].chars().all(char::is_whitespace)
        {
            out.push((*s, m.end()));
        }
        last = Some((m.start(), m.end(), w));
    }
    out
}

pub fn drop_repeats(line: &str) -> String {
    let mut out = line.to_string();
    for (s, e) in repeats(line).into_iter().rev() {
        let first_end = s + line[s..e].find(char::is_whitespace).unwrap_or(e - s);
        out.replace_range(first_end..e, "");
    }
    out
}

fn prose(lines: &[&str]) -> Vec<(usize, bool)> {
    let mut code = false;
    lines
        .iter()
        .map(|l| {
            let t = l.trim_start();
            if t.starts_with("```") {
                code = !code;
                return false;
            }
            !code && !t.starts_with('|') && !t.starts_with('<') && !t.starts_with("    ") && !l.starts_with('\t')
        })
        .enumerate()
        .collect()
}

pub fn check(path: &Path, root: &Path, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let dir = path.parent().unwrap_or(root);
    let mut broken = Vec::new();
    let mut doubled = Vec::new();
    let mut long = Vec::new();
    let mut last_level = 0usize;
    let mut skipped = None;
    for (i, ok) in prose(&lines) {
        if !ok {
            continue;
        }
        let line = lines[i];
        let plain = strip_code(line);
        for c in LINK.captures_iter(&plain) {
            let target = c[1].split('#').next().unwrap_or("");
            if target.is_empty() || target.contains("://") || target.starts_with("mailto:") || target.starts_with('<') {
                continue;
            }
            let file = if let Some(abs) = target.strip_prefix('/') { root.join(abs) } else { dir.join(target) };
            if !file.exists() {
                broken.push((i + 1, target.to_string()));
            }
        }
        if let Some((s, e)) = repeats(&plain).first() {
            doubled.push((i + 1, plain[*s..*e].to_string()));
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && line[hashes..].starts_with(' ') {
            if last_level > 0 && hashes > last_level + 1 && skipped.is_none() {
                skipped = Some((i + 1, last_level, hashes));
            }
            last_level = hashes;
            continue;
        }
        if plain.trim_start().starts_with(['-', '*', '>']) {
            continue;
        }
        for sentence in plain.split_inclusive(['.', '!', '?']) {
            if sentence.split_whitespace().count() > SENTENCE_WORDS {
                long.push(i + 1);
                break;
            }
        }
    }
    if let Some((at, target)) = broken.first() {
        let more = if broken.len() > 1 { format!(" (+{} more)", broken.len() - 1) } else { String::new() };
        out.push(
            Finding::new(
                "broken-link",
                Pack::Docs,
                *at,
                format!("link to {target} goes nowhere{more}"),
                "fix the path or drop the link",
            )
            .rate(2, 3, 1),
        );
    }
    if let Some((at, example)) = doubled.first() {
        out.push(
            Finding::new(
                "repeated-word",
                Pack::Docs,
                *at,
                format!("{} lines repeat a word (\"{example}\")", doubled.len()),
                "drop the copy",
            )
            .rate(1, 3, 1)
            .fixed_by("repeat"),
        );
    }
    if let Some(at) = long.first() {
        out.push(
            Finding::new(
                "long-sentence",
                Pack::Docs,
                *at,
                format!("{} sentences over {SENTENCE_WORDS} words", long.len()),
                "split them",
            )
            .rate(1, 2, 1),
        );
    }
    if let Some((at, from, to)) = skipped {
        out.push(
            Finding::new(
                "heading-skip",
                Pack::Docs,
                at,
                format!("heading jumps from level {from} to {to}"),
                "use the next level",
            )
            .rate(1, 3, 1),
        );
    }
    out
}

fn strip_code(line: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in line.chars() {
        if c == '`' {
            inside = !inside;
        } else if !inside {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_find_links_repeats_and_jumps() {
        let dir = std::env::temp_dir().join(format!("wisply-think-docs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ok.md"), "x").unwrap();
        let long = "word ".repeat(40);
        let text = format!(
            "# A\n\n### C\n\nSee [ok](ok.md) and [gone](gone.md#x) and [web](https://x.y).\nThis is is wrong.\n{long}.\n```\nthe the\n```\n"
        );
        let got: Vec<(String, usize)> =
            check(&dir.join("README.md"), &dir, &text).into_iter().map(|f| (f.rule, f.line)).collect();
        assert_eq!(
            got,
            [
                ("broken-link".to_string(), 5),
                ("repeated-word".to_string(), 6),
                ("long-sentence".to_string(), 7),
                ("heading-skip".to_string(), 3)
            ]
        );
        assert_eq!(drop_repeats("This is is wrong, the the end"), "This is wrong, the end");
        assert!(repeats("a a b").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
