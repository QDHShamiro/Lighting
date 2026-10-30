use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const EXE: &str = env!("CARGO_BIN_EXE_lighting");

struct Home(PathBuf);

impl Home {
    fn new(tag: &str, v3: bool) -> Home {
        let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
        let path = base.join(format!("ltd-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        if v3 {
            fs::write(path.join("v3"), "").unwrap();
        }
        Home(path)
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(EXE)
            .args(args)
            .env("LIGHTING_HOME", &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-1")
            .env_remove("MSYSTEM")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    }

    fn rows(&self) -> Vec<Value> {
        let text = fs::read_to_string(self.0.join("log.jsonl")).unwrap_or_default();
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if self.0.join("v3").exists() {
            let _ = self.run(&["stop"]);
        }
        for _ in 0..20 {
            if fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[test]
fn a_v3_home_starts_its_daemon_and_answers_like_2x() {
    let home = Home::new("core", true);
    assert_eq!(home.run(&["ping"]), (0, "pong".into()));
    assert!(home.0.join("daemon.pid").exists() && home.0.join("key").exists());
    let (code, status) = home.run(&["status"]);
    assert_eq!(code, 0);
    assert!(status.starts_with(&format!("lighting {} | daemon pid ", env!("CARGO_PKG_VERSION"))), "{status}");
    assert_eq!(home.run(&["zzz"]), (1, "err: unknown command 'zzz' -> try: lighting help".into()));
    let (code, snap) = home.run(&["snap"]);
    assert!(code == 1 && snap.starts_with("err: snap is not built into Lighting 3.0 yet"), "{snap}");
    home.run(&["zzz"]);
    let (_, third) = home.run(&["zzz"]);
    assert!(third.ends_with("failed 3 times in a row with this error: repeating it will not help"), "{third}");
    let (_, log) = home.run(&["log", "2"]);
    assert!(log.lines().count() == 2 && log.lines().all(|l| l.contains(" ERR zzz ")), "{log}");
    let rows = home.rows();
    let cmds: Vec<&str> = rows.iter().filter_map(|r| r["cmd"].as_str()).collect();
    assert_eq!(cmds, ["zzz", "snap", "zzz", "zzz"]);
    let last = rows.last().unwrap();
    assert_eq!((last["sid"].as_str(), last["ok"].as_bool()), (Some("it-1"), Some(false)));
    assert_eq!(last["e"].as_str(), Some("unknown command 'zzz'"));
    assert!(last["id"].as_str().is_some_and(|id| id.ends_with(".6")), "{last}");
    let (code, help) = home.run(&[]);
    assert!(code == 0 && help.starts_with("lighting 3.0 core"), "{help}");
    assert_eq!(home.run(&["abort"]), (0, "stopped the running and queued commands".into()));
    assert_eq!(home.run(&["stop"]), (0, "lighting daemon stopped".into()));
    assert!(!home.0.join("daemon.pid").exists());
    thread::sleep(Duration::from_millis(300));
    assert_eq!(home.run(&["stop"]), (0, "lighting daemon not running".into()));
}

#[test]
fn the_daemon_reads_json_and_drops_wrong_tokens() {
    let home = Home::new("wire", true);
    assert_eq!(home.run(&["ping"]).1, "pong");
    let Some(token) = lt_proto::key_token(&home.0) else { panic!("no key") };
    let ask = json!({"token": token, "argv": ["version"], "sid": "j"}).to_string();
    let Ok(reply) = lt_proto::exchange_once(&home.0, ask.as_bytes()) else { panic!("no daemon") };
    let reply: Value = serde_json::from_slice(&reply).unwrap();
    assert_eq!(reply, json!({"out": env!("CARGO_PKG_VERSION"), "code": 0}));
    let Ok(reply) = lt_proto::exchange_once(&home.0, b"L2\0bad\0\0\0-\0ping") else { panic!("no daemon") };
    assert!(reply.is_empty());
    assert_eq!(home.run(&["ping"]).1, "pong");
}

#[test]
fn more_reads_kept_outputs_without_a_daemon() {
    let home = Home::new("more", false);
    let out = home.0.join("headroom").join("out");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("hr-0123456789.txt"), "one\ntwo\nthree\n").unwrap();
    assert_eq!(home.run(&["more", "hr-0123456789", "--lines", "2-3"]), (0, "two\nthree".into()));
    assert_eq!(home.run(&["more", "hr-0123456789", "--grep", "THREE"]), (0, "3: three".into()));
    let (code, missing) = home.run(&["more", "hr-ffffffffff"]);
    assert!(code == 1 && missing.starts_with("err: hr-ffffffffff is not kept any more"), "{missing}");
}
