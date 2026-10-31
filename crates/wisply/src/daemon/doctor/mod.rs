mod install;

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::brain;
use super::ctx::now;
use super::media::tools;
use super::memory::chat::api_key;
use install::{Check, TOOLS};

pub type Ask = fn(&Path, &[&str]) -> Result<(i32, String), String>;

const VERSION_WAIT: Duration = Duration::from_secs(10);
const BIG_LOG: u64 = 50_000_000;
const BIG_OUT: u64 = 500_000_000;
const LOG_TAIL: u64 = 262_144;
const STARTED: &str = "wisply daemon started";
const PANIC: &str = " panic in ";
const FAIL_PCT: f64 = 10.0;
const MIN_FAILS: usize = 10;
const SHOWN_CHARS: usize = 100;
const ERROR_CHARS: usize = 60;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Warn,
    Fail,
}

struct Row {
    level: Level,
    name: &'static str,
    text: String,
    fix: String,
}

fn row(level: Level, name: &'static str, text: impl Into<String>, fix: impl Into<String>) -> Row {
    Row { level, name, text: text.into(), fix: fix.into() }
}

fn first_line(exe: &Path, arg: &str) -> Option<String> {
    let ran = tools::run(exe, &[arg.to_string()], VERSION_WAIT).ok()?;
    let text = if ran.out.trim().is_empty() { ran.err } else { ran.out };
    text.lines().next().map(|l| l.trim().chars().take(60).collect())
}

fn daemon_rows(home: &Path, ask: Ask) -> Vec<Row> {
    match ask(home, &["status"]) {
        Ok((0, out)) => {
            let t0 = Instant::now();
            let ms = ask(home, &["version"]).map_or(0, |_| t0.elapsed().as_millis());
            let head = out.lines().next().unwrap_or("");
            let pid = head.split("daemon pid ").nth(1).and_then(|r| r.split(' ').next()).unwrap_or("?");
            let mut rows = vec![row(Level::Ok, "daemon", format!("pid {pid}, answers in {ms} ms"), "")];
            let browser = out.lines().find(|l| l.starts_with("browser ")).unwrap_or("browser unknown");
            let text = browser.trim_start_matches("browser ").split(" -> ").next().unwrap_or("").to_string();
            rows.push(if text.starts_with("none") {
                row(Level::Warn, "browser", text, "wisply setup, then open Brave")
            } else {
                row(Level::Ok, "browser", text, "")
            });
            rows
        }
        Ok((_, out)) => vec![row(Level::Fail, "daemon", out, format!("see {}", home.join("daemon.log").display()))],
        Err(why) => vec![row(Level::Fail, "daemon", why, "wisply stop, then run any command to start it again")],
    }
}

fn tool_rows(home: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    for tool in &TOOLS {
        let found = match tool.name {
            "ffmpeg" => tools::ffmpeg(home),
            _ => tools::ytdlp(home).ok(),
        };
        let own = found.as_ref().is_some_and(|p| p.starts_with(home.join("bin")));
        let what = if tool.name == "ffmpeg" { "screen video, sound" } else { "video and sound from links" };
        match (found, own) {
            (None, _) => rows.push(row(
                Level::Warn,
                tool.name,
                format!("not installed ({what} fall back to slower ways)"),
                "wisply doctor --fix",
            )),
            (Some(_), true) => match install::verify(home, tool) {
                Check::Verified(v) => {
                    rows.push(row(Level::Ok, tool.name, format!("{v} in ~/.wisply/bin, SHA-256 ok"), ""))
                }
                Check::Changed(why) => {
                    rows.push(row(Level::Fail, tool.name, why, "wisply doctor --fix (downloads it again)"))
                }
                Check::Missing => rows.push(row(
                    Level::Warn,
                    tool.name,
                    "in ~/.wisply/bin but not installed by doctor (unknown version)",
                    "wisply doctor --fix",
                )),
            },
            (Some(path), false) => {
                let version = first_line(&path, if tool.name == "ffmpeg" { "-version" } else { "--version" })
                    .unwrap_or_else(|| "version unknown".into());
                rows.push(row(Level::Ok, tool.name, format!("{version} ({})", path.display()), ""));
            }
        }
    }
    match tools::which("gh") {
        Some(gh) => {
            let ok = tools::run(&gh, &["auth".into(), "status".into()], VERSION_WAIT).is_ok_and(|r| r.ok);
            rows.push(if ok {
                row(Level::Ok, "gh", "logged in (GitHub issues, PRs, releases as short lists)", "")
            } else {
                row(Level::Warn, "gh", "not logged in, GitHub falls back to the API (60 calls/h)", "gh auth login")
            });
        }
        None => rows.push(row(Level::Ok, "gh", "not installed, GitHub uses the API (60 calls/h)", "")),
    }
    if tools::which("git").is_none() {
        rows.push(row(Level::Warn, "git", "not installed: code impact and secure --history need it", "install git"));
    }
    rows
}

