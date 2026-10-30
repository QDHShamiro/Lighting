use std::env;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use lt_proto::{Conn, Reply, Request};

use crate::daemon::MARKER;

const HOOK_INPUT_WAIT: Duration = Duration::from_millis(1000);
const HOOK_INPUT_MAX: u64 = 64 * 1024;
const START_WAIT: Duration = Duration::from_secs(12);

pub fn run(raw: &[String]) -> ExitCode {
    let args: Vec<String> = match msys_root() {
        Some(root) => raw.iter().map(|a| unmangle(a, &root)).collect(),
        None => raw.to_vec(),
    };
    let home = lt_proto::home();
    let first = args.first().map(String::as_str).unwrap_or("");
    if first == "more" {
        return crate::more::run(&home, &args[1..]);
    }
    if matches!(first, "setup" | "install" | "uninstall") {
        return crate::setup::run(&home, &args);
    }
    if !home.join(MARKER).is_file() {
        return v2(&home, raw);
    }
    if first == "doctor" {
        return crate::daemon::doctor(&home, &args[1..], ask);
    }
    if first == "ui" {
        return open_ui(&home);
    }
    if first == "squeeze" {
        return crate::pipe::squeeze(&home, &args);
    }
    let args = if matches!(first, "" | "-h" | "--help") {
        std::iter::once("help".to_string()).chain(args.into_iter().skip(1)).collect()
    } else {
        args
    };
    send(&home, &args)
}

fn open_ui(home: &Path) -> ExitCode {
    let name = if cfg!(windows) { "lighting-ui.exe" } else { "lighting-ui" };
    let ui = env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).filter(|p| p.is_file());
    let Some(ui) = ui else {
        println!("err: {name} is missing next to lighting -> try: reinstall Lighting");
        return ExitCode::from(1);
    };
    if crate::spawn::detached(&ui, &[], home, &home.join("ui.log")) {
        println!("ok (window opened)");
        ExitCode::SUCCESS
    } else {
        println!("err: could not start {name}");
        ExitCode::from(1)
    }
}

pub fn send(home: &Path, args: &[String]) -> ExitCode {
    let first = args.first().map(String::as_str).unwrap_or("");
    let quiet = args.iter().any(|a| a == "--quiet");
    let event = args.get(1).map(String::as_str).unwrap_or("");
    let silent = matches!(first, "done" | "suggest" | "hook");
    let idle = match first {
        "stop" => Some("lighting daemon not running"),
        "done" if quiet => Some(""),
        "done" => Some("nothing to close"),
        "suggest" => Some(""),
        "hook" if event != "start" => Some(""),
        _ => None,
    };
    let hook = first == "hook" && args.len() == 2;
    let wants_input = (first == "suggest" && args.len() == 1) || (first == "done" && quiet) || hook;
    let hook_input = if wants_input && !io::stdin().is_terminal() { read_hook_input() } else { String::new() };
    let mut sid = env::var("CLAUDE_CODE_SESSION_ID").unwrap_or_default();
    if sid.is_empty() {
        sid = json_str(&hook_input, "session_id");
    }
    let started = Instant::now();
    let (mut stats, mut secret, mut argv) = (false, None, Vec::with_capacity(args.len() + 1));
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--stats" {
            stats = true;
        } else if a == "--env" && i + 1 < args.len() {
            match env::var(&args[i + 1]) {
                Ok(v) => secret = Some(v),
                Err(_) => {
                    eprintln!("err: environment variable {} is not set", args[i + 1]);
                    return ExitCode::from(2);
                }
            }
            i += 1;
        } else {
            argv.push(a.clone());
        }
        i += 1;
    }
    if (first == "suggest" || hook) && !hook_input.is_empty() {
        argv.push(hook_input.replace('\0', " "));
    }
    let token = match lt_proto::key_token(home) {
        Some(token) => token,
        None if silent => return ExitCode::SUCCESS,
        None => match first_key(home) {
            Some(token) => token,
            None => {
                eprintln!("lighting: the daemon made no key, see {}", home.join("daemon.log").display());
                return ExitCode::from(1);
            }
        },
    };
    let cwd = env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    let req = Request { token: &token, cwd: &cwd, sid: &sid, secret: secret.as_deref(), argv: &argv }.encode();
    let reply = match exchange(home, &req, idle) {
        Ok(reply) => Reply::decode(&reply),
        Err(why) => {
            eprintln!("lighting: {why}, see {}", home.join("daemon.log").display());
            return ExitCode::from(1);
        }
    };
    let mut out = reply.out;
    if stats {
        out.push_str(&format!("\n[{} ms, ~{} tokens]", started.elapsed().as_millis(), out.len() / 4 + 1));
    }
    if !out.is_empty() {
        let mut so = io::stdout().lock();
        let _ = so.write_all(out.as_bytes());
        let _ = so.write_all(b"\n");
        let _ = so.flush();
    }
    ExitCode::from(u8::try_from(reply.code).unwrap_or(1))
}

