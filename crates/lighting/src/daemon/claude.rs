use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lt_core::{Call, Fail, local_parts};
use regex::Regex;
use serde_json::Value;

use super::ctx::{Ctx, now};
use super::media::tools::which;
use super::{Daemon, Out, Pending};

const TIMEOUT: Duration = Duration::from_secs(90);
const LOG_LINES: usize = 30;
const LIST_DAYS: f64 = 7.0;
const OUT_S: Duration = Duration::from_secs(30);
const NAME_CHARS: usize = 40;
const SHOWN_NAME: usize = 60;
const ERR_TAIL: usize = 200;
const BG_POLL: Duration = Duration::from_millis(300);
#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const NEW_GROUP: u32 = 0x0000_0200;
const SESSION_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_EFFORT",
    "CLAUDE_PID",
];
const NEEDS: &str = "claude --version (needs 2.1.300+)";

static BACKGROUNDED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"backgrounded\W+([0-9a-f]{6,})").expect("bg"));
static ANSI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07").expect("ansi"));
static UNSAFE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w #.-]").expect("name"));
#[cfg(windows)]
static READY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)shift\s*\+\s*tab|for shortcuts|bypass permissions|auto mode|accept edits").expect("ready")
});
#[cfg(windows)]
static TRUST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)trust (the|this) (files|folder)|vertraust").expect("trust"));

fn on_path(file: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?).map(|dir| dir.join(file)).find(|p| p.is_file())
}

fn shim() -> PathBuf {
    on_path("claude.cmd")
        .or_else(|| which("claude"))
        .unwrap_or_else(|| PathBuf::from(env::var_os("APPDATA").unwrap_or_default()).join("npm").join("claude.cmd"))
}

fn claude_exe() -> PathBuf {
    if !cfg!(windows) {
        return which("claude").unwrap_or_else(|| PathBuf::from("claude"));
    }
    if let Some(found) = which("claude") {
        return found;
    }
    let shim = shim();
    let native =
        shim.with_file_name("node_modules").join("@anthropic-ai").join("claude-code").join("bin").join("claude.exe");
    if native.is_file() { native } else { shim }
}

fn clean(cmd: &mut Command) -> &mut Command {
    for k in SESSION_ENV {
        cmd.env_remove(k);
    }
    cmd
}

#[cfg(windows)]
fn quiet(cmd: &mut Command, extra: u32) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(NO_WINDOW | extra);
}

