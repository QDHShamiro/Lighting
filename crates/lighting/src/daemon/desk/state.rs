use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Map, Value, json};

use super::super::ctx::{Ctx, SESSION_IDLE_S, now};
use super::uia::El;

#[derive(Default)]
pub struct State {
    pub windows: BTreeMap<String, isize>,
    pub info: BTreeMap<String, (String, String)>,
    pub hwnd: isize,
    pub launched: BTreeMap<isize, String>,
    pub front0: isize,
    pub refs: Vec<(String, El)>,
    pub ids: HashMap<String, String>,
    pub seq: u64,
    pub ocr: Vec<(String, String, i32, i32)>,
    pub snap_t: f64,
    pub snap_hwnd: isize,
    pub slow: HashSet<isize>,
    pub last_snap: HashMap<isize, String>,
    pub last_lines: HashMap<isize, (f64, Vec<String>)>,
    pub full: Option<(isize, f64, Vec<String>)>,
    pub seen_front: isize,
    used: f64,
}

thread_local! {
    static STATES: RefCell<HashMap<String, State>> = RefCell::new(HashMap::new());
}

fn int(v: &Value) -> Option<isize> {
    v.as_i64().map(|n| n as isize).or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

impl State {
    fn load(desk: &Map<String, Value>) -> State {
        let obj = |key: &str| desk.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
        let mut st = State {
            windows: obj("windows").iter().filter_map(|(k, v)| Some((k.clone(), int(v)?))).collect(),
            hwnd: desk.get("hwnd").and_then(int).unwrap_or(0),
            ..State::default()
        };
        for (k, v) in obj("info") {
            let pair = v.as_array().map(|a| a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect::<Vec<_>>());
            if let Some([exe, title]) = pair.as_deref() {
                st.info.insert(k, (exe.clone(), title.clone()));
            }
        }
        for (k, v) in obj("launched") {
            if let Ok(h) = k.parse() {
                st.launched.insert(h, v.as_str().unwrap_or("").to_string());
            }
        }
        st
    }

    fn save(&self, desk: &mut Map<String, Value>) {
        desk.insert("windows".into(), json!(self.windows));
        desk.insert("hwnd".into(), if self.hwnd == 0 { Value::Null } else { json!(self.hwnd) });
        let info: Map<String, Value> = self.info.iter().map(|(k, (e, t))| (k.clone(), json!([e, t]))).collect();
        desk.insert("info".into(), Value::Object(info));
        let launched: Map<String, Value> = self.launched.iter().map(|(h, n)| (h.to_string(), json!(n))).collect();
        desk.insert("launched".into(), Value::Object(launched));
    }

    pub fn next_ref(&mut self, key: String) -> String {
        if let Some(r) = self.ids.get(&key) {
            return r.clone();
        }
        self.seq += 1;
        let r = format!("d{}", self.seq);
        self.ids.insert(key, r.clone());
        r
    }

    pub fn window_ref(&self, hwnd: isize) -> Option<String> {
        self.windows.iter().find(|(_, h)| **h == hwnd).map(|(k, _)| k.clone())
    }
}

pub fn with<T>(ctx: &mut Ctx, f: impl FnOnce(&mut State, &mut Ctx) -> T) -> T {
    STATES.with(|cell| {
        let mut map = cell.borrow_mut();
        let t = now();
        map.retain(|_, s| t - s.used <= SESSION_IDLE_S);
        let st = map.entry(ctx.sid.clone()).or_insert_with(|| State::load(&ctx.desk));
        st.used = t;
        let out = f(st, ctx);
        st.save(&mut ctx.desk);
        out
    })
}
