mod brain;
mod channels;
mod claude;
mod config;
mod ctx;
mod desk;
mod doctor;
mod flow;
mod game;
mod grep;
mod hosts;
mod lane;
mod media;
mod memory;
mod phone;
mod record;
mod route;
mod routines;
mod secure;
mod squeeze;
mod system;
mod tokens;
mod web;
mod words;

use std::any::Any;
use std::backtrace::Backtrace;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::panic;
use std::path::PathBuf;
use std::process::{self, ExitCode};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lt_core::{Fail, Lane, Log, local_now};
use lt_proto::{Incoming, Link, Listener, Reply};
use serde_json::{Map, Value, json};

use ctx::{Contexts, Ctx};
use hosts::Hosts;

pub const MARKER: &str = "v3";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const FIRST_MESSAGE: Duration = Duration::from_secs(10);
const STOPPED: &str = "stopped (Ctrl+Alt+End or lighting abort)";
const ABORTED: &str = "stopped the running and queued commands";
const LANES: [Lane; 3] = [Lane::Main, Lane::Memory, Lane::Hook];

pub type Prompts = Vec<(f64, Vec<String>)>;

pub struct Daemon {
    pub home: PathBuf,
    pub pid: u32,
    pub started: Instant,
    pub abort: AtomicBool,
    pub hosts: Hosts,
    pub seen: Mutex<HashSet<String>>,
    pub prompts: Mutex<HashMap<String, Prompts>>,
    token: String,
    log: Log,
    next_id: AtomicU64,
    generation: AtomicU64,
}

pub enum Out {
    Text(String),
    Stop(String),
    Wait(Pending),
}

type Poll = Box<dyn FnMut(&Daemon, &mut Ctx, bool) -> Result<Option<String>, Fail> + Send>;

pub struct Pending {
    due: Instant,
    deadline: Instant,
    interval: Duration,
    poll: Poll,
}

impl Pending {
    pub fn new(
        timeout: Duration,
        interval: Duration,
        poll: impl FnMut(&Daemon, &mut Ctx, bool) -> Result<Option<String>, Fail> + Send + 'static,
    ) -> Pending {
        let now = Instant::now();
        Pending { due: now + interval, deadline: now + timeout, interval, poll: Box::new(poll) }
    }

    fn said_after(wait: Duration, text: String) -> Pending {
        Pending::new(wait, wait, move |_, _, last| if last { Err(Fail::said(text.clone())) } else { Ok(None) })
    }
}

