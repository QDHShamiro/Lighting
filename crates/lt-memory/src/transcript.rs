use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::project::project_of;
use super::text::scrub;

pub const CHUNK_CHARS: usize = 24_000;
const CHUNKS: usize = 8;
const TEXT_CHARS: usize = 1500;
const TOOL_CHARS: usize = 240;
const ARG_CHARS: usize = 160;
const ARG_KEYS: &[&str] = &["file_path", "path", "pattern", "skill", "url", "query", "prompt"];
const EDITS: &[&str] = &["Edit", "Write", "NotebookEdit"];
const EDIT_WEIGHT: usize = 3;

static REMINDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<system-reminder>.*?</system-reminder>|<local-command-caveat>.*?</local-command-caveat>")
        .expect("reminder pattern")
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?[a-zA-Z][\w-]*(?:\s[^<>]*)?>").expect("tag pattern"));
static ABSOLUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[A-Za-z]:[\\/]|/)").expect("path pattern"));

#[derive(Debug, Default)]
pub struct Condensed {
    pub text: String,
    pub end: u64,
    pub title: String,
    pub paths: Vec<String>,
    pub more: bool,
}

pub fn cut(text: &str, n: usize) -> String {
    let s = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= n { s } else { format!("{} …", s.chars().take(n).collect::<String>()) }
}

pub fn clean(text: &str) -> String {
    TAG.replace_all(&REMINDER.replace_all(text, ""), "").trim().to_string()
}

fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
    }
}

fn tool_line(name: &str, input: &Value) -> String {
    if name == "Bash" || name == "PowerShell" {
        return format!("{name}: {}", cut(&input.get("command").map(shown).unwrap_or_default(), TOOL_CHARS));
    }
    if let Some(v) = ARG_KEYS.iter().find_map(|k| input.get(*k).filter(|v| truthy(v))) {
        return format!("{name} {}", cut(&shown(v), ARG_CHARS));
    }
    format!("{name} {}", cut(&input.to_string(), ARG_CHARS))
}

fn result_text(c: &Value) -> String {
    match c.get("content") {
        Some(Value::Array(items)) => {
            items.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(" ")
        }
        Some(Value::Null) | None => String::new(),
        Some(v) => shown(v),
    }
}

fn pieces(d: &Value, kind: &str, paths: &mut Vec<String>) -> Vec<String> {
    let content = d.get("message").and_then(|m| m.get("content"));
    let items: Vec<Value> = match content {
        Some(Value::String(s)) => vec![serde_json::json!({"type": "text", "text": s})],
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for c in &items {
        let piece = match c.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = clean(c.get("text").and_then(Value::as_str).unwrap_or(""));
                let who = if kind == "user" { "USER: " } else { "CLAUDE: " };
                if text.is_empty() { String::new() } else { format!("{who}{}", cut(&text, TEXT_CHARS)) }
            }
            Some("tool_use") => {
                let name = c.get("name").and_then(Value::as_str).unwrap_or("");
                let input = c.get("input").cloned().unwrap_or(Value::Null);
                let file =
                    ["file_path", "notebook_path", "path"].iter().find_map(|k| input.get(*k)).and_then(Value::as_str);
                if let Some(f) = file.filter(|f| ABSOLUTE.is_match(f)) {
                    let times = if EDITS.contains(&name) { EDIT_WEIGHT } else { 1 };
                    paths.extend(std::iter::repeat_n(f.to_string(), times));
                }
                format!("TOOL {}", tool_line(name, &input))
            }
            Some("tool_result") if c.get("is_error").is_some_and(truthy) => {
                format!("ERROR: {}", cut(&result_text(c), TOOL_CHARS))
            }
            _ => String::new(),
        };
        if !piece.is_empty() {
            out.push(piece);
        }
    }
    out
}

