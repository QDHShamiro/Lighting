use std::cell::RefCell;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use wisply_core::{Call, Fail};
use wisply_memory::Store;

use super::ctx::Ctx;
use super::{Daemon, Out, game, media, web};

const WAIT: Duration = Duration::from_secs(1800);
const CHARS: usize = 12000;
const CWDS: usize = 200;

fn user_home() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let home = d.home.clone();
    let sid = ctx.sid.clone();
    let cwd = if ctx.cwd.is_empty() { env::current_dir().unwrap_or_default() } else { PathBuf::from(&ctx.cwd) };
    let call = c.clone();
    let pending = media::background(
        "think",
        WAIT,
        "wisply think status (shows where the run stands)",
        move || {
            let cwds = Store::open(&home).and_then(|s| s.recent_cwds(CWDS)).unwrap_or_default();
            let env = wisply_think::Env {
                home,
                user: user_home(),
                sid,
                cwd,
                playing: game::running(),
                cwds,
                notes: RefCell::new(Vec::new()),
            };
            wisply_think::command(&env, &call).map_err(Fail::new)
        },
        |d, ctx, text: String| Ok(web::capped(d, ctx, &text, "think", None, CHARS)),
    );
    Ok(Out::Wait(pending))
}
