use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

pub const STATUSES: &[&str] = &["new", "done", "unchecked", "dropped", "suggestion"];
pub const LESSONS_MAX: usize = 40;
const STEM_MIN: usize = 4;
const PREFIX_MIN: usize = 5;
const SUFFIXES: &[&str] = &["ungen", "ung", "ern", "en", "er", "es", "em", "ing", "ed", "e", "n", "s"];
const STOP: &[&str] = &[
    "und", "oder", "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "mit", "fuer",
    "von", "bei", "auf", "aus", "ins", "zum", "zur", "als", "wie", "wenn", "dass", "nicht", "mehr", "auch", "noch",
    "nur", "statt", "ueber", "unter", "durch", "ohne", "beim", "vom", "the", "and", "for", "with", "from", "into",
    "use", "add",
];

pub fn now() -> String {
    wisply_core::local_now()[..16].to_string()
}

pub fn today() -> String {
    wisply_core::local_now()[..10].to_string()
}

pub fn stamp() -> String {
    let t = wisply_core::local_now();
    format!("{}{}{}-{}{}{}", &t[0..4], &t[5..7], &t[8..10], &t[11..13], &t[14..16], &t[17..19])
}

pub fn slug(text: &str, n: usize) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss").chars()
    {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(n).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "run".into() } else { out }
}

fn stem(w: &str) -> String {
    let mut w = w.to_string();
    for _ in 0..2 {
        if let Some(s) = SUFFIXES.iter().find(|s| w.ends_with(*s) && w.len() - s.len() >= STEM_MIN) {
            w.truncate(w.len() - s.len());
        }
    }
    w
}

fn words(text: &str) -> Vec<String> {
    let t = text.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss");
    let set: HashSet<String> =
        t.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| w.len() >= 3 && !STOP.contains(w)).map(stem).collect();
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    out
}

fn same_word(x: &str, y: &str) -> bool {
    let (short, long) = if x.len() <= y.len() { (x, y) } else { (y, x) };
    x == y || (short.len() >= PREFIX_MIN && long.starts_with(short))
}

pub fn similar(a: &str, b: &str) -> f64 {
    let wa = words(a);
    let mut wb = words(b);
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let total = wa.len() + wb.len();
    let mut hits = 0;
    for w in &wa {
        if let Some(i) = wb.iter().position(|v| same_word(w, v)) {
            wb.remove(i);
            hits += 1;
        }
    }
    hits as f64 / (total - hits) as f64
}

pub fn cut(text: &str, n: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= n { flat } else { format!("{}…", flat.chars().take(n - 1).collect::<String>()) }
}

pub fn table(head: &[&str], rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = vec![format!("| {} |", head.join(" | ")), format!("|{}", "---|".repeat(head.len()))];
    for r in rows {
        out.push(format!("| {} |", r.iter().map(|c| c.replace('|', "\\|")).collect::<Vec<_>>().join(" | ")));
    }
    out.join("\n")
}

pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn int(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

pub fn points(idea: &Value) -> f64 {
    let (w, s, a) = (int(idea, "impact"), int(idea, "sure"), int(idea, "effort").max(1));
    ((w * s) as f64 / a as f64 * 100.0).round() / 100.0
}

pub struct Home {
    pub dir: PathBuf,
    sid: String,
}

impl Home {
    pub fn new(wisply_home: &Path, sid: &str) -> Home {
        let sid: String =
            sid.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(60).collect();
        Home { dir: wisply_home.join("think"), sid }
    }

    pub fn runs(&self) -> PathBuf {
        self.dir.join("runs")
    }

    fn pointer(&self) -> PathBuf {
        let name = if self.sid.is_empty() { "current".to_string() } else { format!("current-{}", self.sid) };
        self.runs().join(name)
    }

    pub fn current_id(&self) -> Option<String> {
        [self.pointer(), self.runs().join("current")].iter().find_map(|f| {
            let id = fs::read_to_string(f).ok()?.trim().to_string();
            self.runs().join(&id).join("state.json").exists().then_some(id)
        })
    }

    pub fn set_current(&self, id: &str) {
        let _ = fs::create_dir_all(self.runs());
        let _ = fs::write(self.pointer(), id);
        let _ = fs::write(self.runs().join("current"), id);
    }

    pub fn load(&self, id: Option<&str>) -> Result<Run, String> {
        let id = match id {
            Some(i) => i.to_string(),
            None => self.current_id().ok_or("no run yet; start one: wisply think start \"<name>\" <path>")?,
        };
        let dir = self.runs().join(&id);
        let raw = fs::read_to_string(dir.join("state.json"))
            .map_err(|_| format!("run {id} not found (wisply think runs)"))?;
        let s: Value = serde_json::from_str(&raw).map_err(|e| format!("run {id} is broken: {e}"))?;
        Ok(Run { dir, s })
    }

    pub fn all(&self) -> Vec<Run> {
        let Ok(list) = fs::read_dir(self.runs()) else { return Vec::new() };
        let mut out: Vec<Run> = list
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| self.load(Some(&e.file_name().to_string_lossy())).ok())
            .collect();
        out.sort_by(|a, b| a.id().cmp(b.id()));
        out
    }

    pub fn lessons_file(&self) -> PathBuf {
        self.dir.join("lessons.md")
    }

    pub fn lessons(&self) -> Vec<String> {
        fs::read_to_string(self.lessons_file())
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("- "))
            .map(str::to_string)
            .collect()
    }

    pub fn write_lessons(&self, items: &[String]) {
        let _ = fs::create_dir_all(&self.dir);
        let head = format!("# Lessons from runs (newest first, max {LESSONS_MAX})\n\n");
        let body: Vec<&str> = items.iter().take(LESSONS_MAX).map(String::as_str).collect();
        let _ = fs::write(
            self.lessons_file(),
            format!("{head}{}{}", body.join("\n"), if body.is_empty() { "" } else { "\n" }),
        );
    }
}

pub fn lesson_text(line: &str) -> String {
    let t = line.strip_prefix("- ").unwrap_or(line);
    let t = if t.len() > 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-' { t[10..].trim_start() } else { t };
    let t = if t.starts_with('[') { t.split_once("] ").map_or(t, |(_, r)| r) } else { t };
    match t.rfind(" (") {
        Some(i) if t.ends_with("x)") && t[i + 2..t.len() - 2].bytes().all(|b| b.is_ascii_digit()) => t[..i].to_string(),
        _ => t.to_string(),
    }
}

pub fn lesson_tag(line: &str) -> String {
    let t = line.strip_prefix("- ").unwrap_or(line);
    t.get(11..)
        .filter(|r| r.starts_with('['))
        .and_then(|r| r[1..].split_once(']'))
        .map(|(tag, _)| tag.to_string())
        .unwrap_or_default()
}

fn lesson_count(line: &str) -> u32 {
    line.rfind(" (")
        .and_then(|i| line.get(i + 2..line.len().saturating_sub(2)))
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
}

pub fn learn(home: &Home, text: &str, tag: &str) -> String {
    let mut items = home.lessons();
    let twin =
        items.iter().enumerate().map(|(i, x)| (i, similar(&lesson_text(x), text))).max_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, sim)) = twin
        && sim >= 0.6
    {
        let old = items.remove(i);
        let n = lesson_count(&old) + 1;
        let tag = lesson_tag(&old);
        let tag = if tag.is_empty() { String::new() } else { format!(" [{tag}]") };
        let body = lesson_text(&old);
        items.insert(0, format!("- {}{tag} {body} ({n}x)", today()));
        home.write_lessons(&items);
        return format!("lesson confirmed ({n}x): {body}");
    }
    let t = if tag.is_empty() { String::new() } else { format!(" [{tag}]") };
    items.insert(0, format!("- {}{t} {text}", today()));
    home.write_lessons(&items);
    format!(
        "lesson saved ({}/{LESSONS_MAX}){}",
        items.len().min(LESSONS_MAX),
        if tag.is_empty() { " - without --tag it never shows up as a matching lesson" } else { "" }
    )
}

pub struct Run {
    pub dir: PathBuf,
    pub s: Value,
}

impl Run {
    pub fn create(home: &Home, name: &str) -> Run {
        let id = format!("{}-{}", stamp(), slug(name, 30));
        let dir = home.runs().join(&id);
        Run { dir, s: json!({"id": id, "name": name, "status": "running", "start": now()}) }
    }

    pub fn id(&self) -> &str {
        text(&self.s, "id")
    }

