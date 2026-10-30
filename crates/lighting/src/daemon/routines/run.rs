use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use lt_core::{Call, Fail, Flag};
use regex::Regex;
use serde_json::{Value, json};

use super::super::ctx::{Ctx, now};
use super::super::web::parse_ms;
use super::super::{Daemon, Out, Pending, system};
use super::align::{Pairs, put};
use super::episode::{close, observe};
use super::store::{self, check_of, kv, place, record, step_line, text, usage, verify, where_of};

const MAX_NEST: usize = 3;
const LIMIT: Duration = Duration::from_secs(900);
const TICK: Duration = Duration::from_millis(50);
const RETRY: Duration = Duration::from_millis(300);
const STEP_MS: u64 = 6000;
const STEP_MIN: Duration = Duration::from_secs(4);
const STEP_MAX: Duration = Duration::from_secs(30);
const ERR_CHARS: usize = 120;

static TRANSIENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)nothing matches|nothing called|no field|is gone|no window matches|page changed|no tab yet|did not react|did not answer|not found|older snapshot|no match|nothing focused|still loading|navigated during",
    )
    .expect("transient")
});

struct Attempt {
    argv: Vec<String>,
    deadline: Instant,
    pending: Option<Pending>,
    again_at: Option<Instant>,
}

struct Job {
    r: Value,
    name: String,
    params: BTreeMap<String, String>,
    steps: Vec<Value>,
    next: usize,
    skipped: usize,
    yes: bool,
    t0: Instant,
    generation: u64,
    now: Option<Attempt>,
    nested: bool,
    quiet: usize,
}

enum Step {
    Done,
    Again,
    Skip,
    Stop(Fail),
}

impl Job {
    fn n(&self) -> usize {
        self.steps.len()
    }

    fn ms(&self) -> f64 {
        self.t0.elapsed().as_secs_f64() * 1000.0
    }

    fn fail(&mut self, d: &Daemon, why: String, fail: Fail) -> Step {
        let ms = self.ms();
        record(&d.home, &mut self.r, false, ms, 0, Some(why));
        Step::Stop(fail)
    }

    fn judge(&mut self, d: &Daemon, attempt: &mut Attempt, fail: &Fail) -> Step {
        let s = self.steps[self.next - 1].clone();
        let cmd = text(&s["cmd"]);
        let msg = fail.msg.split(" -> try: ").next().unwrap_or("").to_string();
        if msg.contains("matches several") && !attempt.argv.iter().any(|a| a == "--first") && cmd != "run" {
            attempt.argv.push("--first".into());
            return Step::Again;
        }
        if s["optional"].as_bool() == Some(true) {
            return Step::Skip;
        }
        if TRANSIENT.is_match(&msg) && Instant::now() < attempt.deadline && cmd != "run" {
            attempt.again_at = Some(Instant::now() + RETRY);
            return Step::Again;
        }
        let short: String = msg.chars().take(ERR_CHARS).collect();
        let (i, n) = (self.next, self.n());
        let line = step_line(&s, Some(&self.params));
        let stop = Fail::hint(
            format!("{} stopped at step {i}/{n} ({line}): {msg}", self.name),
            "the steps before are done; finish by hand (lighting snap), Lighting learns the fix",
        );
        self.fail(d, format!("step {i}: {short}"), stop)
    }

    fn start_step(&mut self, d: &Daemon) -> Result<Attempt, Step> {
        self.next += 1;
        let (i, n) = (self.next, self.n());
        let s = self.steps[i - 1].clone();
        if s["flags"]["yes"].as_bool() == Some(true) && !self.yes {
            let line = step_line(&s, Some(&self.params));
            let stop = Fail::hint(
                format!("{} step {i}/{n} ({line}) needs a confirmation", self.name),
                "rerun with --yes if the user wants this",
            );
            return Err(self.fail(d, "needs --yes".into(), stop));
        }
        let mut argv = store::argv_of(&s, &self.params);
        if text(&s["cmd"]) == "run" && self.yes {
            argv.push("--yes".into());
        }
        let timeout = s["flags"]["timeout"].as_str().map(|t| Flag::Val(t.to_string()));
        let ms = parse_ms(timeout.as_ref(), STEP_MS).unwrap_or(STEP_MS);
        let limit = Duration::from_millis(ms).clamp(STEP_MIN, STEP_MAX);
        Ok(Attempt { argv, deadline: Instant::now() + limit, pending: None, again_at: None })
    }