fn key_rows() -> Vec<Row> {
    let gemini = api_key("GEMINI_API_KEY").is_some();
    let speech = ["GROQ_API_KEY", "OPENAI_API_KEY"].iter().any(|k| api_key(k).is_some());
    let mut have = Vec::new();
    if gemini {
        have.push("Gemini");
    }
    if speech {
        have.push("speech to text");
    }
    if have.is_empty() {
        vec![row(
            Level::Warn,
            "keys",
            "no GEMINI_API_KEY or GROQ_API_KEY: watch uses frames, listen only captions",
            "free keys: aistudio.google.com/apikey, console.groq.com/keys",
        )]
    } else {
        vec![row(Level::Ok, "keys", format!("{} set", have.join(" + ")), "")]
    }
}

fn readable(path: &Path) -> Result<(), String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Value>(&text).map(|_| ()).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn size(path: &Path) -> u64 {
    match fs::metadata(path) {
        Ok(m) if m.is_dir() => fs::read_dir(path).into_iter().flatten().flatten().map(|e| size(&e.path())).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn mb(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1e6)
}

fn data_rows(home: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    let bad: Vec<String> = ["config.json", "channels.json", "sessions.json", "keys.json", "brain.json", "improve.json"]
        .iter()
        .filter_map(|f| readable(&home.join(f)).err())
        .collect();
    let db = home.join("memory.db");
    let db_state = if db.is_file() {
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|c| c.query_row("pragma schema_version", [], |r| r.get::<_, i64>(0)))
            .map(|_| format!("memory.db {}", mb(size(&db))))
            .map_err(|e| format!("memory.db: {e}"))
    } else {
        Ok("no memory.db yet".to_string())
    };
    match (bad.is_empty(), db_state) {
        (true, Ok(state)) => rows.push(row(Level::Ok, "data", format!("settings readable, {state}"), "")),
        (_, state) => {
            let mut why = bad;
            if let Err(e) = state {
                why.push(e);
            }
            rows.push(row(
                Level::Fail,
                "data",
                why.join("; "),
                "fix or delete the named file (a copy is in your backup)",
            ));
        }
    }
    let (out, log) = (size(&home.join("out")), size(&home.join("daemon.log")));
    let text = format!("out {}, daemon.log {}", mb(out), mb(log));
    rows.push(if out > BIG_OUT || log > BIG_LOG {
        row(Level::Warn, "disk", text, format!("delete old files in {}", home.join("out").display()))
    } else {
        row(Level::Ok, "disk", text, "")
    });
    rows
}

fn panics(home: &Path) -> (usize, String) {
    let Ok(mut f) = fs::File::open(home.join("daemon.log")) else { return (0, String::new()) };
    let len = f.metadata().map_or(0, |m| m.len());
    let mut bytes = Vec::new();
    if f.seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL))).and_then(|_| f.read_to_end(&mut bytes)).is_err() {
        return (0, String::new());
    }
    let text = String::from_utf8_lossy(&bytes);
    let text = text.rfind(STARTED).map_or(&text[..], |at| &text[at..]);
    let n = text.matches(PANIC).count();
    let last = text
        .rfind(PANIC)
        .map(|at| text[at + PANIC.len()..].lines().next().unwrap_or("").to_string())
        .and_then(|l| l.split_once(": ").map(|(_, why)| why.to_string()))
        .unwrap_or_default();
    (n, last)
}

