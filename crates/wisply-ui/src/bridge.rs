use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tao::event_loop::EventLoopProxy;
use wisply_proto::{Conn, Reply, Request, WINDOW_SID, exchange_once, key_token};

use super::Ev;

const START_WAIT: Duration = Duration::from_secs(15);
const RETRY: Duration = Duration::from_millis(100);
const ALLOWED: &[&str] =
    &["board", "status", "abort", "tokens", "brain", "memory", "todo", "wiki", "config", "version"];
const CLI: &str = if cfg!(windows) { "wisply.exe" } else { "wisply" };

fn argv_of(msg: &Value) -> Vec<String> {
    msg["argv"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
}

pub fn allowed(argv: &[String]) -> bool {
    argv.first().is_some_and(|a| ALLOWED.contains(&a.as_str()))
}

pub fn ask(home: &Path, proxy: &EventLoopProxy<Ev>, body: &str) {
    let Ok(msg) = serde_json::from_str::<Value>(body) else { return };
    let id = msg["id"].as_u64().unwrap_or(0);
    let argv = argv_of(&msg);
    let secret = msg["body"].as_str().map(String::from);
    let (home, proxy) = (home.to_path_buf(), proxy.clone());
    thread::spawn(move || {
        let (code, out) = if allowed(&argv) {
            call(&home, &argv, secret.as_deref())
        } else {
            (2, "err: the window may not run this command".into())
        };
        let _ = proxy.send_event(Ev::Reply(id, code, out));
    });
}

fn start_daemon() {
    let Some(cli) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(CLI))) else { return };
    let mut cmd = Command::new(cli);
    cmd.arg("ping").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd.spawn();
}

fn call(home: &Path, argv: &[String], secret: Option<&str>) -> (i32, String) {
    let deadline = Instant::now() + START_WAIT;
    let mut started = false;
    loop {
        let token = key_token(home).unwrap_or_default();
        let req = Request { token: &token, cwd: "", sid: WINDOW_SID, secret, argv }.encode();
        match exchange_once(home, &req) {
            Ok(raw) if !raw.is_empty() => {
                let r = Reply::decode(&raw);
                return (r.code, r.out);
            }
            Ok(_) => return (1, "err: the Wisply daemon closed the connection".into()),
            Err(Conn::Missing) if !started => {
                started = true;
                start_daemon();
            }
            Err(_) => {}
        }
        if Instant::now() > deadline {
            return (1, "err: the Wisply daemon did not start".into());
        }
        thread::sleep(RETRY);
    }
}

pub fn open_outside(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.chars().any(char::is_whitespace) {
        return;
    }
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_only_reaches_read_and_manage_commands() {
        let v: Value = serde_json::from_str(r#"{"id": 3, "argv": ["board", "home", 5]}"#).unwrap();
        assert_eq!(argv_of(&v), ["board", "home"]);
        assert!(allowed(&argv_of(&v)));
        for bad in [["click", "e1"], ["open", "x"], ["type", "e2"]] {
            assert!(!allowed(&bad.map(String::from)));
        }
        assert!(!allowed(&[]));
    }
}
