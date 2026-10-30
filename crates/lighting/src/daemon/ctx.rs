use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

pub const SESSION_IDLE_S: f64 = 12.0 * 3600.0;
const REPEAT_NOTE_AT: u32 = 3;
const REPEAT_BRAKE_AT: u32 = 5;
const REPEAT_BRAKE_MAX_S: u64 = 30;
const REPEAT_WINDOW: Duration = Duration::from_secs(120);
const FAILS_KEPT: usize = 20;
const SAVE_STEP_S: i64 = 600;
const DESK: [&str; 4] = ["windows", "hwnd", "info", "launched"];

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[derive(Default)]
pub struct Fails(Vec<(Vec<String>, String, u32, Instant)>);

impl Fails {
    pub fn forget(&mut self, argv: &[String]) {
        self.0.retain(|(key, ..)| key != argv);
    }

    pub fn repeated(&mut self, argv: &[String], out: String, brake: bool) -> Result<String, (Duration, String)> {
        let now = Instant::now();
        let n = match self.0.iter_mut().find(|(key, ..)| key == argv) {
            Some((_, err, n, at)) => {
                *n = if *err == out && now.duration_since(*at) < REPEAT_WINDOW { *n + 1 } else { 1 };
                err.clone_from(&out);
                *at = now;
                *n
            }
            None => {
                self.0.push((argv.to_vec(), out.clone(), 1, now));
                if self.0.len() > FAILS_KEPT {
                    self.0.remove(0);
                }
                1
            }
        };
        if brake && n >= REPEAT_BRAKE_AT {
            let wait = 1u64.checked_shl(n - REPEAT_BRAKE_AT).unwrap_or(u64::MAX).min(REPEAT_BRAKE_MAX_S);
            let text = format!(
                "{out}\n! STOP: the same command failed {n} times in a row with this error, so Lighting waited {wait} s before answering. Change the command or end the loop"
            );
            return Err((Duration::from_secs(wait), text));
        }
        if n >= REPEAT_NOTE_AT {
            return Ok(format!(
                "{out}\n! the same command failed {n} times in a row with this error: repeating it will not help"
            ));
        }
        Ok(out)
    }
}

pub struct Ctx {
    pub sid: String,
    pub group: String,
    pub side: bool,
    pub used: f64,
    pub tabs: Vec<String>,
    pub owned: BTreeSet<String>,
    pub target: Option<(String, String)>,
    pub place: Option<Map<String, Value>>,
    pub last_target: Option<Value>,
    pub closed_url: Option<String>,
    pub closing: f64,
    pub desk: Map<String, Value>,
    pub fails: Fails,
    pub secret: Option<String>,
    pub cwd: String,
    pub notes: Vec<String>,
    pub cut: usize,
}

impl Ctx {
    fn new(sid: &str, group: String, side: bool) -> Ctx {
        Ctx {
            sid: sid.to_string(),
            group,
            side,
            used: now(),
            tabs: Vec::new(),
            owned: BTreeSet::new(),
            target: None,
            place: None,
            last_target: None,
            closed_url: None,
            closing: 0.0,
            desk: Map::new(),
            fails: Fails::default(),
            secret: None,
            cwd: String::new(),
            notes: Vec::new(),
            cut: 0,
        }
    }

    pub fn begin(&mut self, secret: Option<String>, cwd: String) {
        self.secret = secret;
        self.cwd = cwd;
        self.notes.clear();
        self.cut = 0;
    }

    pub fn note(&mut self, text: &str) {
        let line = format!("! {text}");
        if !self.notes.contains(&line) {
            self.notes.push(line);
        }
    }

    pub fn kind(&self) -> Option<&str> {
        self.target.as_ref().map(|(kind, _)| kind.as_str())
    }

    pub fn url(&self) -> String {
        self.place.as_ref().and_then(|w| w.get("url")).and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn row(&self) -> Value {
        let mut row = json!({"group": self.group, "tabs": self.tabs, "owned": self.owned, "used": self.used as i64,
                             "target": self.target.as_ref().map(|(kind, name)| json!([kind, name]))});
        for key in DESK {
            let empty = if key == "hwnd" { Value::Null } else { json!({}) };
            row[key] = self.desk.get(key).cloned().unwrap_or(empty);
        }
        row["closing"] = json!(self.closing as i64);
        row
    }
}

#[derive(Default)]
pub struct Contexts {
    map: HashMap<String, Ctx>,
    side: bool,
    saved: String,
    resets: u64,
}

impl Contexts {
    pub fn side() -> Contexts {
        Contexts { side: true, ..Contexts::default() }
    }

    pub fn get(&mut self, sid: &str) -> &mut Ctx {
        let t = now();
        self.map.retain(|key, c| key.is_empty() || t - c.used <= SESSION_IDLE_S);
        let group = if self.map.contains_key(sid) { String::new() } else { self.free_group(sid) };
        let side = self.side;
        let ctx = self.map.entry(sid.to_string()).or_insert_with(|| Ctx::new(sid, group, side));
        ctx.used = t;
        ctx
    }

    pub fn find(&mut self, sid: &str) -> Option<&mut Ctx> {
        self.map.get_mut(sid)
    }

    fn free_group(&self, sid: &str) -> String {
        if sid.is_empty() {
            return "Lighting".into();
        }
        let taken: HashSet<&str> = self.map.values().map(|c| c.group.as_str()).collect();
        let n = (1..).find(|n| !taken.contains(format!("Lighting #{n}").as_str())).unwrap_or(1);
        format!("Lighting #{n}")
    }

    pub fn note_resets(&mut self, resets: u64) {
        if resets != self.resets {
            self.resets = resets;
            for c in self.map.values_mut() {
                c.tabs.clear();
                c.owned.clear();
            }
        }
    }