fn claude_out(args: &[&str]) -> Result<String, Fail> {
    let failed = |e: String| Fail::new(format!("claude {} failed: {e}", args[0]));
    let mut cmd = Command::new(claude_exe());
    clean(&mut cmd).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    quiet(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(|e| failed(e.to_string()))?;
    let t0 = Instant::now();
    while child.try_wait().map_err(|e| failed(e.to_string()))?.is_none() {
        if t0.elapsed() > OUT_S {
            let _ = child.kill();
            return Err(failed(format!("timed out after {} s", OUT_S.as_secs())));
        }
        thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().map_err(|e| failed(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() && text.trim().is_empty() {
        let err = String::from_utf8_lossy(&out.stderr).split_whitespace().collect::<Vec<_>>().join(" ");
        let tail: String = err.chars().rev().take(ERR_TAIL).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(Fail::hint(
            format!("claude {} failed: {tail}", args[0]),
            "lighting claude list shows the session ids",
        ));
    }
    Ok(text)
}

fn stamp(ms: f64) -> String {
    let [_, mo, d, h, mi, _] = local_parts(ms / 1000.0);
    format!("{mo:02}-{d:02} {h:02}:{mi:02}")
}

pub fn list_text(rows: &[Value], all: bool, at: f64) -> String {
    let since = (at - LIST_DAYS * 86_400.0) * 1000.0;
    let started = |r: &Value| r["startedAt"].as_f64().unwrap_or(0.0);
    let bg: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "background").collect();
    let mut shown: Vec<&Value> =
        bg.iter().copied().filter(|r| all || (r["state"] != "stopped" && started(r) >= since)).collect();
    let hidden = bg.len() - shown.len();
    let more = if hidden > 0 {
        format!("\n({hidden} stopped or older than {LIST_DAYS} days: lighting claude list --all)")
    } else {
        String::new()
    };
    if shown.is_empty() {
        return format!("no background Claude sessions running{more}");
    }
    shown.sort_by(|a, b| started(b).total_cmp(&started(a)));
    let s = |r: &Value, k: &str| r[k].as_str().map_or_else(|| r[k].to_string(), String::from);
    let lines: Vec<String> = shown
        .iter()
        .map(|r| {
            let waiting =
                r["waitingFor"].as_str().filter(|w| !w.is_empty()).map_or(String::new(), |w| format!(" ({w})"));
            let name: String = r["name"].as_str().unwrap_or("").chars().take(SHOWN_NAME).collect();
            format!(
                "{} {}{waiting} \"{name}\" {}, started {}",
                s(r, "id"),
                s(r, "state"),
                s(r, "cwd"),
                stamp(started(r))
            )
        })
        .collect();
    lines.join("\n") + &more
}

pub fn log_tail(raw: &str, sid: &str) -> String {
    let text = ANSI.replace_all(raw, "");
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|x| {
            let t = x.trim();
            let rules = x.chars().filter(|c| *c == '─' || *c == '━').count();
            !t.is_empty() && rules < t.chars().count() / 2
        })
        .collect();
    if lines.is_empty() {
        return format!("no output yet from {sid}");
    }
    lines[lines.len().saturating_sub(LOG_LINES)..].join("\n")
}

pub fn session_name(given: Option<&str>, folder: &Path) -> String {
    let base = given.map_or_else(
        || format!("Claude {}", folder.file_name().map_or_else(|| "home".into(), |n| n.to_string_lossy().to_string())),
        String::from,
    );
    let cut: String = UNSAFE_NAME.replace_all(&base, "").chars().take(NAME_CHARS).collect();
    let cut = cut.trim();
    if cut.is_empty() { "Claude".into() } else { cut.to_string() }
}

fn passed(c: &Call, name: &str, mut args: Vec<String>) -> Vec<String> {
    for f in ["model", "effort"] {
        if let Some(v) = c.val(f).filter(|v| !v.is_empty()) {
            args.extend([format!("--{f}"), v.to_string()]);
        }
    }
    if !c.on("no-remote") {
        args.extend(["--remote-control".to_string(), name.to_string()]);
    }
    args
}

fn background(d: &Daemon, c: &Call, prompt: &str, folder: &Path, name: &str) -> Result<Out, Fail> {
    let mut args = passed(c, name, vec!["--bg".into(), "--name".into(), name.into()]);
    if !prompt.is_empty() {
        args.push(prompt.into());
    }
    let out_dir = d.home.join("out");
    let _ = fs::create_dir_all(&out_dir);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |t| t.as_nanos());
    let log = out_dir.join(format!("claude-bg-{}-{nanos}.txt", std::process::id()));
    let file = File::create(&log).map_err(|e| Fail::new(format!("could not write {}: {e}", log.display())))?;
    let err = file.try_clone().map_err(|e| Fail::new(e.to_string()))?;
    let mut cmd = Command::new(claude_exe());
    clean(&mut cmd).args(&args).current_dir(folder).stdin(Stdio::null()).stdout(file).stderr(err);
    #[cfg(windows)]
    quiet(&mut cmd, NEW_GROUP);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child: Child =
        cmd.spawn().map_err(|e| Fail::hint(format!("could not start claude: {e}"), "claude --version"))?;
    let worktree = if folder.ancestors().any(|p| p.join(".git").exists()) {
        ", edits go to a git worktree under .claude/worktrees"
    } else {
        ""
    };
    let head = format!(
        "started Claude \"{name}\" in the background ({}){}, no window{worktree}",
        folder.display(),
        if prompt.is_empty() { "" } else { ", prompt sent" }
    );
    Ok(Out::Wait(Pending::new(TIMEOUT, BG_POLL, move |_, _, last| {
        if !last && child.try_wait().ok().flatten().is_none() {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&fs::read(&log).unwrap_or_default()).to_string();
        let Some(m) = BACKGROUNDED.captures(&text) else {
            let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let tail: String = flat.chars().rev().take(ERR_TAIL).collect::<Vec<_>>().into_iter().rev().collect();
            let said = if tail.is_empty() { String::new() } else { format!(": {tail}") };
            return Err(Fail::hint(format!("claude --bg did not start{said}"), NEEDS));
        };
        let sid = &m[1];
        Ok(Some(format!("{head}: claude attach {sid} | claude logs {sid} | claude stop {sid}")))
    })))
}

#[cfg(windows)]
fn terminal(c: &Call, prompt: String, folder: &Path, name: String) -> Result<Out, Fail> {
    use super::desk::{find_tab, tab_text, type_prompt};
    let wt =
        which("wt").ok_or_else(|| Fail::hint("needs Windows Terminal", "winget install Microsoft.WindowsTerminal"))?;
    let name = if find_tab(&name).is_some() { format!("{name} {}", now() as u64 % 1000) } else { name };
    let base = vec![
        "-w".into(),
        "0".into(),
        "new-tab".into(),
        "--title".into(),
        name.clone(),
        "--suppressApplicationTitle".into(),
        "-d".into(),
        folder.display().to_string(),
        shim().display().to_string(),
    ];
    let args = passed(c, &name, base);
    let mut cmd = Command::new(wt);
    clean(&mut cmd).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map_err(|e| Fail::hint(format!("could not start Windows Terminal: {e}"), "wt --version"))?;
    let remote = if c.on("no-remote") { "" } else { ", remote control on (claude.ai/code or the Claude app)" };
    let head = format!("started Claude \"{name}\" in a new Windows Terminal tab ({}){remote}", folder.display());
    Ok(Out::Wait(Pending::new(TIMEOUT, Duration::from_secs(1), move |d, ctx, last| {
        let hwnd = find_tab(&name);
        let text = hwnd.map(tab_text).unwrap_or_default();
        if TRUST.is_match(&text) {
            return Err(Fail::new(format!(
                "{head}, but Claude asks whether to trust this folder: the user answers in the terminal"
            )));
        }
        if !READY.is_match(&text) {
            if last {
                return Err(Fail::new(format!("{head}, but it did not get ready within {} s", TIMEOUT.as_secs())));
            }
            return Ok(None);
        }
        let Some(hwnd) = hwnd.filter(|_| !prompt.is_empty()) else { return Ok(Some(head.clone())) };
        if find_tab(&name) != Some(hwnd) {
            return Err(Fail::new(format!("{head}, but its tab is no longer in front: prompt not sent")));
        }
        type_prompt(d, ctx, hwnd, &prompt)?;
        Ok(Some(format!("{head}, prompt sent ({} chars)", prompt.chars().count())))
    })))
}

#[cfg(not(windows))]
fn terminal(_: &Call, _: String, _: &Path, _: String) -> Result<Out, Fail> {
    Err(super::route::later("claude in a terminal window on this system (use --bg)"))
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let yes = c.on("yes");
    let first = c.pos.first().map(|p| p.to_lowercase());
    if !yes && first.as_deref() == Some("list") && c.pos.len() == 1 {
        let rows: Value = serde_json::from_str(claude_out(&["agents", "--json", "--all"])?.trim())
            .map_err(|_| Fail::hint("claude agents gave no list", NEEDS))?;
        return Ok(Out::Text(list_text(rows.as_array().map_or(&[][..], Vec::as_slice), c.on("all"), now())));
    }
    if !yes && first.as_deref() == Some("logs") {
        let [_, sid] = c.pos.as_slice() else {
            return Err(Fail::hint(
                "which session?",
                "lighting claude list shows the ids, then lighting claude logs <id>",
            ));
        };
        return Ok(Out::Text(log_tail(&claude_out(&["logs", sid])?, sid)));
    }
    let prompt = c.pos.join(" ").trim().to_string();
    if !prompt.is_empty() && !yes {
        return Err(Fail::hint(
            "a Claude session with a prompt starts working on its own: only when the user asked for it",
            "lighting claude \"the task\" --yes",
        ));
    }
    let folder = c
        .val("dir")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| (!ctx.cwd.is_empty()).then(|| PathBuf::from(&ctx.cwd)))
        .or_else(|| env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from))
        .unwrap_or_default();
    if !folder.is_dir() {
        return Err(Fail::new(format!("no folder {}", folder.display())));
    }
    let name = session_name(c.val("name").filter(|v| !v.is_empty()), &folder);
    if c.on("bg") {
        return background(d, c, &prompt, &folder, &name);
    }
    terminal(c, prompt, &folder, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn background_sessions_are_listed_newest_first() {
        let at = 1.9e9;
        let ms = |s: f64| s * 1000.0;
        let rows = vec![
            json!({"id": "aaa111", "kind": "background", "state": "running", "name": "old", "cwd": "E:/a", "startedAt": ms(at - 3600.0)}),
            json!({"id": "bbb222", "kind": "background", "state": "idle", "waitingFor": "input", "name": "new", "cwd": "E:/b", "startedAt": ms(at - 60.0)}),
            json!({"id": "ccc333", "kind": "background", "state": "stopped", "name": "done", "cwd": "E:/c", "startedAt": ms(at - 60.0)}),
            json!({"id": "ddd444", "kind": "interactive", "state": "running", "startedAt": ms(at)}),
        ];
        let text = list_text(&rows, false, at);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("bbb222 idle (input) \"new\" E:/b, started "), "{text}");
        assert!(lines[1].starts_with("aaa111 running \"old\" E:/a, started "), "{text}");
        assert_eq!(lines[2], "(1 stopped or older than 7 days: lighting claude list --all)");
        assert_eq!(list_text(&rows, true, at).lines().count(), 3);
        assert_eq!(list_text(&[], false, at), "no background Claude sessions running");
    }

    #[test]
    fn logs_drop_colors_and_rules_and_names_are_safe() {
        let raw = "\x1b[1mHello\x1b[0m\n────────────\n\n  done  \n\x1b]0;title\x07xyz\nx";
        assert_eq!(log_tail(raw, "abc"), "Hello\n  done\nxyz");
        assert_eq!(log_tail("\n──\n", "abc"), "no output yet from abc");
        assert_eq!(session_name(Some("My <Task>: v2!"), Path::new("E:/x")), "My Task v2");
        assert_eq!(session_name(None, Path::new("E:/Lighting-v3")), "Claude Lighting-v3");
        assert_eq!(session_name(Some("<>"), Path::new("E:/x")), "Claude");
    }
}