    pub fn save(&self) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("cannot write the run: {e}"))?;
        let tmp = self.dir.join("state.json.tmp");
        let body = serde_json::to_string_pretty(&self.s).map_err(|e| e.to_string())?;
        fs::write(&tmp, body).map_err(|e| format!("cannot write the run: {e}"))?;
        fs::rename(&tmp, self.dir.join("state.json")).map_err(|e| format!("cannot write the run: {e}"))
    }

    pub fn map(&mut self) -> &mut Map<String, Value> {
        self.s.as_object_mut().expect("run state is an object")
    }

    pub fn list(&mut self, key: &str) -> &mut Vec<Value> {
        let m = self.map();
        if !m.get(key).is_some_and(Value::is_array) {
            m.insert(key.into(), json!([]));
        }
        m.get_mut(key).and_then(Value::as_array_mut).expect("list just set")
    }

    pub fn ideas(&self) -> Vec<Value> {
        self.s.get("ideas").and_then(Value::as_array).cloned().unwrap_or_default()
    }

    pub fn round(&self) -> i64 {
        int(&self.s, "round").max(1)
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.s
            .get("paths")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect())
            .unwrap_or_default()
    }

    pub fn root(&self) -> PathBuf {
        PathBuf::from(text(&self.s, "root"))
    }

    pub fn idea_mut(&mut self, id: i64) -> Option<&mut Value> {
        self.list("ideas").iter_mut().find(|i| int(i, "id") == id)
    }

    pub fn add_idea(&mut self, mut idea: Value, force: bool) -> Result<i64, String> {
        let title = text(&idea, "title").to_string();
        let twin = self
            .ideas()
            .into_iter()
            .map(|i| (int(&i, "id"), similar(text(&i, "title"), &title)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if !force
            && let Some((id, sim)) = twin
            && sim >= DEDUPE
        {
            return Err(format!("#{id}"));
        }
        let id = self.ideas().len() as i64 + 1;
        let round = self.round();
        let p = points(&idea);
        let m = idea.as_object_mut().expect("idea is an object");
        m.insert("id".into(), json!(id));
        m.insert("round".into(), json!(round));
        m.entry("status").or_insert(json!("new"));
        m.insert("points".into(), json!(p));
        self.list("ideas").push(idea);
        Ok(id)
    }
}

pub const DEDUPE: f64 = 0.55;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_slugs_and_similarity() {
        assert_eq!(slug("Wisply 3.0: Größe & Tempo!", 30), "wisply-3-0-groesse-tempo");
        assert!(similar("Cache the parsed rules", "cache parsed rule") > 0.6);
        assert!(similar("Cache the parsed rules", "Shorter README intro") < 0.2);
        assert_eq!(lesson_text("- 2026-10-30 [plan] Measure first (3x)"), "Measure first");
        assert_eq!(lesson_tag("- 2026-10-30 [plan] Measure first (3x)"), "plan");
        assert_eq!(lesson_count("- 2026-10-30 [plan] Measure first (3x)"), 3);
        assert_eq!(cut("a  b   c", 10), "a b c");
        assert_eq!(table(&["a", "b"], &[vec!["1".into(), "x|y".into()]]), "| a | b |\n|---|---|\n| 1 | x\\|y |");
    }

    #[test]
    fn runs_keep_ideas_and_lessons() {
        let base = std::env::temp_dir().join(format!("wisply-think-run-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let home = Home::new(&base, "sid-1");
        let mut run = Run::create(&home, "Test run");
        run.map().insert("round".into(), json!(1));
        let a = run
            .add_idea(json!({"title": "Cache the parsed rules", "impact": 3, "sure": 2, "effort": 1}), false)
            .unwrap();
        assert_eq!(a, 1);
        assert_eq!(
            run.add_idea(json!({"title": "cache parsed rule", "impact": 1, "sure": 1, "effort": 1}), false),
            Err("#1".into())
        );
        run.save().unwrap();
        home.set_current(run.id());
        let back = home.load(None).unwrap();
        assert_eq!(points(&back.ideas()[0]), 6.0);
        assert_eq!(home.all().len(), 1);
        assert!(learn(&home, "Measure before you change", "plan").starts_with("lesson saved (1/40)"));
        assert!(learn(&home, "measure before changing", "").starts_with("lesson confirmed (2x)"));
        assert!(home.lessons()[0].contains("[plan] Measure before you change (2x)"));
        let _ = fs::remove_dir_all(&base);
    }
}
