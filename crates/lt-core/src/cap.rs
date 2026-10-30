use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::local_now;

pub const OUT_CHARS: usize = 12_000;

pub fn cap(out_dir: &Path, text: &str, name: &str, lines: Option<usize>, chars: usize) -> (String, usize) {
    let rows: Vec<&str> = text.split('\n').collect();
    let total = text.chars().count();
    if lines.is_none_or(|n| rows.len() <= n) && total <= chars {
        return (text.to_string(), 0);
    }
    let path = out_file(out_dir, name, "txt");
    if fs::create_dir_all(out_dir).and_then(|()| fs::write(&path, text)).is_err() {
        return (text.to_string(), 0);
    }
    let keep = lines.filter(|n| *n > 0).map_or(&rows[..], |n| &rows[..n.min(rows.len())]);
    let mut out = keep.join("\n");
    if out.chars().count() > chars {
        let head: String = out.chars().take(chars).collect();
        out = head.rsplit_once('\n').map_or(head.clone(), |(before, _)| before.to_string());
    }
    let shown = out.matches('\n').count() + 1;
    let full = path.to_string_lossy().replace('\\', "/");
    out.push_str(&format!("\n... +{} more lines (full: {full})", rows.len().saturating_sub(shown)));
    let cut = total.saturating_sub(out.chars().count());
    (out, cut)
}

fn out_file(dir: &Path, name: &str, ext: &str) -> PathBuf {
    let now = local_now();
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_millis());
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(30).collect();
    let slug = if slug.is_empty() { "out".to_string() } else { slug };
    dir.join(format!("{}-{millis:03}-{slug}.{ext}", now[11..].replace(':', "")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_texts_are_cut_and_kept_like_2x() {
        let dir = std::env::temp_dir().join(format!("lt-core-cap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let text: String = (1..=500).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        assert_eq!(cap(&dir, "short", "x", Some(40), 12_000), ("short".to_string(), 0));
        let (out, cut) = cap(&dir, &text, "Log Stats!", Some(40), 12_000);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 41);
        assert_eq!(lines[39], "line 40");
        assert!(lines[40].starts_with("... +460 more lines (full: "), "{out}");
        assert!(lines[40].contains("-log-stats.txt)"), "{out}");
        let kept = lines[40].split("(full: ").nth(1).unwrap().trim_end_matches(')');
        assert_eq!(fs::read_to_string(kept).unwrap(), text);
        assert_eq!(cut, text.chars().count() - out.chars().count());
        let (out, cut) = cap(&dir, &"ä".repeat(30), "", None, 10);
        assert_eq!(out.lines().next(), Some("ääääääääää"));
        assert!(out.contains("... +0 more lines") && out.contains("-out.txt"), "{out}");
        assert_eq!(cut, 0);
        fs::remove_dir_all(&dir).unwrap();
    }
}