    fn outcome(&mut self, d: &Daemon, ctx: &mut Ctx, attempt: &mut Attempt, got: Result<String, Fail>) -> Step {
        match got {
            Ok(text) => {
                observe(d, ctx, &attempt.argv, &text, true);
                Step::Done
            }
            Err(fail) => {
                let step = self.judge(d, attempt, &fail);
                if matches!(step, Step::Stop(_)) {
                    observe(d, ctx, &attempt.argv, &fail.text(), false);
                }
                step
            }
        }
    }

    fn try_once(&mut self, d: &Daemon, ctx: &mut Ctx, attempt: &mut Attempt, last: bool) -> Option<Step> {
        if let Some(mut pending) = attempt.pending.take() {
            let now = Instant::now();
            if now < pending.due && !last {
                attempt.pending = Some(pending);
                return None;
            }
            let final_poll = last || now >= pending.deadline;
            let got = match (pending.poll)(d, ctx, final_poll) {
                Ok(None) if !final_poll => {
                    pending.due = Instant::now() + pending.interval;
                    attempt.pending = Some(pending);
                    return None;
                }
                Ok(None) => Err(Fail::new("the step did not finish in time")),
                Ok(Some(text)) => Ok(text),
                Err(fail) => Err(fail),
            };
            return Some(self.outcome(d, ctx, attempt, got));
        }
        if let Some(at) = attempt.again_at {
            if Instant::now() < at && !last {
                return None;
            }
            attempt.again_at = None;
        }
        let got = match system::run_one(d, ctx, &attempt.argv) {
            Ok((Out::Wait(pending), _)) => {
                attempt.pending = Some(pending);
                return None;
            }
            Ok((Out::Text(text) | Out::Stop(text), _)) => Ok(text),
            Err(fail) => Err(fail),
        };
        Some(self.outcome(d, ctx, attempt, got))
    }

    fn finish(&mut self, d: &Daemon, ctx: &mut Ctx) -> Result<String, Fail> {
        let ms = self.ms();
        let w = where_of(ctx);
        let status = verify(&self.r, w.as_ref(), &self.params);
        let done = self.n() - self.skipped;
        if !self.nested {
            close(d, ctx);
        } else if self.quiet == 0 && status != "not verified" {
            let mut call = vec![self.name.clone()];
            call.extend(self.params.iter().map(|(p, v)| kv(p, v)));
            let t = now();
            let at = w.clone().map_or(Value::Null, Value::Object);
            ctx.ep
                .steps
                .push(json!({"cmd": "run", "args": call, "flags": {}, "where": at, "t": (t * 100.0).round() / 100.0}));
            ctx.ep.last = t;
        }
        if status == "not verified" {
            record(&d.home, &mut self.r, false, ms, 0, Some("result not verified".into()));
            return Err(Fail::hint(
                format!(
                    "{} ran all {done} steps, but the result does not look right: {}",
                    self.name,
                    place(w.as_ref())
                ),
                "check with lighting snap; wrong parameter or the site changed",
            ));
        }
        let status = if status.is_empty() { String::new() } else { format!(", {status}") };
        let shown = if w.is_some() { format!(" -> {}", place(w.as_ref())) } else { String::new() };
        let line = format!("ok {}: {done} steps in {:.1} s{status}{shown}", self.name, ms / 1000.0);
        let has = self.r["check"]["has"].as_array().is_some_and(|h| !h.is_empty());
        if w.is_some() && !has {
            let mut values = Pairs::new();
            for (p, v) in self.params.iter().filter(|(_, v)| v.chars().count() >= 2) {
                put(&mut values, v.clone(), p.clone());
            }
            let better = check_of(w.as_ref(), &values);
            if better["has"].as_array().is_some_and(|h| !h.is_empty()) {
                self.r["check"] = better;
            }
        }
        record(&d.home, &mut self.r, true, ms, line.len() / 4 + 15, None);
        Ok(line)
    }

    fn advance(&mut self, d: &Daemon, ctx: &mut Ctx, last: bool) -> Result<Option<String>, Fail> {
        loop {
            if d.generation() != self.generation {
                let (i, n) = (self.next.max(1), self.n());
                return Err(Fail::new(format!("{} stopped by hotkey at step {i}/{n}", self.name)));
            }
            let mut attempt = match self.now.take() {
                Some(a) => a,
                None if self.next >= self.n() => return self.finish(d, ctx).map(Some),
                None => match self.start_step(d) {
                    Ok(a) => a,
                    Err(Step::Stop(f)) => return Err(f),
                    Err(_) => continue,
                },
            };
            match self.try_once(d, ctx, &mut attempt, last) {
                None => {
                    self.now = Some(attempt);
                    return Ok(None);
                }
                Some(Step::Done) => {}
                Some(Step::Skip) => self.skipped += 1,
                Some(Step::Again) => {
                    if last {
                        return Err(Fail::new(format!("{} ran out of time at step {}", self.name, self.next)));
                    }
                    self.now = Some(attempt);
                    if self.now.as_ref().is_some_and(|a| a.again_at.is_some()) {
                        return Ok(None);
                    }
                }
                Some(Step::Stop(f)) => return Err(f),
            }
        }
    }
}