pub fn ask(home: &Path, argv: &[&str]) -> Result<(i32, String), String> {
    let token = lt_proto::key_token(home).or_else(|| first_key(home)).ok_or("the daemon made no key")?;
    let cwd = env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    let argv: Vec<String> = argv.iter().map(|a| (*a).to_string()).collect();
    let req = Request { token: &token, cwd: &cwd, sid: "", secret: None, argv: &argv }.encode();
    let reply = Reply::decode(&exchange(home, &req, None)?);
    Ok((reply.code, reply.out))
}

fn v2(home: &Path, raw: &[String]) -> ExitCode {
    let Some(exe) = crate::v2::client_exe() else {
        eprintln!("lighting: {} belongs to Lighting 2.x and its client was not found", home.display());
        return ExitCode::from(127);
    };
    match Command::new(&exe).args(raw).status() {
        Ok(status) => ExitCode::from(status.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("lighting: cannot start the 2.x client {} ({e})", exe.display());
            ExitCode::from(127)
        }
    }
}

pub fn start(home: &Path) {
    let Ok(exe) = env::current_exe() else { return };
    crate::spawn::detached(&exe, &["daemon"], home, &home.join("daemon.log"));
}

fn first_key(home: &Path) -> Option<String> {
    start(home);
    let deadline = Instant::now() + START_WAIT;
    while Instant::now() < deadline {
        if let Some(token) = lt_proto::key_token(home) {
            return Some(token);
        }
        thread::sleep(Duration::from_millis(40));
    }
    None
}

fn exchange(home: &Path, req: &[u8], idle: Option<&str>) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + START_WAIT;
    let mut spawned = false;
    loop {
        match lt_proto::exchange_once(home, req) {
            Ok(buf) if !buf.is_empty() => return Ok(buf),
            Ok(_) => return Err("the daemon closed the connection".into()),
            Err(e) => {
                let busy = matches!(e, Conn::Busy);
                if !busy && !spawned {
                    if let Some(text) = idle {
                        return Ok(format!("0\0\0{text}").into_bytes());
                    }
                    start(home);
                    spawned = true;
                }
                if Instant::now() > deadline {
                    return Err("the daemon did not start".into());
                }
                thread::sleep(Duration::from_millis(if busy { 15 } else { 40 }));
            }
        }
    }
}

fn msys_root() -> Option<String> {
    env::var_os("MSYSTEM")?;
    let exe = env::var("EXEPATH").ok()?.replace('\\', "/");
    let root = exe.trim_end_matches('/');
    let root = root.strip_suffix("/usr/bin").or_else(|| root.strip_suffix("/bin")).unwrap_or(root);
    Some(root.to_string())
}

fn strip_root(value: &str, prefix: &str) -> Option<String> {
    let head = value.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| format!("/{}", &value[prefix.len()..]))
}

fn unmangle(arg: &str, root: &str) -> String {
    if arg.eq_ignore_ascii_case(root) {
        return "/".to_string();
    }
    let prefix = format!("{root}/");
    if let Some(v) = strip_root(arg, &prefix) {
        return v;
    }
    if let Some(i) = arg.find('=') {
        let (key, value) = arg.split_at(i + 1);
        if let Some(v) = strip_root(value, &prefix) {
            return format!("{key}{v}");
        }
    }
    arg.to_string()
}

fn json_str(text: &str, key: &str) -> String {
    let pat = format!("\"{key}\"");
    let Some(at) = text.find(&pat) else { return String::new() };
    let rest = text[at + pat.len()..].trim_start();
    let Some(rest) = rest.strip_prefix(':') else { return String::new() };
    let Some(rest) = rest.trim_start().strip_prefix('"') else { return String::new() };
    rest.split('"').next().unwrap_or("").to_string()
}

fn read_hook_input() -> String {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = io::stdin().take(HOOK_INPUT_MAX).read_to_string(&mut text);
        let _ = tx.send(text);
    });
    rx.recv_timeout(HOOK_INPUT_WAIT).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undoes_git_bash_path_rewrites() {
        let root = "C:/Program Files/Git";
        assert_eq!(unmangle("C:/Program Files/Git/QDHShamiro", root), "/QDHShamiro");
        assert_eq!(unmangle("c:/program files/git", root), "/");
        assert_eq!(unmangle("a=C:/Program Files/Git/b", root), "a=/b");
        assert_eq!(unmangle("C:/Users/x", root), "C:/Users/x");
        assert_eq!(unmangle("C:/Program Files/Gitx", root), "C:/Program Files/Gitx");
    }

    #[test]
    fn reads_the_session_from_hook_json() {
        let hook = r#"{"session_id": "4717b1cc-c49d", "prompt": "hi \"x\""}"#;
        assert_eq!(json_str(hook, "session_id"), "4717b1cc-c49d");
        assert_eq!(json_str(r#"{"session_id":"abc"}"#, "session_id"), "abc");
        assert_eq!(json_str("{}", "session_id"), "");
        assert_eq!(json_str("", "session_id"), "");
    }
}
