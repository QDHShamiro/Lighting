#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_lighting");

struct Home(PathBuf);

impl Home {
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(EXE)
            .args(args)
            .env("LIGHTING_HOME", &self.0)
            .env("USERPROFILE", &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-desk")
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
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn apps_list_windows_share_the_clipboard_and_name_missing_windows() {
    let path = std::env::temp_dir().join(format!("ltd-desk-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("v3"), "").unwrap();
    let home = Home(path);
    let (code, list) = home.run(&["windows"]);
    assert!(code == 0 && (list == "no windows" || list.starts_with("w1 ")), "{list}");
    let (_, saved) = home.run(&["clip"]);
    assert_eq!(home.run(&["clip", "set", "lighting desk test"]), (0, "ok (clipboard set)".into()));
    let (_, got) = home.run(&["clip"]);
    if saved != "(clipboard is empty)" && saved != "err: clipboard is busy" {
        home.run(&["clip", "set", &saved]);
    }
    assert_eq!(got, "lighting desk test");
    let (code, out) = home.run(&["snap", "app:no-such-window-xyz"]);
    assert!(code == 1 && out.starts_with("err: no window matches \"no-such-window-xyz\""), "{out}");
    let (code, out) = home.run(&["hover", "w1"]);
    assert!(code == 1 && out.contains("hover on desktop apps would steal your mouse"), "{out}");
    assert_eq!(home.run(&["keep", "w99"]), (0, "nothing to keep".into()));
    let (code, out) = home.run(&["launch"]);
    assert!(code == 1 && out.starts_with("err: launch needs an app name, URI or path"), "{out}");
    let (code, out) = home.run(&["launch", "ms-msdt:x"]);
    assert!(code == 1 && out.starts_with("err: ms-msdt: links can run programs or open remote files"), "{out}");
}