fn leave(ctx: &mut Ctx, job: &Job, failed: bool) {
    if let Some(at) = ctx.run_stack.iter().rposition(|n| *n == job.name) {
        ctx.run_stack.truncate(at);
    }
    ctx.quiet_steps = job.quiet;
    if failed && !job.nested && ctx.ep.via.as_ref() == Some(&job.name) && ctx.ep.failed.is_none() {
        ctx.ep.failed = Some(ctx.ep.steps.len());
    }
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    if c.pos.is_empty() {
        return Err(Fail::hint("run needs a routine name", "lighting routines"));
    }
    let query: Vec<&str> = c.pos.iter().map(String::as_str).filter(|p| !p.contains('=')).collect();
    let wanted = if query.is_empty() { c.pos[0].clone() } else { query.join("-") };
    let r = match store::load(&d.home, &wanted) {
        Ok(r) => r,
        Err(_) if !query.is_empty() => store::find(&d.home, &query.join(" "))?,
        Err(e) => return Err(e),
    };
    let name = text(&r["name"]);
    let mut params: BTreeMap<String, String> = if c.on("defaults") {
        r["params"].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), text(v))).collect()
    } else {
        BTreeMap::new()
    };
    for p in &c.pos {
        if let Some((k, v)) = p.split_once('=') {
            params.insert(k.trim().to_string(), v.to_string());
        }
    }
    let missing: Vec<&String> =
        r["params"].as_object().into_iter().flatten().map(|(k, _)| k).filter(|k| !params.contains_key(*k)).collect();
    if !missing.is_empty() {
        let list: Vec<&str> = missing.iter().map(|k| k.as_str()).collect();
        return Err(Fail::hint(format!("{name} needs {}", list.join(", ")), usage(&r, &BTreeMap::new())));
    }
    let steps: Vec<Value> = r["steps"].as_array().cloned().unwrap_or_default();
    let secret = steps
        .iter()
        .flat_map(|s| s["args"].as_array().cloned().unwrap_or_default())
        .any(|a| text(&a).contains("@secret"));
    if secret && ctx.secret.is_none() {
        return Err(Fail::hint(format!("{name} types a secret"), format!("lighting run {name} ... --env VAR")));
    }
    if ctx.run_stack.contains(&name) {
        let chain = [ctx.run_stack.clone(), vec![name.clone()]].concat().join(" > ");
        return Err(Fail::new(format!("routine {name} calls itself ({chain})")));
    }
    if ctx.run_stack.len() >= MAX_NEST {
        return Err(Fail::hint(
            format!("routines nest deeper than {MAX_NEST} ({})", ctx.run_stack.join(" > ")),
            "flatten one of them",
        ));
    }
    let nested = !ctx.run_stack.is_empty() || (!ctx.ep.steps.is_empty() && ctx.ep.via.is_none());
    let quiet = ctx.quiet_steps;
    if !nested {
        ctx.ep.via = Some(name.clone());
        ctx.ep.params = json!(params);
    }
    ctx.quiet_steps = quiet + usize::from(nested);
    ctx.run_stack.push(name.clone());
    let mut job = Job {
        r,
        name: name.clone(),
        params,
        steps,
        next: 0,
        skipped: 0,
        yes: c.on("yes"),
        t0: Instant::now(),
        generation: d.generation(),
        now: None,
        nested,
        quiet,
    };
    let first = job.advance(d, ctx, false);
    match first {
        Ok(Some(text)) => {
            leave(ctx, &job, false);
            Ok(Out::Text(text))
        }
        Err(f) => {
            leave(ctx, &job, true);
            Err(f)
        }
        Ok(None) => Ok(Out::Wait(Pending::new(LIMIT, TICK, move |d, ctx, last| {
            let got = job.advance(d, ctx, last);
            if !matches!(got, Ok(None)) {
                leave(ctx, &job, got.is_err());
            }
            got
        }))),
    }
}
