use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::{local_now, stamp_secs};

const SKIP: &[&str] = &["ping", "status", "log", "suggest", "tokens"];
const NOT_COUNTED: &[&str] = &["selftest", "bench", "demo", "done", "ext-reload"];
const ACTION_VERBS: &[&str] =
    &["click", "type", "press", "fill", "select", "check", "scroll", "hover", "open", "launch", "do", "run"];
const MAX_BYTES: u64 = 5_000_000;
const REOPEN_EVERY: u32 = 500;
const SNAP_GAP_S: i64 = 15;

pub struct Row<'a> {
    pub name: &'a str,
    pub argv: &'a [String],
    pub code: i32,
    pub ms: u64,
    pub out: &'a str,
    pub test: bool,
    pub cut: usize,
    pub sid: &'a str,
    pub id: &'a str,
    pub queued_ms: u64,
    pub stages: &'a [(&'a str, u64)],
}

impl Row<'_> {
    pub fn json(&self, t: &str) -> String {
        let reference = self.argv.get(1).filter(|a| is_ref(a)).map_or("", String::as_str);
        let mut row = Fields::default();
        row.text("t", t);
        row.text("cmd", self.name);
        row.text("ref", reference);
        row.raw("ok", if self.code == 0 { "true" } else { "false" });
        row.raw("ms", &self.ms.to_string());
        row.raw("n", &self.out.chars().count().to_string());
        if self.argv.iter().skip(1).any(|a| a == "-f") {
            row.raw("f", "1");
        }
        if self.test {
            row.raw("x", "1");
        }
        if self.cut > 0 {
            row.raw("c", &self.cut.to_string());
        }
        if self.code != 0 {
            let text = self.out.replacen("err: ", "", 1);
            row.text("e", text.split(" -> try:").next().unwrap_or_default());
        }
        if !self.sid.is_empty() {
            row.text("sid", self.sid);
        }
        row.text("id", self.id);
        if self.queued_ms > 0 {
            row.raw("q", &self.queued_ms.to_string());
        }
        if !self.stages.is_empty() {
            let mut stages = Fields::default();
            for (stage, ms) in self.stages {
                stages.raw(stage, &ms.to_string());
            }
            row.raw("st", &stages.done());
        }
        row.done()
    }
}

#[derive(Default)]
struct Fields(String);

impl Fields {
    fn raw(&mut self, key: &str, value: &str) {
        self.0.push_str(if self.0.is_empty() { "{" } else { ", " });
        self.0.push_str(&Value::from(key).to_string());
        self.0.push_str(": ");
        self.0.push_str(value);
    }

    fn text(&mut self, key: &str, value: &str) {
        self.raw(key, &Value::from(value).to_string());
    }

    fn done(mut self) -> String {
        self.0.push_str(if self.0.is_empty() { "{}" } else { "}" });
        self.0
    }
}

fn is_ref(token: &str) -> bool {
    let rest = match token.split_once('.') {
        Some((frame, rest)) if frame.len() > 1 && frame.starts_with('f') && digits(&frame[1..]) => rest,
        Some(_) => return false,
        None => token,
    };
    let mut chars = rest.chars();
    matches!(chars.next(), Some('e' | 'd' | 'o' | 'w' | 't')) && digits(chars.as_str())
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

pub struct Log {
    path: PathBuf,
    file: Mutex<(Option<File>, u32)>,
}

impl Log {
    pub fn new(home: &Path) -> Log {
        Log { path: home.join("log.jsonl"), file: Mutex::new((None, 0)) }
    }

    pub fn write(&self, row: &Row) {
        if SKIP.contains(&row.name) {
            return;
        }
        let line = row.json(&local_now()) + "\n";
        let mut state = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if state.0.is_none() || state.1 >= REOPEN_EVERY {
            state.0 = None;
            if fs::metadata(&self.path).is_ok_and(|m| m.len() > MAX_BYTES) {
                let _ = fs::rename(&self.path, self.path.with_extension("old.jsonl"));
            }
            *state = (OpenOptions::new().create(true).append(true).open(&self.path).ok(), 0);
        }
        state.1 += 1;
        if state.0.as_mut().is_some_and(|f| f.write_all(line.as_bytes()).is_err()) {
            state.0 = None;
        }
    }
}

fn rows(path: &Path) -> Option<Vec<Value>> {
    let text = fs::read_to_string(path).ok()?;
    Some(text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(Value::is_object).collect())
}

fn tail(len: usize, n: usize) -> usize {
    if n == 0 { 0 } else { len.saturating_sub(n) }
}

pub fn listing(home: &Path, n: usize) -> String {
    let Ok(text) = fs::read_to_string(home.join("log.jsonl")) else { return "log is empty".into() };
    let lines: Vec<&str> = text.lines().collect();
    let out: Vec<String> = lines[tail(lines.len(), n)..]
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|e| {
            let t = e["t"].as_str()?;
            let ok = e["ok"].as_bool()?;
            let reference = e.get("ref").and_then(Value::as_str).unwrap_or_default();
            let ms = e["ms"].as_f64()? as i64;
            Some(format!(
                "{} {} {} {reference} {ms}ms",
                t.get(11..).unwrap_or_default(),
                if ok { "ok " } else { "ERR" },
                e["cmd"].as_str()?
            ))
        })
        .collect();
    if out.is_empty() { "log is empty".into() } else { out.join("\n") }
}

