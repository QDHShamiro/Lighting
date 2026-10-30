use std::time::{Duration, Instant};

use lt_core::{Call, Fail, parse};

use super::ctx::Ctx;
use super::{Daemon, Out, Pending, STOPPED, system};

const LIMIT: Duration = Duration::from_secs(900);
const TICK: Duration = Duration::from_millis(50);
const READS: &[&str] = &[
    "snap",
    "text",
    "table",
    "read",
    "js",
    "fetch",
    "tabs",
    "windows",
    "shot",
    "log",
    "status",
    "downloads",
    "console",
    "expect",
    "wait",
    "clip",
    "help",
    "version",
    "config",
    "routines",
    "frames",
    "inbox",
    "keys",
    "listen",
    "watch",
    "unread",
    "net",
    "tools",
    "recall",
    "todo",
    "memory",
    "code",
    "channels",
];
const NESTED: &[&str] = &["do", "batch", "stop"];

pub fn split_steps(text: &str) -> Vec<String> {
    let (mut steps, mut cur, mut quote) = (Vec::new(), String::new(), None);
    for ch in text.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
                cur.push(ch);
            }
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                cur.push(ch);
            }
            None if ch == ';' || ch == '\n' => steps.push(std::mem::take(&mut cur)),
            None => cur.push(ch),
        }
    }
    steps.push(cur);
    steps.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

struct Step {
    pending: Pending,
    name: String,
    optional: bool,
}

struct Job {
    steps: Vec<String>,
    next: usize,
    lines: Vec<String>,
    ok: usize,
    failed: usize,
    keep: bool,
    quiet: bool,
    generation: u64,
    current: Option<Step>,
}

impl Job {
    fn prefix(&self, i: usize) -> String {
        if self.steps.len() > 1 { format!("{i} ") } else { String::new() }
    }

    fn done_one(&mut self, i: usize, name: &str, text: &str) {
        self.ok += 1;
        let line = if i < self.steps.len() && !READS.contains(&name) {
            if self.quiet {
                return;
            }
            text.split('\n').next().unwrap_or("").to_string()
        } else {
            text.to_string()
        };
        self.lines.push(format!("{}{line}", self.prefix(i)));
    }

    fn failed_one(&mut self, i: usize, optional: bool, fail: &Fail) -> bool {
        if optional {
            self.lines.push(format!("{}skipped, {}", self.prefix(i), fail.text()));
            return true;
        }
        self.failed += 1;
        let left = self.steps.len() - i;
        if self.keep || left == 0 {
            self.lines.push(format!("{}{}", self.prefix(i), fail.text()));
            return true;
        }
        self.lines.push(format!("{i} {} (stopped, {left} skipped)", fail.text()));
        false
    }

    fn summary(&mut self) -> String {
        let n = self.steps.len();
        if self.keep && self.failed > 0 {
            self.lines.push(format!("{} of {n} steps failed", self.failed));
        } else if self.quiet && self.ok == n && n > 1 {
            self.lines.insert(0, format!("ok {}/{n}", self.ok));
        }
        self.lines.join("\n")
    }

    fn advance(&mut self, d: &Daemon, ctx: &mut Ctx, last: bool) -> Result<Option<String>, Fail> {
        loop {
            if d.generation() != self.generation {
                self.lines.push(format!("{} {STOPPED}", self.next));
                return Ok(Some(self.summary()));
            }
            if let Some(mut step) = self.current.take() {
                let now = Instant::now();
                if now < step.pending.due && !last {
                    self.current = Some(step);
                    return Ok(None);
                }
                let final_poll = last || now >= step.pending.deadline;
                ctx.image.clear();
                match (step.pending.poll)(d, ctx, final_poll) {
                    Ok(None) if !final_poll => {
                        step.pending.due = Instant::now() + step.pending.interval;
                        self.current = Some(step);
                        return Ok(None);
                    }
                    Ok(None) => {
                        let fail = Fail::new(format!("{} did not finish in time", step.name));
                        if !self.failed_one(self.next, step.optional, &fail) {
                            return Ok(Some(self.summary()));
                        }
                    }
                    Ok(Some(text)) => self.done_one(self.next, &step.name, &text),
                    Err(fail) => {
                        if !self.failed_one(self.next, step.optional, &fail) {
                            return Ok(Some(self.summary()));
                        }
                    }
                }
            }
            if self.next >= self.steps.len() {
                return Ok(Some(self.summary()));
            }
            self.next += 1;
            let i = self.next;
            let raw = self.steps[i - 1].clone();
            let optional = raw.starts_with('?');
            let body = if optional { raw[1..].trim() } else { raw.as_str() };
            let argv = match lt_proto::words(body) {
                Ok(argv) => argv,
                Err(why) => {
                    if !self.failed_one(i, optional, &Fail::new(why)) {
                        return Ok(Some(self.summary()));
                    }
                    continue;
                }
            };
            let name = parse(&argv).name;
            if NESTED.contains(&name.as_str()) {
                let fail = Fail::hint(format!("{name} cannot run inside do"), "run it as its own command");
                if !self.failed_one(i, optional, &fail) {
                    return Ok(Some(self.summary()));
                }
                continue;
            }
            match system::run_one(d, ctx, &argv) {
                Ok((Out::Wait(pending), name)) => self.current = Some(Step { pending, name, optional }),
                Ok((Out::Text(text) | Out::Stop(text), name)) => self.done_one(i, &name, &text),
                Err(fail) => {
                    if !self.failed_one(i, optional, &fail) {
                        return Ok(Some(self.summary()));
                    }
                }
            }
        }
    }
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let steps = split_steps(&c.pos.join(" "));
    if steps.is_empty() {
        return Err(Fail::hint("do needs steps", "lighting do \"click e3; type e4 hi; press Enter\""));
    }
    let mut job = Job {
        steps,
        next: 0,
        lines: Vec::new(),
        ok: 0,
        failed: 0,
        keep: c.on("keep-going"),
        quiet: c.on("quiet"),
        generation: d.generation(),
        current: None,
    };
    match job.advance(d, ctx, false)? {
        Some(text) => Ok(Out::Text(text)),
        None => Ok(Out::Wait(Pending::new(LIMIT, TICK, move |d, ctx, last| job.advance(d, ctx, last)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_split_on_semicolons_and_lines_outside_quotes() {
        assert_eq!(
            split_steps("click e3; type e4 \"a;b\"\npress Enter;;"),
            vec!["click e3", "type e4 \"a;b\"", "press Enter"]
        );
        assert_eq!(split_steps("  ; \n"), Vec::<String>::new());
        assert_eq!(split_steps("type e1 'it''s; ok'"), vec!["type e1 'it''s; ok'"]);
    }
}
