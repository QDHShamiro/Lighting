use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use lt_memory::{Store, claude_dirs};

use super::super::{config, game};
use super::embed;

const EVERY: Duration = Duration::from_secs(300);
const EMBED_PASS: Duration = Duration::from_secs(50);
const EMBED_WAIT: Duration = Duration::from_secs(30);
const EMBED_BATCH: usize = 32;
const VAULT_DIR: &str = "vault";

static WAKE: OnceLock<SyncSender<()>> = OnceLock::new();

pub fn start(home: PathBuf) {
    let (tx, rx) = mpsc::sync_channel(1);
    if WAKE.set(tx).is_err() {
        return;
    }
    let work = move || {
        let mut store = None;
        loop {
            pass(&home, &mut store);
            if let Err(RecvTimeoutError::Disconnected) = rx.recv_timeout(EVERY) {
                return;
            }
        }
    };
    let _ = thread::Builder::new().name("index".into()).spawn(work);
}

pub fn nudge() {
    if let Some(tx) = WAKE.get() {
        let _ = tx.try_send(());
    }
}

fn user_home() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

pub fn vault(home: &Path) -> PathBuf {
    let own = config::text(&config::load(home), "vault");
    if own.trim().is_empty() { home.join(VAULT_DIR) } else { PathBuf::from(own.trim()) }
}

fn pass(home: &Path, slot: &mut Option<Store>) {
    let cfg = config::load(home);
    if !config::on(&cfg, "memory") {
        return;
    }
    if slot.is_none() {
        *slot = Store::open(home).ok();
    }
    let Some(store) = slot.as_mut() else { return };
    let inbox = config::text(&cfg, "vault_inbox");
    if store.index_files(&vault(home), &inbox, &claude_dirs(&user_home())).is_err() {
        *slot = None;
        return;
    }
    embed_some(home, store);
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