    pub fn next_closing(&self) -> Option<f64> {
        self.map.values().map(|c| c.closing).filter(|t| *t > 0.0).min_by(f64::total_cmp)
    }

    pub fn closing_due(&self, t: f64) -> Vec<String> {
        self.map.iter().filter(|(_, c)| c.closing > 0.0 && c.closing <= t).map(|(sid, _)| sid.clone()).collect()
    }

    pub fn save(&mut self, home: &Path, epochs: &Map<String, Value>) {
        let mut sids: Vec<&String> = self.map.keys().filter(|s| !s.is_empty()).collect();
        sids.sort();
        let (mut rows, mut sig) = (Map::new(), Map::new());
        for sid in sids {
            let c = &self.map[sid];
            let row = c.row();
            let mut short = row.clone();
            short["used"] = json!(c.used as i64 / SAVE_STEP_S);
            sig.insert(sid.clone(), short);
            rows.insert(sid.clone(), row);
        }
        let sig = json!([sig, epochs]).to_string();
        if sig == self.saved {
            return;
        }
        self.saved = sig;
        let _ = fs::write(home.join("sessions.json"), json!({"epochs": epochs, "sessions": rows}).to_string());
    }

    pub fn load(&mut self, home: &Path) -> Map<String, Value> {
        let Ok(text) = fs::read_to_string(home.join("sessions.json")) else { return Map::new() };
        let Ok(Value::Object(data)) = serde_json::from_str::<Value>(&text) else { return Map::new() };
        let t = now();
        let strings = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default()
        };
        for (sid, s) in data.get("sessions").and_then(Value::as_object).into_iter().flatten() {
            let used = s.get("used").and_then(Value::as_f64).unwrap_or(0.0);
            if sid.is_empty() || t - used > SESSION_IDLE_S {
                continue;
            }
            let group = s.get("group").and_then(Value::as_str).filter(|g| !g.is_empty()).unwrap_or("Lighting");
            let mut c = Ctx::new(sid, group.to_string(), self.side);
            c.tabs = strings(s.get("tabs"));
            c.owned = strings(s.get("owned")).into_iter().collect();
            c.used = used;
            c.target = match strings(s.get("target")).as_slice() {
                [kind, name] => Some((kind.clone(), name.clone())),
                _ => None,
            };
            for key in DESK {
                if let Some(v) = s.get(key) {
                    c.desk.insert(key.to_string(), v.clone());
                }
            }
            let closing = s.get("closing").and_then(Value::as_f64).unwrap_or(0.0);
            c.closing = if closing > 0.0 { closing.max(t + 1.0) } else { 0.0 };
            self.map.insert(sid.clone(), c);
        }
        data.get("epochs").and_then(Value::as_object).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn repeated_failures_get_a_note_then_a_brake() {
        let mut fails = Fails::default();
        let a = argv(&["click", "e9"]);
        let err = "err: no element e9".to_string();
        assert_eq!(fails.repeated(&a, err.clone(), true).ok().as_deref(), Some(err.as_str()));
        assert_eq!(fails.repeated(&a, err.clone(), true).ok().as_deref(), Some(err.as_str()));
        let third = fails.repeated(&a, err.clone(), true).ok().unwrap();
        assert!(third.ends_with("failed 3 times in a row with this error: repeating it will not help"), "{third}");
        assert!(fails.repeated(&a, err.clone(), true).is_ok());
        let Err((wait, text)) = fails.repeated(&a, err.clone(), true) else { panic!("no brake at 5") };
        assert_eq!(wait, Duration::from_secs(1));
        assert!(text.contains("! STOP: the same command failed 5 times in a row") && text.contains("waited 1 s"));
        let Err((wait, _)) = fails.repeated(&a, err.clone(), true) else { panic!("no brake at 6") };
        assert_eq!(wait, Duration::from_secs(2));
        assert_eq!(fails.repeated(&a, "err: other".into(), true).ok().as_deref(), Some("err: other"));
        assert!(fails.repeated(&a, err.clone(), false).is_ok());
        fails.forget(&a);
        assert!(fails.0.is_empty());
        for i in 0..30 {
            let _ = fails.repeated(&argv(&["x", &i.to_string()]), err.clone(), true);
        }
        assert_eq!(fails.0.len(), FAILS_KEPT);
        assert_eq!(fails.0[0].0, argv(&["x", "10"]));
    }

    #[test]
    fn sessions_get_groups_and_survive_a_restart() {
        let home = std::env::temp_dir().join(format!("lt-ctx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let mut ctxs = Contexts::default();
        assert_eq!(ctxs.get("").group, "Lighting");
        assert_eq!(ctxs.get("a").group, "Lighting #1");
        assert_eq!(ctxs.get("b").group, "Lighting #2");
        let a = ctxs.get("a");
        a.tabs = vec!["t4".into(), "t2".into()];
        a.owned.insert("t4".into());
        a.target = Some(("web".into(), "brave".into()));
        let epochs: Map<String, Value> = [("brave".to_string(), json!(7))].into_iter().collect();
        ctxs.save(&home, &epochs);
        let mut back = Contexts::default();
        assert_eq!(back.load(&home), epochs);
        let a = back.get("a");
        assert_eq!((a.group.as_str(), a.tabs.len(), a.owned.contains("t4")), ("Lighting #1", 2, true));
        assert_eq!(a.target, Some(("web".into(), "brave".into())));
        assert_eq!(back.get("c").group, "Lighting #3");
        back.note_resets(1);
        assert!(back.get("a").tabs.is_empty());
        let saved: Value = serde_json::from_str(&fs::read_to_string(home.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(saved["sessions"]["a"]["windows"], json!({}));
        assert!(saved["sessions"].get("").is_none());
        fs::remove_dir_all(&home).unwrap();
    }
}