pub fn condense(path: &Path, offset: u64, limit: Option<usize>) -> std::io::Result<Condensed> {
    let limit = limit.unwrap_or(CHUNK_CHARS * CHUNKS);
    let mut file = BufReader::new(File::open(path)?);
    file.seek(SeekFrom::Start(offset))?;
    let mut out = Condensed { end: offset, ..Condensed::default() };
    let (mut parts, mut size, mut raw) = (Vec::new(), 0usize, Vec::new());
    loop {
        raw.clear();
        if file.read_until(b'\n', &mut raw)? == 0 || !raw.ends_with(b"\n") {
            break;
        }
        if size >= limit {
            out.more = true;
            break;
        }
        out.end += raw.len() as u64;
        let Ok(d) = serde_json::from_slice::<Value>(&raw) else { continue };
        let kind = d.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "ai-title" {
            if let Some(t) = d.get("aiTitle").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                out.title = t.to_string();
            }
            continue;
        }
        let side = ["isMeta", "isSidechain"].iter().any(|k| d.get(*k).is_some_and(truthy));
        if !["user", "assistant"].contains(&kind) || side {
            continue;
        }
        for piece in pieces(&d, kind, &mut out.paths) {
            size += piece.chars().count() + 1;
            parts.push(piece);
        }
    }
    out.text = scrub(&parts.join("\n"));
    Ok(out)
}

pub fn chunks(text: &str) -> Vec<String> {
    let (mut out, mut cur, mut size) = (Vec::new(), Vec::new(), 0usize);
    for line in text.split('\n') {
        let n = line.chars().count();
        if size + n > CHUNK_CHARS && !cur.is_empty() {
            out.push(cur.join("\n"));
            cur.clear();
            size = 0;
        }
        cur.push(line);
        size += n + 1;
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    out
}

pub fn project_of_files(paths: &[String]) -> String {
    let mut seen: Vec<(String, usize)> = Vec::new();
    for f in paths {
        let Some(parent) = Path::new(f).parent().filter(|_| Path::new(f).is_absolute()) else { continue };
        let name = project_of(&parent.to_string_lossy());
        if name.is_empty() {
            continue;
        }
        match seen.iter_mut().find(|(n, _)| *n == name) {
            Some((_, c)) => *c += 1,
            None => seen.push((name, 1)),
        }
    }
    let best = seen.iter().map(|(_, c)| *c).max().unwrap_or(0);
    seen.into_iter().find(|(_, c)| *c == best).map(|(n, _)| n).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn transcripts_are_condensed_like_2x() {
        let dir = std::env::temp_dir().join(format!("ltm-transcript-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let lines = [
            r#"{"type":"ai-title","aiTitle":"Cart fix"}"#,
            r#"{"type":"user","message":{"content":"fix the <b>cart</b><system-reminder>noise</system-reminder>"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"On it"},{"type":"tool_use","name":"Edit","input":{"file_path":"C:/x/a.rs"}}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"skip"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"boom   token: abcdef123"}]}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo   test"}}]}}"#,
        ];
        let body = lines.join("\n") + "\n{\"type\":\"user\"";
        let path = dir.join("t.jsonl");
        fs::write(&path, &body).unwrap();
        let c = condense(&path, 0, None).unwrap();
        let want = "USER: fix the cart\nCLAUDE: On it\nTOOL Edit C:/x/a.rs\nERROR: boom token: [secret]\nTOOL Bash: cargo test";
        assert_eq!(c.text, want);
        assert_eq!((c.title.as_str(), c.more, c.paths.len()), ("Cart fix", false, 3));
        assert_eq!(c.end as usize, body.len() - "{\"type\":\"user\"".len());
        let again = condense(&path, c.end, None).unwrap();
        assert!(again.text.is_empty() && again.end == c.end);
        let small = condense(&path, 0, Some(10)).unwrap();
        assert!(small.more && small.text.starts_with("USER: fix the cart") && small.end < c.end);
        assert_eq!(cut("a  b\nc", 10), "a b c");
        assert_eq!(cut("abcdef", 3), "abc …");
        let long = format!("{}\n{}", "a".repeat(CHUNK_CHARS - 5), "b".repeat(10));
        assert_eq!(chunks(&long).len(), 2);
        assert_eq!(chunks("x\ny"), ["x\ny"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
