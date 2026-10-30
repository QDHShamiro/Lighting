use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::json;

const EXE: &str = env!("CARGO_BIN_EXE_lighting");

struct Home(PathBuf);

impl Home {
    fn new() -> Home {
        let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
        let path = base.join(format!("ltd-brain-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("v3"), "").unwrap();
        Home(path)
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(EXE)
            .args(args)
            .env("LIGHTING_HOME", &self.0)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-brain")
            .env_remove("MSYSTEM")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["stop"]);
        for _ in 0..20 {
            if fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

fn click(label: &str, t: f64) -> serde_json::Value {
    json!({"cmd": "click", "args": [label], "flags": {}, "role": "button", "t": t,
           "where": {"kind": "web", "url": "https://shop.example/cart/12345", "title": "Cart"}})
}

#[test]
fn the_brain_learns_from_2x_episodes_and_improve_groups_errors() {
    let home = Home::new();
    let ep = json!({"t0": 1.0e9, "t1": 1.0e9, "start": null, "cost": 0, "via": null, "failed": null, "lossy": false,
        "params": null, "steps": [click("Checkout", 1.9e9), click("Checkout", 1.9e9), click("Apply coupon 10", 1.9e9)]});
    fs::write(home.0.join("episodes.jsonl"), format!("{ep}\n")).unwrap();
    let (code, out) = home.run(&["brain"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("brain: 2 pages and apps, 4 controls, 0 fixes learned | 0 snapshots trimmed"), "{out}");
    assert!(out.contains("\nshop.example/cart/# (3 uses): Checkout, Apply coupon 10\n"), "{out}");
    assert!(out.ends_with("brain.json) | brain forget <site> | brain reset --yes | config brain off"), "{out}");
    let (code, out) = home.run(&["brain", "reset"]);
    assert!(code == 1 && out.contains("-> try: lighting brain reset --yes"), "{out}");
    assert_eq!(home.run(&["brain", "forget", "https://shop.example/"]), (0, "forgot 2 pages".into()));
    assert_eq!(home.run(&["brain", "reset", "--yes"]), (0, "brain reset: nothing learned any more".into()));
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.0.join("brain.json")).unwrap()).unwrap();
    assert_eq!(saved["pages"], json!({}));
    for _ in 0..2 {
        assert_eq!(home.run(&["nonsense"]).0, 1);
    }
    let (_, out) = home.run(&["improve"]);
    assert!(out.starts_with("Lighting self-check, last 7 days: "), "{out}");
    assert!(
        out.contains("\n1. 2x unknown command 'nonsense' (nonsense) -> check the name in lighting help\n"),
        "{out}"
    );
    assert!(out.ends_with(
        "brain: 0 pages known, 0 fixes learned, 0 snapshots trimmed (-0 tokens), 0 refs regained, 0 fixes used"
    ));
}
