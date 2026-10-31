use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Map, Value, json};
use wisply_core::Fail;
use wisply_proto::Link;

use super::ctx::now;
use super::{VERSION, config};

const EVENTS_KEPT: usize = 30;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn yes(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f.abs() > 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(yes)
}

pub enum Called {
    Timeout,
    Aborted,
}

pub struct Host {
    pub brand: String,
    pub connected: f64,
    focused: Mutex<f64>,
    info: Mutex<Map<String, Value>>,
    link: Link,
    sending: Mutex<()>,
    pending: Mutex<HashMap<u64, SyncSender<Value>>>,
    seq: AtomicU64,
    alive: AtomicBool,
}

impl Host {
    fn new(link: Link, hello: Map<String, Value>) -> Host {
        let brand = hello.get("brand").and_then(Value::as_str).filter(|b| !b.is_empty()).unwrap_or("browser");
        let t = now();
        Host {
            brand: brand.to_lowercase(),
            connected: t,
            focused: Mutex::new(if truthy(hello.get("focused")) { t } else { 0.0 }),
            info: Mutex::new(hello),
            link,
            sending: Mutex::new(()),
            pending: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(0),
            alive: AtomicBool::new(true),
        }
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn focused(&self) -> f64 {
        *lock(&self.focused)
    }

    pub fn info(&self, key: &str) -> Option<String> {
        lock(&self.info).get(key).map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }

    fn send(&self, msg: &Value) -> bool {
        let _turn = lock(&self.sending);
        self.link.send(msg.to_string().as_bytes()).is_ok()
    }

    pub fn call(
        &self,
        cmd: &str,
        args: Value,
        tab: Option<&str>,
        timeout: Duration,
        group: Option<&str>,
    ) -> Result<Value, Called> {
        let id = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = mpsc::sync_channel(1);
        lock(&self.pending).insert(id, tx);
        let msg = json!({"id": id, "cmd": cmd, "args": args, "tab": tab, "group": group});
        let got = if self.send(&msg) {
            rx.recv_timeout(timeout).ok()
        } else {
            Some(json!({"ok": false, "error": "browser disconnected"}))
        };
        lock(&self.pending).remove(&id);
        match got {
            None => Err(Called::Timeout),
            Some(reply) if truthy(reply.get("abort")) => Err(Called::Aborted),
            Some(reply) => Ok(reply),
        }
    }

    pub fn post(&self, cmd: &str, args: Value) {
        let _ = self.send(&json!({"id": 0, "cmd": cmd, "args": args}));
    }

    fn fail_all(&self, msg: &Value) {
        for tx in lock(&self.pending).values() {
            let _ = tx.try_send(msg.clone());
        }
    }
}

#[derive(Default)]
pub struct Hosts {
    list: Mutex<Vec<Arc<Host>>>,
    events: Mutex<VecDeque<String>>,
    epochs: Mutex<Map<String, Value>>,
    resets: AtomicU64,
    reloaded: Mutex<HashSet<String>>,
}

impl Hosts {
    pub fn all(&self) -> Vec<Arc<Host>> {
        lock(&self.list).clone()
    }

    pub fn alive(&self) -> Vec<Arc<Host>> {
        lock(&self.list).iter().filter(|h| h.alive()).cloned().collect()
    }

    pub fn resets(&self) -> u64 {
        self.resets.load(Ordering::SeqCst)
    }

    pub fn epochs(&self) -> Map<String, Value> {
        lock(&self.epochs).clone()
    }

    pub fn set_epochs(&self, epochs: Map<String, Value>) {
        *lock(&self.epochs) = epochs;
    }

    pub fn reloaded(&self, brand: &str) -> bool {
        lock(&self.reloaded).contains(brand)
    }

    pub fn events(&self) -> Vec<String> {
        lock(&self.events).iter().cloned().collect()
    }

    pub fn event(&self, text: String) {
        let mut events = lock(&self.events);
        if events.len() >= EVENTS_KEPT {
            events.pop_front();
        }
        events.push_back(text);
    }

    pub fn take_events(&self) -> Vec<String> {
        lock(&self.events).drain(..).collect()
    }

