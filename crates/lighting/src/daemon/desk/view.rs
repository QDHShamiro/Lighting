use std::collections::HashMap;

use regex::Regex;
use std::sync::LazyLock;

pub const DELTA_S: f64 = 300.0;
const DELTA_SHARE: f64 = 0.5;
const FOLD_LEAST: usize = 4;

static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^d\d+ ").expect("ref pattern"));

fn counts(lines: &[String]) -> HashMap<&str, usize> {
    let mut map = HashMap::new();
    for l in lines {
        *map.entry(l.as_str()).or_insert(0) += 1;
    }
    map
}

pub fn fold(lines: &[String]) -> (Vec<String>, usize) {
    let keys: Vec<Option<&str>> = lines.iter().map(|l| REF.find(l).map(|m| &l[m.end()..])).collect();
    let mut count: HashMap<&str, usize> = HashMap::new();
    for k in keys.iter().flatten() {
        *count.entry(*k).or_insert(0) += 1;
    }
    let (mut out, mut seen, mut folded) = (Vec::new(), Vec::new(), 0);
    for (line, key) in lines.iter().zip(&keys) {
        match key {
            Some(k) if count[k] >= FOLD_LEAST => {
                if seen.contains(k) {
                    folded += 1;
                    continue;
                }
                seen.push(*k);
                out.push(format!("{line} (x{}, same in each item)", count[k]));
            }
            _ => out.push(line.clone()),
        }
    }
    (out, folded)
}

pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn changes(old: Option<&(f64, Vec<String>)>, lines: &[String], wref: &str, now: f64) -> Option<String> {
    let (at, before_lines) = old?;
    if before_lines.is_empty() || now - at > DELTA_S {
        return None;
    }
    let (before, after) = (counts(before_lines), counts(lines));
    let same: usize = after.iter().map(|(k, n)| (*n).min(before.get(k).copied().unwrap_or(0))).sum();
    if (same as f64) < lines.len() as f64 * DELTA_SHARE {
        return None;
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut added = Vec::new();
    for line in lines {
        let n = seen.entry(line.as_str()).or_insert(0);
        *n += 1;
        if *n > before.get(line.as_str()).copied().unwrap_or(0) {
            added.push(line.clone());
        }
    }
    let gone: usize = before.iter().map(|(k, n)| n.saturating_sub(after.get(k).copied().unwrap_or(0))).sum();
    let tokens = lines.iter().map(|x| x.chars().count() + 1).sum::<usize>() / 4;
    let gone = if gone > 0 { format!(", -{gone} gone") } else { String::new() };
    let mut out = vec![format!(
        "changed since your last snap: +{}{gone} (all {} lines, ~{} tokens: lighting snap {wref} --full)",
        added.len(),
        lines.len(),
        thousands(tokens)
    )];
    out.extend(fold(&added).0);
    Some(out.join("\n"))
}

pub fn score(spec: &str, title: &str, exe: &str) -> u8 {
    let (s, t, e) = (spec.to_lowercase(), title.to_lowercase(), exe.to_lowercase());
    if t == s {
        3
    } else if e.trim_end_matches(".exe") == s {
        2
    } else if t.contains(&s) || e.contains(&s) {
        1
    } else {
        0
    }
}

#[cfg(any(windows, test))]
pub fn hint_words(hint: &str) -> Vec<String> {
    hint.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| w.len() > 2).map(String::from).collect()
}

pub fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max - 3).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn repeated_rows_fold_like_2x() {
        let rows =
            lines(&["d1 button \"Del\"", "d2 button \"Del\"", "d3 button \"Del\"", "d4 button \"Del\"", "  text"]);
        let (out, folded) = fold(&rows);
        assert_eq!(out, ["d1 button \"Del\" (x4, same in each item)", "  text"]);
        assert_eq!(folded, 3);
        assert_eq!(fold(&rows[..3]).1, 0);
    }

    #[test]
    fn snap_delta_lists_new_lines() {
        let old = (100.0, lines(&["a", "b", "c", "d"]));
        let now = lines(&["a", "b", "c", "e"]);
        let out = changes(Some(&old), &now, "w2", 110.0).unwrap();
        assert_eq!(
            out,
            "changed since your last snap: +1, -1 gone (all 4 lines, ~2 tokens: lighting snap w2 --full)\ne"
        );
        assert!(changes(Some(&old), &now, "w2", 500.0).is_none());
        assert!(changes(Some(&old), &lines(&["x", "y", "z", "e"]), "w2", 110.0).is_none());
        assert!(changes(None, &now, "w2", 110.0).is_none());
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(12), "12");
    }

    #[test]
    fn windows_match_by_title_then_exe() {
        assert_eq!(score("notepad", "Notepad", "x.exe"), 3);
        assert_eq!(score("notepad", "Untitled", "notepad.exe"), 2);
        assert_eq!(score("note", "Untitled", "notepad.exe"), 1);
        assert_eq!(score("calc", "Untitled", "notepad.exe"), 0);
        assert_eq!(hint_words("spotify:search:SOS"), ["spotify", "search", "sos"]);
        assert_eq!(short("abcdefghij", 8), "abcde...");
    }
}
