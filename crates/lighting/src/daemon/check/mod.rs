#[cfg(windows)]
mod app;
mod bench;
mod demo;
mod serve;
mod web;
mod web2;

use std::time::Instant;

use lt_core::{Call, Fail};
use serde_json::{Value, json};

use super::ctx::Ctx;
use super::routines::{align, build, step_line};
use super::system::run_one;
use super::words::{best_only, terms, tiers};
use super::{Daemon, Out, config};

pub use bench::bench;
pub use demo::demo;

const TAIL: usize = 180;
const BLOCKED: &str = "_blocked";

pub struct Run<'a> {
    pub d: &'a Daemon,
    pub ctx: &'a mut Ctx,
    pub rows: Vec<String>,
    passed: usize,
    total: usize,
}

pub fn once(d: &Daemon, ctx: &mut Ctx, argv: &[String]) -> (String, bool) {
    if argv.first().is_some_and(|a| a == BLOCKED) {
        return blocked(d, ctx);
    }
    match run_one(d, ctx, argv) {
        Ok((Out::Text(t) | Out::Stop(t), _)) => (t, false),
        Ok((Out::Wait(p), _)) => match p.settle(d, ctx) {
            Ok(t) => (t, false),
            Err(f) => (f.text(), true),
        },
        Err(f) => (f.text(), true),
    }
}

pub fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_string()).collect()
}

fn one_line(text: &str, max: usize) -> String {
    text.replace('\n', " / ").chars().take(max).collect()
}

impl<'a> Run<'a> {
    pub fn new(d: &'a Daemon, ctx: &'a mut Ctx) -> Run<'a> {
        Run { d, ctx, rows: Vec::new(), passed: 0, total: 0 }
    }

    fn record(&mut self, name: &str, good: bool, ms: u128, out: &str) {
        self.total += 1;
        self.passed += usize::from(good);
        let tail = if good { String::new() } else { format!(" | {}", one_line(out, TAIL)) };
        let mark = if good { "PASS" } else { "FAIL" };
        self.rows.push(format!("{mark} {name:<26} {ms:>5} ms {:>5} ch{tail}", out.chars().count()));
    }

    pub fn step(&mut self, name: &str, words: &[&str], check: impl Fn(&str, bool) -> bool) -> String {
        self.secret_step(name, words, None, check)
    }

    pub fn secret_step(
        &mut self,
        name: &str,
        words: &[&str],
        secret: Option<&str>,
        check: impl Fn(&str, bool) -> bool,
    ) -> String {
        let t = Instant::now();
        self.ctx.secret = secret.map(String::from);
        let (out, err) = once(self.d, self.ctx, &argv(words));
        self.ctx.secret = None;
        let good = check(&out, err);
        self.record(name, good, t.elapsed().as_millis(), &out);
        out
    }

    pub fn check(&mut self, name: &str, f: impl FnOnce(&Daemon, &mut Ctx) -> (bool, String)) {
        let t = Instant::now();
        let (good, out) = f(self.d, self.ctx);
        self.record(name, good, t.elapsed().as_millis(), &out);
    }

    pub fn diag(&mut self, words: &[&str]) {
        let (out, _) = once(self.d, self.ctx, &argv(words));
        self.rows.push(format!("DIAG {:<26} | {}", words[0], one_line(&out, 300)));
    }

    pub fn skip(&mut self, what: &str) {
        self.rows.push(format!("SKIP {what}"));
    }

    fn finish(mut self) -> String {
        self.rows.push(format!("{}/{} passed", self.passed, self.total));
        self.rows.join("\n")
    }
}

fn blocked(d: &Daemon, ctx: &mut Ctx) -> (String, bool) {
    let host = match super::web::ensure_host(d) {
        Ok(h) => h,
        Err(f) => return (f.text(), true),
    };
    let mut payload = config::payload(&d.home);
    if let Some(list) = payload["blocklist"].as_array_mut() {
        list.push(json!("127.0.0.1"));
    }
    host.post("config", payload);
    let got = once(d, ctx, &argv(&["click", "Sign in"]));
    d.hosts.push_config(&d.home);
    got
}

fn tiers_check() -> (bool, String) {
    let words = terms("sos|mit");
    let names = ["SOS", "Sosa La M", "SOS wiedergeben", "commits", "MIT license"];
    let rows: Vec<(Vec<u8>, &str)> =
        names.iter().map(|n| (tiers(&words, &[n]), *n)).filter(|(t, _)| t.iter().any(|x| *x > 0)).collect();
    let kept = best_only(&rows);
    (kept == ["SOS", "SOS wiedergeben", "MIT license"], kept.join(", "))
}

fn learn_check() -> (bool, String) {
    let st = |c: &str, a: &str| json!({"cmd": c, "args": [a], "flags": {}, "where": null});
    let a = [st("launch", "spotify:search:SOS"), st("click", "SOS wiedergeben")];
    let b = [st("launch", "spotify:search:Numb"), st("click", "Numb wiedergeben")];
    let Some(blk) = align(&a, &b) else { return (false, "no alignment".into()) };
    let body = build(&a, &b, &Value::Null, &blk);
    let lines: Vec<String> = body["steps"].as_array().into_iter().flatten().map(|s| step_line(s, None)).collect();
    let good = body["params"] == json!({"search": "Numb"})
        && lines == ["launch spotify:search:{search}", "click \"{search} wiedergeben\""];
    (good, lines.join(" | "))
}

pub fn selftest(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let only = c.pos.first().map_or("all", String::as_str);
    if !["all", "web", "app", "chat"].contains(&only) {
        return Err(Fail::hint(format!("no selftest part '{only}'"), "lighting selftest [web|app|chat]"));
    }
    let mut r = Run::new(d, ctx);
    r.check("filter tiers", |_, _| tiers_check());
    r.check("routine learning", |_, _| learn_check());
    if matches!(only, "all" | "web") {
        if super::web::any_browser(d) {
            let srv = serve::Server::start(serve::Site::Fixture).map_err(|e| Fail::new(format!("test server: {e}")))?;
            let decoy = web::browser(&mut r, &srv);
            web2::browser(&mut r, &srv, &decoy);
        } else {
            r.skip("browser tests (no browser connected: lighting setup)");
        }
    }
    #[cfg(windows)]
    {
        if matches!(only, "all" | "app") {
            app::desktop(&mut r);
        }
        if matches!(only, "all" | "app" | "chat") {
            app::chat(&mut r);
        }
    }
    #[cfg(not(windows))]
    if only != "web" {
        r.skip("app and chat tests (Windows only; macOS and Linux apps are tested in CI through AX and AT-SPI)");
    }
    Ok(r.finish())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_quick_checks_pass_on_their_own() {
        assert!(super::tiers_check().0, "{}", super::tiers_check().1);
        assert!(super::learn_check().0, "{}", super::learn_check().1);
    }
}
