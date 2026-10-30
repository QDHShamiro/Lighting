use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use lt_core::{QUIET, Row};

use super::ctx::{Contexts, Ctx, now};
use super::{Daemon, Out, Pending, Req, Res, STOPPED, Work, panic_text, route, system, web};

const CLOSE_RETRY_S: f64 = 1.0;

struct Wait {
    pending: Pending,
    req: Req,
    name: String,
}

pub fn work(d: &Daemon, jobs: &Receiver<Work>, mut ctxs: Contexts, main: bool) {
    let mut waits: Vec<Wait> = Vec::new();
    loop {
        let close_at = ctxs.next_closing().map(|t| Instant::now() + web::secs(t - now()));
        let next = waits.iter().map(|w| w.pending.due).chain(close_at).min();
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
                ctxs.note_resets(d.hosts.resets());
                waits.extend(run(d, &mut ctxs, *req));
            }
        }
        let (t, generation) = (Instant::now(), d.generation());
        let (due, later): (Vec<Wait>, Vec<Wait>) =
            waits.drain(..).partition(|w| w.pending.due <= t || w.req.generation != generation);
        waits = later;
        waits.extend(due.into_iter().filter_map(|w| poll(d, &mut ctxs, w)));
        close_due(d, &mut ctxs, &waits);
        if main {
            ctxs.save(&d.home, &d.hosts.epochs());
        }
    }
}

fn run(d: &Daemon, ctxs: &mut Contexts, mut req: Req) -> Option<Wait> {
    req.started = Instant::now();
    let raw = req.argv.first().cloned().unwrap_or_default();
    let quiet = QUIET.contains(&raw.to_lowercase().as_str());
    let ctx = ctxs.get(&req.sid);
    ctx.begin(req.secret.take(), mem::take(&mut req.cwd));
    if !quiet {
        ctx.closing = 0.0;
    }
    let done = panic::catch_unwind(AssertUnwindSafe(|| system::run_one(d, ctx, &req.argv)));
    ctx.secret = None;
    req.cut += mem::take(&mut ctx.cut);
    let (out, code, name, stop) = match done {
        Ok(Ok((Out::Wait(pending), name))) => {
            ctx.fails.forget(&req.argv);
            return Some(Wait { pending, req, name });
        }
        Ok(Ok((Out::Text(text), name))) => {
            ctx.fails.forget(&req.argv);
            (text, 0, name, false)
        }
        Ok(Ok((Out::Stop(text), name))) => (text, 0, name, true),
        Ok(Err(fail)) if !fail.counted => (fail.text(), 1, raw, false),
        Ok(Err(fail)) => match ctx.fails.repeated(&req.argv, fail.text(), !quiet) {
            Ok(text) => (text, 1, raw, false),
            Err((wait, text)) => return Some(Wait { pending: Pending::said_after(wait, text), req, name: raw }),
        },
        Err(panic) => (format!("err: panic: {}", panic_text(&*panic)), 1, raw, false),
    };
    if name == "suggest" {
        let _ = req.reply.send(Res::new(0, if code == 0 { out } else { String::new() }));
        return None;
    }
    finish(d, ctx, req, &name, out, code, stop);
    None
}

fn poll(d: &Daemon, ctxs: &mut Contexts, mut wait: Wait) -> Option<Wait> {
    let ctx = ctxs.get(&wait.req.sid);
    if wait.req.generation != d.generation() {
        finish(d, ctx, wait.req, &wait.name, STOPPED.into(), 1, false);
        return None;
    }
    let last = Instant::now() >= wait.pending.deadline;
    let saved = (ctx.target.clone(), ctx.place.clone(), ctx.last_target.clone());
    ctx.image.clear();
    let polled = panic::catch_unwind(AssertUnwindSafe(|| (wait.pending.poll)(d, ctx, last)));
    (ctx.target, ctx.place, ctx.last_target) = saved;
    wait.req.cut += mem::take(&mut ctx.cut);
    let (out, code) = match polled {
        Ok(Ok(None)) if !last => {
            wait.pending.due = Instant::now() + wait.pending.interval;
            return Some(wait);
        }
        Ok(Ok(text)) => (text.unwrap_or_default(), 0),
        Ok(Err(fail)) => (fail.text(), 1),
        Err(panic) => (format!("err: panic: {}", panic_text(&*panic)), 1),
    };
    finish(d, ctx, wait.req, &wait.name, out, code, false);
    None
}

fn close_due(d: &Daemon, ctxs: &mut Contexts, waits: &[Wait]) {
    for sid in ctxs.closing_due(now()) {
        let busy = waits.iter().any(|w| w.req.sid == sid);
        let Some(ctx) = ctxs.find(&sid) else { continue };
        if busy {
            ctx.closing = now() + CLOSE_RETRY_S;
            continue;
        }
        ctx.closing = 0.0;
        if ctx.sid.is_empty() || !ctx.owned.is_empty() || ctx.launched_apps() {
            let _ = panic::catch_unwind(AssertUnwindSafe(|| web::cleanup_now(d, ctx, true)));
        }
    }
}

fn finish(d: &Daemon, ctx: &mut Ctx, req: Req, name: &str, mut out: String, code: i32, stop: bool) {
    super::routines::observe(d, ctx, &req.argv, &out, code == 0);
    super::memory::learn::after(d, ctx, name, &req.argv, code, &mut out);
    if code == 0 && route::foreign(name) {
        out = route::ai_note(out);
    }
    let mut extra = mem::take(&mut ctx.notes);
    if !ctx.side {
        for event in d.hosts.take_events() {
            let quoted = event.split('"').nth(1).filter(|q| !q.is_empty() && event.matches('"').count() >= 2);
            if quoted.is_none_or(|q| !out.contains(&format!("\"{q}\""))) {
                extra.push(format!("! {event}"));
            }
        }
    }
    if !extra.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&extra.join("\n"));
    }
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
    d.log.write(&row);
    let image = if code == 0 { mem::take(&mut ctx.image) } else { String::new() };
    ctx.image.clear();
    let _ = req.reply.send(Res { code, out, image, stop });
}