    pub fn pick(&self, pref: &str, quiet: bool) -> Result<Option<Arc<Host>>, Fail> {
        let hosts = self.alive();
        if hosts.is_empty() {
            return Ok(None);
        }
        if pref != "auto" {
            if let Some(h) = hosts.iter().find(|h| h.brand.contains(pref)) {
                return Ok(Some(Arc::clone(h)));
            }
            if !quiet {
                let names: Vec<&str> = hosts.iter().map(|h| h.brand.as_str()).collect();
                return Err(Fail::hint(
                    format!("{pref} is not connected (connected: {})", names.join(", ")),
                    "wisply config browser auto",
                ));
            }
        }
        let mut best: Option<Arc<Host>> = None;
        for h in hosts {
            if best.as_ref().is_none_or(|b| (h.focused(), h.connected) > (b.focused(), b.connected)) {
                best = Some(h);
            }
        }
        Ok(best)
    }

    pub fn abort(&self) {
        for host in self.all() {
            host.fail_all(&json!({"ok": false, "abort": true}));
            host.post("abort", json!({}));
        }
    }

    pub fn push_config(&self, home: &Path) {
        let payload = config::payload(home);
        for host in self.all() {
            host.post("config", payload.clone());
        }
    }

    pub fn serve(&self, home: &Path, link: Link, hello: Map<String, Value>) {
        let epoch = hello.get("epoch").cloned();
        let host = Arc::new(Host::new(link, hello));
        self.note_epoch(&host.brand, epoch.as_ref());
        lock(&self.list).push(Arc::clone(&host));
        self.on_connect(home, &host);
        while let Ok(raw) = host.link.recv() {
            let Ok(Value::Object(msg)) = serde_json::from_slice::<Value>(&raw) else { break };
            let id = msg.get("id").and_then(Value::as_u64).unwrap_or(0);
            if id != 0 {
                if let Some(tx) = lock(&host.pending).get(&id) {
                    let _ = tx.try_send(Value::Object(msg));
                }
            } else if truthy(msg.get("event")) {
                self.on_event(&host, msg);
            }
        }
        host.alive.store(false, Ordering::SeqCst);
        host.fail_all(&json!({"ok": false, "error": "browser disconnected"}));
        lock(&self.list).retain(|h| !Arc::ptr_eq(h, &host));
    }

    fn on_connect(&self, home: &Path, host: &Host) {
        host.post("config", config::payload(home));
        let outdated = host.info("ext").is_some_and(|ext| !ext.is_empty() && ext != wisply_core::ext_version(VERSION));
        if outdated
            && home.join("extension").join("manifest.json").exists()
            && lock(&self.reloaded).insert(host.brand.clone())
        {
            host.post("reload-extension", json!({}));
        }
    }

    fn on_event(&self, host: &Host, msg: Map<String, Value>) {
        match msg.get("event").and_then(Value::as_str) {
            Some("focus") => {
                if truthy(msg.get("focused")) {
                    *lock(&host.focused) = now();
                }
            }
            Some("hello") => {
                let epoch = msg.get("epoch").cloned();
                lock(&host.info).extend(msg);
                self.note_epoch(&host.brand, epoch.as_ref());
            }
            _ => {
                if let Some(text) = msg.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                    let mut events = lock(&self.events);
                    if events.len() >= EVENTS_KEPT {
                        events.pop_front();
                    }
                    events.push_back(text.to_string());
                }
            }
        }
    }

    fn note_epoch(&self, brand: &str, epoch: Option<&Value>) {
        let Some(epoch) = epoch.filter(|e| yes(e)) else { return };
        let mut epochs = lock(&self.epochs);
        if epochs.get(brand).is_some_and(|old| old != epoch) {
            self.resets.fetch_add(1, Ordering::SeqCst);
        }
        epochs.insert(brand.to_string(), epoch.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truth_follows_python() {
        for v in [json!(null), json!(false), json!(0), json!(0.0), json!(""), json!([]), json!({})] {
            assert!(!yes(&v), "{v}");
        }
        for v in [json!(true), json!(2), json!(-0.5), json!("x"), json!([0]), json!({"a": 0})] {
            assert!(yes(&v), "{v}");
        }
        assert!(!truthy(None));
    }
}