fn error_rows(home: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    let (n, last) = panics(home);
    if n > 0 {
        let last: String = last.chars().take(SHOWN_CHARS).collect();
        rows.push(row(
            Level::Warn,
            "errors",
            format!(
                "{n} error{} inside the daemon since it started, last: {}",
                if n == 1 { "" } else { "s" },
                if last.is_empty() { "?" } else { &last }
            ),
            format!("report it with {}: github.com/QDHShamiro/Wisply/issues", home.join("daemon.log").display()),
        ));
    }
    let s = brain::summary(home, 7, now());
    if s.runs == 0 {
        rows.push(row(Level::Ok, "errors", "no commands in the last 7 days", ""));
        return rows;
    }
    let rate = brain::pct(s.failed, s.runs);
    match s.patterns.first() {
        Some(p) if s.failed >= MIN_FAILS && rate >= FAIL_PCT => {
            let most: String = p.error.chars().take(ERROR_CHARS).collect();
            rows.push(row(
                Level::Warn,
                "errors",
                format!("{rate:.0}% of {} commands failed in 7 days, most often: {most}", s.runs),
                "wisply improve",
            ));
        }
        _ => rows.push(row(Level::Ok, "errors", format!("{} commands in 7 days, {rate:.1}% failed", s.runs), "")),
    }
    rows
}

fn print(rows: &[Row]) -> ExitCode {
    let (mut fails, mut warns) = (0, 0);
    for r in rows {
        let tag = match r.level {
            Level::Ok => "ok  ",
            Level::Warn => {
                warns += 1;
                "warn"
            }
            Level::Fail => {
                fails += 1;
                "FAIL"
            }
        };
        let fix = if r.fix.is_empty() || r.level == Level::Ok { String::new() } else { format!(" -> fix: {}", r.fix) };
        println!("{tag} {:<7} {}{fix}", r.name, r.text);
    }
    let oks = rows.len() - fails - warns;
    let plural = |n: usize, w: &str| if n == 1 { format!("{n} {w}") } else { format!("{n} {w}s") };
    println!("{}, {}, {oks} ok", plural(fails, "problem"), plural(warns, "warning"));
    if fails > 0 { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

pub fn run(home: &Path, args: &[String], ask: Ask) -> ExitCode {
    if let Some(bad) = args.iter().find(|a| *a != "--fix") {
        eprintln!("err: doctor takes only --fix ({bad}) -> try: wisply doctor --fix");
        return ExitCode::from(2);
    }
    println!(
        "wisply doctor {} | {}",
        env!("CARGO_PKG_VERSION"),
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
    );
    if !args.is_empty() {
        let system = |p: Option<std::path::PathBuf>| p.filter(|p| !p.starts_with(home.join("bin")));
        for name in ["yt-dlp", "ffmpeg"] {
            let found = if name == "ffmpeg" { tools::ffmpeg(home) } else { tools::ytdlp(home).ok() };
            if let Some(path) = system(found) {
                println!("ok   {name:<7} uses {}, not touched", path.display());
                continue;
            }
            let got = if name == "ffmpeg" { install::ffmpeg(home) } else { install::ytdlp(home) };
            match got {
                Ok(text) => println!("fixed {name:<6} {text}"),
                Err(why) => println!("FAIL {name:<7} could not install: {why}"),
            }
        }
    }
    let mut rows = daemon_rows(home, ask);
    rows.extend(tool_rows(home));
    rows.extend(key_rows());
    rows.extend(data_rows(home));
    rows.extend(error_rows(home));
    print(&rows)
}
