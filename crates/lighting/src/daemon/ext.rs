use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use lt_core::Fail;
use serde_json::json;

use super::Daemon;
use super::hosts::Host;

const FIND_WAIT: Duration = Duration::from_secs(12);
const RECONNECT_WAIT: Duration = Duration::from_secs(15);
const STEP: Duration = Duration::from_millis(100);

fn until<T>(limit: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + limit;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() >= end {
            return None;
        }
        thread::sleep(STEP);
    }
}

pub fn reload(d: &Daemon) -> Result<String, Fail> {
    crate::setup::refresh_extension(&d.home)
        .map_err(|e| Fail::new(format!("could not copy the extension files: {e}")))?;
    let Some(old) = until(FIND_WAIT, || Some(d.hosts.alive()).filter(|h| !h.is_empty())) else {
        return Ok("extension files copied (no browser connected to reload)".into());
    };
    for h in &old {
        h.post("reload-extension", json!({}));
    }
    let fresh = |h: &Arc<Host>| !old.iter().any(|o| Arc::ptr_eq(o, h));
    Ok(match until(RECONNECT_WAIT, || d.hosts.alive().into_iter().find(fresh)) {
        Some(_) => "extension reloaded".into(),
        None => "reload requested, the extension has not reconnected yet".into(),
    })
}
