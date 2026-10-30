use std::fs;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde_json::{Value, json};

use super::super::Daemon;
use super::super::ctx::Ctx;
use super::serve::Server;
use super::web::{grab, has, ok, tab_ref};
use super::{Run, argv, once};

const BRAKE_AT: usize = 5;
const LANDED_TRIES: usize = 10;
const LANDED_GAP: Duration = Duration::from_millis(150);

fn lit_line(out: &str, tab: &str) -> Option<bool> {
    let lit = Regex::new(r"^L\d*[*a]* ").ok()?;
    out.lines().find_map(|l| l.strip_prefix(tab).and_then(|rest| rest.strip_prefix(' ')).map(|rest| lit.is_match(rest)))
}

fn listed(out: &str, tab: &str, lit: bool) -> bool {
    lit_line(out, tab) == Some(lit)
}

fn run(d: &Daemon, ctx: &mut Ctx, words: &[&str]) -> String {
    once(d, ctx, &argv(words)).0
}

fn first_lines(seen: &[String]) -> String {
    seen.iter().map(|s| s.lines().next().unwrap_or("")).collect::<Vec<_>>().join(" / ")
}

pub fn sessions_check(d: &Daemon, base: &str) -> (bool, String) {
    let mut a = Ctx::scratch("selftest-a", "Lighting selftest a");
    let mut b = Ctx::scratch("selftest-b", "Lighting selftest b");
    let (second, quiet) = (format!("{base}second"), format!("{base}quiet"));
    run(d, &mut a, &["open", &second]);
    run(d, &mut b, &["open", &quiet]);
    let mut seen =
        vec![run(d, &mut a, &["snap", "--force"]), run(d, &mut b, &["done"]), run(d, &mut a, &["snap", "--force"])];
    run(d, &mut b, &["open", &quiet]);
    seen.push(run(d, &mut b, &["keep"]));
    seen.push(run(d, &mut b, &["snap", "--force"]));
    run(d, &mut b, &["close"]);
    run(d, &mut a, &["done"]);
    run(d, &mut b, &["done"]);
    let good = seen[0].contains("Second")
        && seen[1].starts_with("closed 1 tab")
        && seen[2].contains("Second")
        && seen[3].starts_with("kept 1 tab")
        && seen[4].contains("Quiet");
    (good, if good { first_lines(&seen) } else { seen.join(" / ").replace('\n', " ; ") })
}

pub fn reopen_check(d: &Daemon, base: &str) -> (bool, String) {
    let mut c = Ctx::scratch("selftest-reopen", "Lighting selftest r");
    run(d, &mut c, &["open", &format!("{base}second")]);
    let seen = vec![run(d, &mut c, &["done"]), run(d, &mut c, &["snap", "--force"])];
    run(d, &mut c, &["done"]);
    let good = seen[0].starts_with("closed 1 tab") && seen[1].contains("reopened") && seen[1].contains("Second");
    (good, first_lines(&seen))
}

pub fn brake_check(d: &Daemon) -> (bool, String) {
    let mut c = Ctx::scratch("selftest-brake", "Lighting selftest k");
    let words = argv(&["tab", "t9998"]);
    let mut last = Ok(String::new());
    for _ in 0..BRAKE_AT {
        let (text, _) = once(d, &mut c, &words);
        last = c.fails.repeated(&words, text, true);
    }
    match last {
        Err((wait, text)) => {
            let good = wait >= Duration::from_secs(1) && text.contains("STOP") && text.contains("no tab t9998");
            (
                good,
                format!(
                    "waits {} s after {BRAKE_AT} identical failures, then: {}",
                    wait.as_secs(),
                    text.lines().last().unwrap_or("")
                ),
            )
        }
        Ok(text) => (false, text),
    }
}

fn landed(d: &Daemon, ctx: &mut Ctx, words: &[&str], want: &str) -> (bool, String) {
    let mut out = String::new();
    for _ in 0..LANDED_TRIES {
        out = run(d, ctx, words);
        if out.contains(want) {
            return (true, out);
        }
        thread::sleep(LANDED_GAP);
    }
    (false, out)
}

fn focus_probe(r: &Run) -> String {
    let Ok(host) = super::super::web::ensure_host(r.d) else { return "no host".into() };
    let args =
        if r.ctx.sid.is_empty() { json!({"close": false}) } else { json!({"close": false, "only": r.ctx.owned}) };
    match host.call("cleanup", args, None, Duration::from_secs(5), Some(r.ctx.group.as_str())) {
        Ok(v) => v.get("focus").map_or_else(|| v.to_string(), Value::to_string),
        Err(_) => "no answer".into(),
    }
}