#[derive(Default)]
struct Tally {
    runs: i64,
    err: i64,
    chars: i64,
    sized: i64,
    ms: i64,
    snap_after: i64,
}

pub fn stats(home: &Path, last: usize) -> Option<String> {
    let mut rows = rows(&home.join("log.jsonl"))?;
    rows.retain(|r| !r.get("x").is_some_and(truthy) && !NOT_COUNTED.contains(&r["cmd"].as_str().unwrap_or_default()));
    let rows = &rows[tail(rows.len(), last)..];
    if rows.is_empty() {
        return None;
    }
    let mut seen: Vec<(String, Tally)> = Vec::new();
    let mut prev: Option<&Value> = None;
    for e in rows {
        let cmd = e["cmd"].as_str().unwrap_or_default();
        let at = match seen.iter().position(|(c, _)| c == cmd) {
            Some(at) => at,
            None => {
                seen.push((cmd.to_string(), Tally::default()));
                seen.len() - 1
            }
        };
        let s = &mut seen[at].1;
        s.runs += 1;
        s.err += i64::from(!e.get("ok").is_some_and(truthy));
        s.ms += e.get("ms").and_then(Value::as_f64).unwrap_or(0.0) as i64;
        if let Some(n) = e.get("n") {
            s.chars += n.as_f64().unwrap_or(0.0) as i64;
            s.sized += 1;
        }
        if let Some(p) = prev {
            let pcmd = p["cmd"].as_str().unwrap_or_default();
            if cmd == "snap" && ACTION_VERBS.contains(&pcmd) && !p.get("f").is_some_and(truthy) {
                let gap = stamp_secs(e["t"].as_str().unwrap_or_default())
                    .zip(stamp_secs(p["t"].as_str().unwrap_or_default()))
                    .map(|(a, b)| a - b);
                if gap.is_some_and(|g| g <= SNAP_GAP_S)
                    && let Some((_, t)) = seen.iter_mut().find(|(c, _)| c == pcmd)
                {
                    t.snap_after += 1;
                }
            }
        }
        prev = Some(e);
    }
    seen.sort_by_key(|(_, s)| -s.runs);
    let mut out = vec!["cmd          runs  err%  avg ms  avg tok  snap after".to_string()];
    for (cmd, s) in &seen {
        let tok = if s.sized > 0 { format!("{:7}", s.chars / s.sized / 4 + 1) } else { "      -".into() };
        let after = if ACTION_VERBS.contains(&cmd.as_str()) {
            format!("{}%", 100 * s.snap_after / s.runs)
        } else {
            String::new()
        };
        out.push(format!("{cmd:<12} {:5} {:4}% {:7}  {tok}  {after}", s.runs, 100 * s.err / s.runs, s.ms / s.runs));
    }
    Some(out.join("\n"))
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    fn row<'a>(name: &'a str, argv: &'a [String], code: i32, out: &'a str) -> Row<'a> {
        Row { name, argv, code, ms: 12, out, test: false, cut: 0, sid: "", id: "7.1", queued_ms: 0, stages: &[] }
    }

    #[test]
    fn rows_keep_the_2x_fields_and_add_the_full_error() {
        let a = argv(&["click", "f1.e12", "-f", "save"]);
        assert_eq!(
            row("click", &a, 0, "clicked ü").json("2026-10-30 12:00:01"),
            r#"{"t": "2026-10-30 12:00:01", "cmd": "click", "ref": "f1.e12", "ok": true, "ms": 12, "n": 9, "f": 1, "id": "7.1"}"#
        );
        let long = format!("err: {} -> try: lighting help\n! note", "x".repeat(80));
        let mut r = row("open", &a[..1], 1, &long);
        r.sid = "s-1";
        r.queued_ms = 3;
        r.test = true;
        r.cut = 40;
        r.stages = &[("route", 5), ("observe", 1)];
        let line = r.json("2026-10-30 12:00:02");
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["e"].as_str().map(str::len), Some(80));
        assert_eq!(
            (v["sid"].as_str(), v["q"].as_i64(), v["x"].as_i64(), v["c"].as_i64()),
            (Some("s-1"), Some(3), Some(1), Some(40))
        );
        assert_eq!(v["st"]["route"].as_i64(), Some(5));
        assert!(line.contains(r#""st": {"route": 5, "observe": 1}"#), "{line}");
        assert_eq!(v["ref"].as_str(), Some(""));
    }

    #[test]
    fn refs_are_read_like_2x() {
        for good in ["e1", "d22", "o3", "w4", "t5", "f1.e3", "f12.t0"] {
            assert!(is_ref(good), "{good}");
        }
        for bad in ["e", "x1", "f.e1", "fx.e1", "f1.", "f1.e", "e1a", "", "f1.f2.e3", "https://a.b"] {
            assert!(!is_ref(bad), "{bad}");
        }
    }

    #[test]
    fn the_log_lists_and_counts_like_2x() {
        let home = std::env::temp_dir().join(format!("lt-core-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        assert_eq!(listing(&home, 15), "log is empty");
        assert_eq!(stats(&home, 2000), None);
        let lines = [
            r#"{"t": "2026-10-30 12:00:00", "cmd": "click", "ref": "e3", "ok": true, "ms": 10, "n": 40}"#,
            r#"{"t": "2026-10-30 12:00:05", "cmd": "snap", "ref": "", "ok": true, "ms": 30, "n": 400}"#,
            r#"{"t": "2026-10-30 12:00:40", "cmd": "click", "ref": "e4", "ok": false, "ms": 7, "n": 20, "e": "x"}"#,
            r#"{"t": "2026-10-30 12:01:00", "cmd": "snap", "ref": "", "ok": true, "ms": 20, "n": 200}"#,
            r#"{"t": "2026-10-30 12:01:01", "cmd": "selftest", "ref": "", "ok": true, "ms": 900, "n": 2}"#,
            r#"{"t": "2026-10-30 12:01:02", "cmd": "recall", "ref": "", "ok": true, "ms": 3, "n": 9, "x": 1}"#,
            "not json",
        ];
        fs::write(home.join("log.jsonl"), lines.join("\n") + "\n").unwrap();
        assert_eq!(listing(&home, 3), "12:01:01 ok  selftest  900ms\n12:01:02 ok  recall  3ms");
        let want = [
            "cmd          runs  err%  avg ms  avg tok  snap after",
            "click            2   50%       8        8  50%",
            "snap             2    0%      25       76  ",
        ];
        assert_eq!(stats(&home, 2000).unwrap(), want.join("\n"));
        let log = Log::new(&home);
        let a = argv(&["ping"]);
        log.write(&row("ping", &a, 0, "pong"));
        let a = argv(&["version"]);
        log.write(&row("version", &a, 0, "3.0.0"));
        let text = fs::read_to_string(home.join("log.jsonl")).unwrap();
        assert_eq!(text.lines().count(), lines.len() + 1);
        assert!(text.lines().last().unwrap().contains(r#""cmd": "version""#));
        fs::remove_dir_all(&home).unwrap();
    }
}
