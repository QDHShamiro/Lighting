use std::env;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use lt_memory::{Store, claude_dirs};

use super::super::{config, game};
use super::{embed, watch};

const EVERY: Duration = Duration::from_secs(300);
const RETRY: Duration = Duration::from_secs(2);
const VAULT_WATCHED: Duration = Duration::from_secs(3600);
const SETTLE: Duration = Duration::from_secs(3);
const EMBED_PASS: Duration = Duration::from_secs(50);
const EMBED_WAIT: Duration = Duration::from_secs(30);
const EMBED_BATCH: usize = 32;
const VAULT_DIR: &str = "vault";

static WAKE: OnceLock<SyncSender<()>> = OnceLock::new();
static PROBLEM: Mutex<String> = Mutex::new(String::new());

#[derive(Default)]
struct Last {
    vault: Option<Instant>,
    claude: Option<Instant>,
}

fn due(last: Option<Instant>, every: Duration) -> bool {
    last.is_none_or(|t| t.elapsed() >= every)
}

pub fn start(home: PathBuf) {
    let (tx, rx) = mpsc::sync_channel(1);
    if WAKE.set(tx).is_err() {
        return;
    }
    let vault_every = if watch::watch(&vault(&home), nudge) { VAULT_WATCHED } else { EVERY };
    let work = move || {
        let (mut store, mut last, mut force) = (None, Last::default(), true);
        loop {
            let wait = match pass(&home, &mut store, &mut last, force, vault_every) {
                Ok(()) => EVERY,
                Err(why) => {
                    store = None;
                    if let Ok(mut p) = PROBLEM.lock() {
                        *p = why;
                    }
                    RETRY
                }
            };
            force = match rx.recv_timeout(wait) {
                Ok(()) => {
                    while rx.recv_timeout(SETTLE).is_ok() {}
                    true
                }
                Err(RecvTimeoutError::Timeout) => store.is_none(),
                Err(RecvTimeoutError::Disconnected) => return,
            };
        }
    };
    let _ = thread::Builder::new().name("index".into()).spawn(work);
}

pub fn nudge() {
    if let Some(tx) = WAKE.get() {
        let _ = tx.try_send(());
    }
}

pub fn problem() -> String {
    PROBLEM.lock().map(|p| p.clone()).unwrap_or_default()
}

fn user_home() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

pub fn vault(home: &Path) -> PathBuf {
    let own = config::text(&config::load(home), "vault");
    if own.trim().is_empty() { home.join(VAULT_DIR) } else { PathBuf::from(own.trim()) }
}

fn pass(
    home: &Path,
    slot: &mut Option<Store>,
    last: &mut Last,
    force: bool,
    vault_every: Duration,
) -> Result<(), String> {
    let cfg = config::load(home);
    if !config::on(&cfg, "memory") {
        return Ok(());
    }
    if slot.is_none() {
        *slot = Some(Store::open(home).map_err(|e| format!("cannot open memory.db ({e})"))?);
    }
    let Some(store) = slot.as_mut() else { return Ok(()) };
    if force || due(last.claude, EVERY) {
        store.index_claude(&claude_dirs(&user_home())).map_err(|e| format!("Claude memory index: {e}"))?;
        last.claude = Some(Instant::now());
    }
    if force || due(last.vault, vault_every) {
        let inbox = config::text(&cfg, "vault_inbox");
        store.index_vault(&vault(home), &inbox).map_err(|e| format!("vault index: {e}"))?;
        last.vault = Some(Instant::now());
    }
    if let Ok(mut p) = PROBLEM.lock() {
        p.clear();
    }
    embed_some(home, store);
    Ok(())
}

fn embed_some(home: &Path, store: &mut Store) {
    let end = Instant::now() + EMBED_PASS;
    while Instant::now() < end {
        let Ok(todo) = store.unembedded(EMBED_BATCH) else { break };
        if todo.is_empty() || !game::running().is_empty() {
            break;
        }
        let texts: Vec<String> = todo.iter().map(|(_, t)| t.clone()).collect();
        let Ok(vecs) = embed::embed(home, &texts, false, EMBED_WAIT) else { break };
        let pairs: Vec<(i64, Vec<f32>)> = todo.iter().map(|(id, _)| *id).zip(vecs).collect();
        if store.save_vecs(&pairs).is_err() {
            break;
        }
    }
}