pub fn browser(r: &mut Run, srv: &Server, decoy: &str) {
    let base = srv.base.as_str();
    let at = |p: &str| format!("{base}{p}");
    let script = r.d.home.join("out").join("selftest.js");
    let _ = fs::write(&script, "return document.title");
    let script = script.display().to_string();
    r.step("js --file", &["js", "--file", &script], |o, _| o.trim() == "Lighting Fixture");
    r.step("unknown tab alias", &["tab", "t9999"], |o, e| e && o.contains("no tab t9999"));
    r.step("viewport 390x844", &["viewport", "390x844"], |o, e| !e && o.contains("mobile"));
    r.step("viewport applied", &["js", "innerWidth"], |o, _| o.trim() == "390");
    r.step("viewport reset", &["viewport", "reset"], ok);
    r.step("viewport restored", &["js", "innerWidth"], |o, _| {
        let o = o.trim();
        !o.is_empty() && o.chars().all(|c| c.is_ascii_digit()) && o != "390"
    });
    r.step("shot --marks", &["shot", "--marks"], |o, e| {
        !e && o.contains(".jpg") && has(r"\b[1-9]\d* orange labels", o)
    });
    r.step("marks removed", &["js", "document.querySelectorAll('lt-pointer[data-marks]').length"], |o, _| {
        o.trim() == "0"
    });
    r.secret_step(
        "fill + submit form",
        &["fill", "Email=test@example.com", "Password=@secret", "Country=Germany", "Remember me=on", "--submit"],
        Some("hunter2"),
        ok,
    );
    r.step("form result", &["expect", "Submitted: test@example.com / Germany / yes / pw 7"], ok);
    r.step("secret not echoed", &["log", "3"], |o, _| !o.contains("hunter2"));
    r.step("risky click needs --yes", &["click", "Delete all"], |o, e| e && o.contains("--yes"));
    r.step("click opens confirm()", &["click", "Delete all", "--yes"], |o, _| o.contains("dialog"));
    r.step("accept dialog", &["dialog", "accept"], |o, e| !e && o.contains("accepted"));
    r.step("dialog result", &["expect", "Deleted"], ok);
    r.step("open modal (diff)", &["click", "Open modal"], |o, e| !e && o.contains("Close modal"));
    r.step("snap scopes to modal", &["snap"], |o, _| {
        o.contains("modal") && o.contains("Close modal") && !o.contains("Sign in")
    });
    r.step("close modal", &["click", "Close modal"], ok);
    r.step("click role-less card", &["click", "Clickable card without role"], ok);
    r.step("card clicked", &["expect", "Card clicked"], ok);
    r.step("wait for late button", &["wait", "Appears later", "--timeout", "5000"], ok);
    let aout =
        r.step("find upload menu item", &["snap", "-f", "Upload avatar"], |o, e| !e && has(r"(?m)^e\d+ menuitem", o));
    let aref = grab(r"(?m)^(e\d+) menuitem", &aout, "e0");
    r.step("upload through a label", &["upload", &aref, &script], |o, e| !e && o.contains("1 file set"));
    r.step("upload landed", &["expect", "Avatar: selftest.js"], ok);
    r.step("table as tsv", &["table", "#prices"], |o, e| !e && o.contains("Beta\t2.50"));
    r.step("type contenteditable", &["type", "Notes", "hello from lighting"], ok);
    r.step("notes text present", &["expect", "hello from lighting"], ok);
    r.step("text (markdown)", &["text"], |o, e| {
        !e && o.contains("| Beta | 2.50 |")
            && !o.contains("IGNORE ALL")
            && !o.contains("<iframe")
            && !o.contains("readable view")
    });
    r.step("js", &["js", "document.title"], |o, _| o.trim() == "Lighting Fixture");
    r.step("js with const", &["js", "const v = 2; v * 3"], |o, _| o.trim() == "6");
    r.step("js with const again", &["js", "const v = 2; v * 3"], |o, _| o.trim() == "6");
    r.step("js with await and const", &["js", "const r = await Promise.resolve(21); r * 2"], |o, _| o.trim() == "42");
    r.step("js await expression", &["js", "await Promise.resolve('ok')"], |o, _| o.trim() == "ok");
    r.step("fetch json-less", &["fetch", &at("frame")], |o, e| !e && o.contains("200"));
    r.step("snap iframe", &["snap", "--frame", "127.0.0.1"], |o, e| !e && o.contains("Frame button"));
    let fout = r.step("snap other-site frame", &["snap", "--frame", "localhost", "-f", "Frame button"], |o, e| {
        !e && has(r"(?m)^f\d+\.e\d+ button", o)
    });
    let fref = grab(r"(?m)^(f\d+\.e\d+) button", &fout, "f0.e0");
    r.step("click in other-site frame", &["click", &fref], ok);
    r.check("frame click landed", |d, ctx| {
        landed(d, ctx, &["snap", "--frame", "localhost", "--full"], "Frame clicked")
    });
    r.step("wait sees control names", &["wait", "checks all green", "--timeout", "2000"], ok);
    r.step("filter falls back to text", &["snap", "-f", "Fact"], |o, e| {
        !e && o.contains("this text does") && o.contains("\"Fact")
    });
    r.step("shot element", &["shot", "e1"], |o, e| !e && o.contains(".jpg"));
    r.step("new tab event", &["click", "Open in new tab"], |o, e| !e && o.contains("new tab t"));
    r.step("close popup tab", &["close"], ok);
    r.step("read url without browser", &["read", base], |o, e| {
        !e && o.contains("Lighting Fixture") && !o.contains("IGNORE")
    });
    r.step("read 2 urls in parallel", &["read", base, &at("second")], |o, e| {
        !e && o.contains("Lighting Fixture") && o.contains("Opened in a new tab")
    });
    r.step("read pdf as text", &["read", &at("doc.pdf")], |o, e| {
        !e && o.contains("Hello Lighting PDF") && o.contains("pdf, 1 pages")
    });
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |t| t.as_millis());
    let leak = format!("http://leak-{stamp}.invalid/?d={}", "x".repeat(300));
    r.step("leak guard blocks", &["read", &leak], |o, e| e && o.contains("possible data leak"));
    r.step("leak guard --yes passes", &["read", &leak, "--yes"], |o, e| e && o.contains("read failed"));
    r.step("unchanged snap", &["snap"], ok);
    r.step("snap --full", &["snap", "--full"], ok);
    r.step("toggle for a small change", &["click", "Dark mode"], ok);
    r.step("snap shows only changes", &["snap"], |o, e| {
        !e && o.contains("changed since your last snap") && o.contains("Dark mode")
    });
    r.step("blocklist enforced", &[super::BLOCKED], |o, e| e && o.contains("blocked"));
    r.step("record start (web)", &["record", "start", "selftest-rec", "--web"], |o, e| !e && o.contains("recording"));
    r.step("recorded fill", &["fill", "Email=rec@example.com"], ok);
    r.step("recorded click", &["click", "Dark mode"], ok);
    r.step("record stop (web)", &["record", "stop", "email=rec@example.com"], |o, e| {
        !e && o.contains("fill Email={email}") && o.contains("click \"Dark mode\"")
    });
    r.step("run recorded routine", &["run", "selftest-rec", "email=run@example.com"], |o, e| {
        !e && o.starts_with("ok selftest-rec")
    });
    r.step("routine filled field", &["js", "document.getElementById('email').value"], |o, _| {
        o.trim() == "run@example.com"
    });
    r.step("remove web routine", &["routine", "rm", "selftest-rec"], ok);
    r.step("close fixture tab", &["close"], ok);
    r.step("target back to decoy", &["snap"], |o, e| !e && o.contains("Second"));
    r.step("keep hands tab over", &["keep", decoy], |o, e| !e && o.contains("kept 1 tab"));
    r.step("kept tab left the group", &["tabs"], |o, _| listed(o, decoy, false));
    let thrown = r.step("throwaway tab", &["open", &at("second"), "--new"], |o, e| !e && o.contains("Second"));
    let tref = tab_ref(&thrown);
    r.step("open a slow page", &["open", &at("slow")], |o, e| {
        !e && o.contains("Slow page") && o.contains("(still loading)")
    });
    r.step("snap while loading", &["snap"], |o, e| !e && o.contains("(still loading)"));
    r.step("text while loading", &["text"], |o, e| !e && o.contains("Slow page") && o.contains("(still loading)"));
    let tabs = run(r.d, r.ctx, &["tabs"]);
    let lit = Regex::new(r"(?m)^(t\d+) L\d*[*a]* ")
        .map(|re| re.captures_iter(&tabs).filter_map(|c| c.get(1)).any(|m| m.as_str() != tref));
    let launched = r.ctx.desk.get("launched").and_then(|v| v.as_object()).is_some_and(|m| !m.is_empty());
    if lit.unwrap_or(true) || launched {
        r.skip("done closes tabs (other Lighting tabs or launched apps are open)");
        r.step("close throwaway tab", &["close", &tref], ok);
    } else {
        let probe = focus_probe(r);
        let out = r.step("done closes tabs", &["done"], |o, e| !e && o.starts_with("closed 1 tab"));
        if !out.starts_with("closed 1 tab") {
            r.rows.push(format!("DIAG {:<26} | {probe}", "focus before done"));
        }
        r.step("done kept the kept tab", &["tabs"], |o, _| !listed(o, &tref, true) && listed(o, decoy, false));
    }
    r.step("close kept tab", &["close", decoy, "--force"], ok);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_lines_tell_lighting_tabs_from_handed_over_ones() {
        let out = "t12 L2* Fixture - http://x\nt13 Second - http://y\nt14 La* Other";
        assert!(listed(out, "t12", true) && listed(out, "t13", false) && listed(out, "t14", true));
        assert!(!listed(out, "t1", false) && !listed(out, "t12", false));
    }
}