pub struct Req {
    pub argv: Vec<String>,
    pub sid: String,
    pub cut: usize,
    pub stages: Vec<(&'static str, u64)>,
    pub used: Option<Vec<String>>,
    secret: Option<String>,
    cwd: String,
    id: String,
    generation: u64,
    recv: Instant,
    started: Instant,
    reply: SyncSender<Res>,
}

struct Res {
    code: i32,
    out: String,
    image: String,
    stop: bool,
}

impl Res {
    fn new(code: i32, out: impl Into<String>) -> Res {
        Res { code, out: out.into(), image: String::new(), stop: false }
    }
}

enum Work {
    Job(Box<Req>),
    Wake,
}

enum Msg {
    Cmd(Incoming, bool),
    Hello(Map<String, Value>),
}

pub use doctor::run as doctor;
#[cfg(windows)]
pub use media::tools::{ffmpeg, run as run_tool, which};
#[cfg(windows)]
pub use media::{multipart, spoken, whisper_service};
#[cfg(windows)]
pub use memory::chat::api_key;
pub use squeeze::{MAX_BYTES as SQUEEZE_MAX_BYTES, USAGE as SQUEEZE_USAGE};

pub fn may_serve() -> bool {
    let home = lt_proto::home();
    !lt_proto::is_default_home(&home) || home.join(MARKER).is_file()
}

pub fn serve() -> ExitCode {
    let home = lt_proto::home();
    if let Err(e) = fs::create_dir_all(&home) {
        eprintln!("lighting daemon: cannot use {} ({e})", home.display());
        return ExitCode::FAILURE;
    }
    let mut listener = match Listener::bind(&home) {
        Ok(listener) => listener,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lighting daemon: cannot listen ({e})");
            return ExitCode::FAILURE;
        }
    };
    let token = match lt_proto::make_key(&home) {
        Ok(token) => token,
        Err(e) => {
            eprintln!("lighting daemon: cannot make the key in {} ({e})", home.display());
            return ExitCode::FAILURE;
        }
    };
    let _ = fs::write(home.join(MARKER), VERSION);
    let pid = process::id();
    let _ = fs::write(home.join("daemon.pid"), pid.to_string());
    eprintln!("lighting daemon started {VERSION} {} pid {pid}", local_now());
    panic::set_hook(Box::new(|info| {
        let thread = thread::current();
        let name = thread.name().unwrap_or("?");
        eprintln!("{} panic in {name}: {info}\n{}", local_now(), Backtrace::force_capture());
    }));
    let mut sessions = Contexts::default();
    let hosts = Hosts::default();
    hosts.set_epochs(sessions.load(&home));
    let d = Arc::new(Daemon {
        log: Log::new(&home),
        home,
        pid,
        started: Instant::now(),
        abort: AtomicBool::new(false),
        hosts,
        seen: Mutex::new(HashSet::new()),
        prompts: Mutex::new(HashMap::new()),
        token,
        next_id: AtomicU64::new(0),
        generation: AtomicU64::new(0),
    });
    memory::index::start(d.home.clone());
    let mut sessions = Some(sessions);
    let mut senders = Vec::with_capacity(LANES.len());
    for lane in LANES {
        let (tx, rx) = mpsc::channel();
        let d = Arc::clone(&d);
        let main = lane == Lane::Main;
        let ctxs = if main { sessions.take().unwrap_or_default() } else { Contexts::side() };
        let name = format!("{lane:?}").to_lowercase();
        if thread::Builder::new().name(name).spawn(move || lane::work(&d, &rx, ctxs, main)).is_err() {
            eprintln!("lighting daemon: cannot start the {lane:?} lane");
            return ExitCode::FAILURE;
        }
        senders.push(tx);
    }
    let lanes: Arc<[Sender<Work>]> = senders.into();
    loop {
        match listener.accept() {
            Ok(link) => {
                let (d, lanes) = (Arc::clone(&d), Arc::clone(&lanes));
                let _ = thread::Builder::new().name("conn".into()).spawn(move || handle(&d, link, &lanes));
            }
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn handle(d: &Daemon, link: Link, lanes: &[Sender<Work>]) {
    let Ok(raw) = link.recv_within(FIRST_MESSAGE) else { return };
    let (msg, fast) = match decode(&raw) {
        Some(Msg::Cmd(msg, fast)) => (msg, fast),
        Some(Msg::Hello(mut hello)) => {
            let token = hello.remove("token");
            if lt_proto::same_token(token.as_ref().and_then(Value::as_str).unwrap_or(""), &d.token) {
                d.hosts.serve(&d.home, link, hello);
            }
            return;
        }
        None => return,
    };
    if !lt_proto::same_token(&msg.token, &d.token) {
        return;
    }
    let name = msg.argv.first().map(|a| a.to_lowercase()).unwrap_or_default();
    let res = if name == "abort" {
        d.abort_all(lanes);
        Res::new(0, ABORTED)
    } else {
        let (reply, answer) = mpsc::sync_channel(1);
        let now = Instant::now();
        let req = Req {
            argv: msg.argv,
            sid: msg.sid,
            cut: 0,
            stages: Vec::new(),
            used: None,
            secret: msg.secret,
            cwd: msg.cwd,
            id: format!("{}.{}", d.pid, d.next_id.fetch_add(1, Ordering::Relaxed) + 1),
            generation: d.generation(),
            recv: now,
            started: now,
            reply,
        };
        let lane = LANES.iter().position(|l| *l == lt_core::lane(&name)).unwrap_or(0);
        if lanes[lane].send(Work::Job(Box::new(req))).is_ok() {
            answer.recv().unwrap_or_else(|_| Res::new(1, "err: the command ended without an answer, see daemon.log"))
        } else {
            Res::new(1, "err: the lighting daemon is stopping")
        }
    };
    let bytes = if fast {
        Reply { code: res.code, image: res.image, out: res.out }.encode()
    } else {
        let mut v = json!({"out": res.out, "code": res.code});
        if !res.image.is_empty() {
            v["image"] = json!(res.image);
        }
        v.to_string().into_bytes()
    };
    if res.stop {
        d.release();
    }
    let _ = link.send(&bytes);
    drop(link);
    if res.stop {
        process::exit(0);
    }
}

fn decode(raw: &[u8]) -> Option<Msg> {
    if let Some(msg) = Incoming::decode(raw) {
        return Some(Msg::Cmd(msg, true));
    }
    let v: Value = serde_json::from_slice(raw).ok()?;
    if v.get("type").and_then(Value::as_str) == Some("hello") {
        return match v {
            Value::Object(hello) => Some(Msg::Hello(hello)),
            _ => None,
        };
    }
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(String::from);
    let argv = v.get("argv")?.as_array()?.iter().map(|a| a.as_str().map(String::from)).collect::<Option<Vec<_>>>()?;
    let msg = Incoming {
        token: text("token")?,
        cwd: text("cwd").unwrap_or_default(),
        sid: text("sid").unwrap_or_default(),
        secret: text("secret"),
        argv,
    };
    Some(Msg::Cmd(msg, false))
}

impl Daemon {
    pub fn prompts_of(&self, sid: &str) -> Prompts {
        self.prompts.lock().unwrap_or_else(|e| e.into_inner()).get(sid).cloned().unwrap_or_default()
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn abort_all(&self, lanes: &[Sender<Work>]) {
        self.abort.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.hosts.abort();
        for lane in lanes {
            let _ = lane.send(Work::Wake);
        }
    }

    fn release(&self) {
        let _ = fs::remove_file(self.home.join("daemon.pid"));
        if cfg!(not(windows)) {
            let _ = fs::remove_file(lt_proto::address(&self.home));
        }
    }
}

fn panic_text(panic: &(dyn Any + Send)) -> String {
    let text = match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
        (Some(s), _) => (*s).to_string(),
        (_, Some(s)) => s.clone(),
        _ => "unknown".into(),
    };
    text.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_wire_formats_are_read() {
        let Some(Msg::Cmd(msg, fast)) = decode(b"L2\0ab\0C:/x\0s1\0-\0ping") else { panic!("no L2") };
        assert!(fast && msg.argv == ["ping"] && msg.sid == "s1" && msg.cwd == "C:/x");
        let raw = br#"{"token":"ab","argv":["recall","x"],"sid":"s2","secret":"k"}"#;
        let Some(Msg::Cmd(msg, fast)) = decode(raw) else { panic!("no json") };
        assert!(!fast && msg.argv == ["recall", "x"] && msg.secret.as_deref() == Some("k") && msg.cwd.is_empty());
        let Some(Msg::Hello(hello)) = decode(br#"{"type":"hello","token":"ab","brand":"Brave"}"#) else {
            panic!("no hello")
        };
        assert_eq!(hello.get("brand"), Some(&json!("Brave")));
        assert!(decode(br#"{"argv":["ping"]}"#).is_none());
        assert!(decode(br#"{"token":"ab","argv":[1]}"#).is_none());
        assert!(decode(b"garbage").is_none());
    }

    #[test]
    fn panics_are_named() {
        let caught = panic::catch_unwind(|| panic!("boom {}", 7)).unwrap_err();
        assert_eq!(panic_text(&*caught), "boom 7");
        let caught = panic::catch_unwind(|| panic!("static")).unwrap_err();
        assert_eq!(panic_text(&*caught), "static");
    }
}
