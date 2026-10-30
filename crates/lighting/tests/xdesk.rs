#![cfg(target_os = "linux")]

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_lighting");
const TITLE: &str = "LtProbe";

struct Home(PathBuf);

impl Home {
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(EXE)
            .args(args)
            .env("LIGHTING_HOME", &self.0)
            .env("HOME", &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-xdesk")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["stop"]);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn done(app: &mut Child, wait: Duration) -> Option<String> {
    let end = Instant::now() + wait;
    while Instant::now() < end {
        if app.try_wait().ok().flatten().is_some() {
            let mut out = String::new();
            std::io::Read::read_to_string(app.stdout.as_mut()?, &mut out).ok()?;
            return Some(out.trim().to_string());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = app.kill();
    None
}

#[test]
fn linux_apps_list_snap_type_read_and_click_through_atspi() {
    if std::env::var_os("LT_XDESK").is_none() {
        return;
    }
    let path = std::env::temp_dir().join(format!("ltd-xdesk-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("v3"), "").unwrap();
    let home = Home(path);
    let mut app = Command::new("zenity")
        .args(["--entry", &format!("--title={TITLE}"), "--text=Your name"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut list = String::new();
    for _ in 0..100 {
        list = home.run(&["windows", "-f", TITLE]).1;
        if list.contains("(chosen") {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    assert!(list.starts_with('w') && list.contains(TITLE) && list.contains("(chosen"), "{list}");
    let (code, snap) = home.run(&["snap"]);
    assert!(code == 0 && snap.starts_with("[w") && snap.contains(" controls)"), "{snap}");
    let field = snap.lines().find(|l| l.contains(" edit")).and_then(|l| l.split(' ').next()).expect(&snap).to_string();
    assert_eq!(home.run(&["type", &field, "Ada"]), (0, format!("ok (typed 3 chars into {field})")));
    let (_, again) = home.run(&["snap", "--full"]);
    assert!(again.contains(&format!("{field} edit")) && again.contains("=\"Ada\""), "{again}");
    let (code, read) = home.run(&["read"]);
    assert!(code == 0 && read.contains("Your name") && read.contains("(text, "), "{read}");
    let (code, out) = home.run(&["click", "OK"]);
    assert!(code == 0 && out.starts_with("ok (clicked d"), "{out}");
    assert_eq!(done(&mut app, Duration::from_secs(10)).as_deref(), Some("Ada"));
    let (code, out) = home.run(&["snap", "app:no-such-window-xyz"]);
    assert!(code == 1 && out.starts_with("err: no window matches \"no-such-window-xyz\""), "{out}");
    let (code, out) = home.run(&["hover", "w1"]);
    assert!(code == 1 && out.contains("would steal your mouse"), "{out}");
}
