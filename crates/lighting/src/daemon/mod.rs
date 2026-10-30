mod system;

use std::any::Any;
use std::backtrace::Backtrace;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::{self, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use lt_core::{Fail, Lane, Log, QUIET, Row, local_now};
use lt_proto::{Incoming, Link, Listener, Reply};
use serde_json::{Value, json};

pub const MARKER: &str = "v3";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const FIRST_MESSAGE: Duration = Duration::from_secs(10);
const SESSION_IDLE: Duration = Duration::from_secs(12 * 3600);
const REPEAT_NOTE_AT: u32 = 3;
const REPEAT_BRAKE_AT: u32 = 5;
const REPEAT_BRAKE_MAX_S: u64 = 30;
const REPEAT_WINDOW: Duration = Duration::from_secs(120);
const FAILS_KEPT: usize = 20;
const STOPPED: &str = "stopped (Ctrl+Alt+End or lighting abort)";
const ABORTED: &str = "stopped the running and queued commands";
const LANES: [Lane; 3] = [Lane::Main, Lane::Memory, Lane::Hook];

pub struct Daemon {
    pub home: PathBuf,
    pub pid: u32,
    pub started: Instant,
    pub abort: AtomicBool,
    token: String,
    log: Log,
    next_id: AtomicU64,
    generation: AtomicU64,
}

pub enum Out {
    Text(String),
    Stop(String),
}

pub struct Pending {
    due: Instant,
    deadline: Instant,
    interval: Duration,
    poll: Box<dyn FnMut(bool) -> Result<Option<String>, Fail> + Send>,
}

impl Pending {
    pub fn new(
        timeout: Duration,
        interval: Duration,
        poll: impl FnMut(bool) -> Result<Option<String>, Fail> + Send + 'static,
    ) -> Pending {
        let now = Instant::now();
        Pending { due: now + interval, deadline: now + timeout, interval, poll: Box::new(poll) }
    }

    fn said_after(wait: Duration, text: String) -> Pending {
        Pending::new(wait, wait, move |last| if last { Err(Fail::said(text.clone())) } else { Ok(None) })
    }
}

pub struct Req {
    pub argv: Vec<String>,
    pub sid: String,
    pub cut: usize,
    pub stages: Vec<(&'static str, u64)>,
    id: String,
    generation: u64,
    recv: Instant,
    started: Instant,
    reply: SyncSender<Res>,
}

struct Res {
    code: i32,
    out: String,
    stop: bool,
}

impl Res {
    fn new(code: i32, out: impl Into<String>) -> Res {
        Res { code, out: out.into(), stop: false }
    }
}

enum Work {
    Job(Req),
    Wake,
}

struct Wait {
    pending: Pending,
    req: Req,
    name: String,
}

#[derive(Default)]
struct Fails(Vec<(Vec<String>, String, u32, Instant)>);

impl Fails {
    fn forget(&mut self, argv: &[String]) {
        self.0.retain(|(key, ..)| key != argv);
    }

    fn repeated(&mut self, argv: &[String], out: String, brake: bool) -> Result<String, Pending> {
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
            return Err(Pending::said_after(Duration::from_secs(wait), text));
        }
        if n >= REPEAT_NOTE_AT {
            return Ok(format!(
                "{out}\n! the same command failed {n} times in a row with this error: repeating it will not help"
            ));
        }
        Ok(out)
    }
}

#[derive(Default)]
struct Sessions(HashMap<String, (Fails, Instant)>);

impl Sessions {
    fn fails(&mut self, sid: &str) -> &mut Fails {
        let now = Instant::now();
        self.0.retain(|key, (_, used)| key.is_empty() || now.duration_since(*used) < SESSION_IDLE);
        let entry = self.0.entry(sid.to_string()).or_insert_with(|| (Fails::default(), now));
        entry.1 = now;
        &mut entry.0
    }
}

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
    let d = Arc::new(Daemon {
        log: Log::new(&home),
        home,
        pid,
        started: Instant::now(),
        abort: AtomicBool::new(false),
        token,
        next_id: AtomicU64::new(0),
        generation: AtomicU64::new(0),
    });
    let mut senders = Vec::with_capacity(LANES.len());
    for lane in LANES {
        let (tx, rx) = mpsc::channel();
        let d = Arc::clone(&d);
        let name = format!("{lane:?}").to_lowercase();
        if thread::Builder::new().name(name).spawn(move || work(&d, &rx, lane == Lane::Main)).is_err() {
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
    let Some((msg, fast)) = decode(&raw) else { return };
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
            id: format!("{}.{}", d.pid, d.next_id.fetch_add(1, Ordering::Relaxed) + 1),
            generation: d.generation(),
            recv: now,
            started: now,
            reply,
        };
        let lane = LANES.iter().position(|l| *l == lt_core::lane(&name)).unwrap_or(0);
        if lanes[lane].send(Work::Job(req)).is_ok() {
            answer.recv().unwrap_or_else(|_| Res::new(1, "err: the command ended without an answer, see daemon.log"))
        } else {
            Res::new(1, "err: the lighting daemon is stopping")
        }
    };
    let bytes = if fast {
        Reply { code: res.code, image: String::new(), out: res.out }.encode()
    } else {
        json!({"out": res.out, "code": res.code}).to_string().into_bytes()
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

fn decode(raw: &[u8]) -> Option<(Incoming, bool)> {
    if let Some(msg) = Incoming::decode(raw) {
        return Some((msg, true));
    }
    let v: Value = serde_json::from_slice(raw).ok()?;
    if v.get("type").and_then(Value::as_str) == Some("hello") {
        return None;
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
    Some((msg, false))
}

fn work(d: &Daemon, jobs: &Receiver<Work>, main: bool) {
    let mut sessions = Sessions::default();
    let mut waits: Vec<Wait> = Vec::new();
    loop {
        let next = waits.iter().map(|w| w.pending.due).min();
        let got = match next {
            Some(due) => match jobs.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(work) => Some(work),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match jobs.recv() {
                Ok(work) => Some(work),
                Err(_) => return,
            },
        };
        if let Some(Work::Job(req)) = got {
            if main && req.generation != d.generation() {
                let _ = req.reply.send(Res::new(1, STOPPED));
            } else {
                if main {
                    d.abort.store(false, Ordering::SeqCst);
                }
                waits.extend(d.run(&mut sessions, req));
            }
        }
        let (now, generation) = (Instant::now(), d.generation());
        let (due, later): (Vec<Wait>, Vec<Wait>) =
            waits.drain(..).partition(|w| w.pending.due <= now || w.req.generation != generation);
        waits = later;
        waits.extend(due.into_iter().filter_map(|w| d.poll(w)));
    }
}

impl Daemon {
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn abort_all(&self, lanes: &[Sender<Work>]) {
        self.abort.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        for lane in lanes {
            let _ = lane.send(Work::Wake);
        }
    }

    fn run(&self, sessions: &mut Sessions, mut req: Req) -> Option<Wait> {
        req.started = Instant::now();
        let raw = req.argv.first().cloned().unwrap_or_default();
        let brake = !QUIET.contains(&raw.to_lowercase().as_str());
        let done = panic::catch_unwind(AssertUnwindSafe(|| system::run_one(self, &mut req)));
        let fails = sessions.fails(&req.sid);
        let (out, code, name, stop) = match done {
            Ok(Ok((Out::Text(text), name))) => {
                fails.forget(&req.argv);
                (text, 0, name, false)
            }
            Ok(Ok((Out::Stop(text), name))) => (text, 0, name, true),
            Ok(Err(fail)) => match fails.repeated(&req.argv, fail.text(), brake) {
                Ok(text) => (text, 1, raw, false),
                Err(pending) => return Some(Wait { pending, req, name: raw }),
            },
            Err(panic) => (format!("err: panic: {}", panic_text(&*panic)), 1, raw, false),
        };
        if name == "suggest" {
            let _ = req.reply.send(Res::new(0, if code == 0 { out } else { String::new() }));
            return None;
        }
        self.finish(req, &name, out, code, stop);
        None
    }

    fn poll(&self, mut wait: Wait) -> Option<Wait> {
        if wait.req.generation != self.generation() {
            self.finish(wait.req, &wait.name, STOPPED.into(), 1, false);
            return None;
        }
        let last = Instant::now() >= wait.pending.deadline;
        let polled = panic::catch_unwind(AssertUnwindSafe(|| (wait.pending.poll)(last)));
        let (out, code) = match polled {
            Ok(Ok(None)) if !last => {
                wait.pending.due = Instant::now() + wait.pending.interval;
                return Some(wait);
            }
            Ok(Ok(text)) => (text.unwrap_or_default(), 0),
            Ok(Err(fail)) => (fail.text(), 1),
            Err(panic) => (format!("err: panic: {}", panic_text(&*panic)), 1),
        };
        self.finish(wait.req, &wait.name, out, code, false);
        None
    }

    fn finish(&self, req: Req, name: &str, out: String, code: i32, stop: bool) {
        let row = Row {
            name,
            argv: &req.argv,
            code,
            ms: req.started.elapsed().as_millis() as u64,
            out: &out,
            test: false,
            cut: req.cut,
            sid: &req.sid,
            id: &req.id,
            queued_ms: req.started.duration_since(req.recv).as_millis() as u64,
            stages: &req.stages,
        };
        self.log.write(&row);
        let _ = req.reply.send(Res { code, out, stop });
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
        let Err(mut brake) = fails.repeated(&a, err.clone(), true) else { panic!("no brake at 5") };
        assert!(brake.deadline.duration_since(brake.due) < Duration::from_millis(5));
        let said = (brake.poll)(true).unwrap_err();
        assert!(
            said.said && said.text().contains("! STOP: the same command failed 5 times in a row"),
            "{}",
            said.text()
        );
        assert!(said.text().contains("waited 1 s"));
        let Err(next) = fails.repeated(&a, err.clone(), true) else { panic!("no brake at 6") };
        assert!(next.deadline.duration_since(Instant::now()) > Duration::from_millis(1500));
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
    fn both_wire_formats_are_read() {
        let (msg, fast) = decode(b"L2\0ab\0C:/x\0s1\0-\0ping").unwrap();
        assert!(fast && msg.argv == ["ping"] && msg.sid == "s1");
        let (msg, fast) = decode(br#"{"token":"ab","argv":["recall","x"],"sid":"s2","secret":"k"}"#).unwrap();
        assert!(!fast && msg.argv == ["recall", "x"] && msg.secret.as_deref() == Some("k") && msg.cwd.is_empty());
        assert!(decode(br#"{"type":"hello","token":"ab"}"#).is_none());
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
