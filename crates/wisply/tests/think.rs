use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::json;

const EXE: &str = env!("CARGO_BIN_EXE_wisply");

struct Home(PathBuf);

impl Home {
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(EXE);
        cmd.args(args)
            .env("WISPLY_HOME", &self.0)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-think")
            .env_remove("MSYSTEM");
        cmd
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        self.feed(args, None)
    }

    fn feed(&self, args: &[&str], input: Option<&str>) -> (i32, String) {
        let mut child = self
            .command(args)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(text) = input {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(text.as_bytes()).unwrap();
        }
        let out = child.wait_with_output().unwrap();
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
fn think_finds_fixes_and_reports_without_ai() {
    let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
    let path = base.join(format!("ltd-think-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    let proj = path.join("proj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(path.join("v3"), "").unwrap();
    let readme = proj.join("README.md");
    fs::write(&readme, "# Demo\n\nThis is is a test.\n\nSee [x](missing.md).\n").unwrap();
    let home = Home(path);
    let at = proj.display().to_string();

    let (code, out) = home.run(&["think", "start", "demo", &at, "--goal", "clean docs", "--quick"]);
    assert!(code == 0 && out.contains("size: file -> at least 10 ideas") && out.contains("checks: docs"), "{out}");

    let (code, out) = home.run(&["think", "scan"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("README.md:5 broken-link: link to missing.md goes nowhere"), "{out}");
    assert!(out.contains("README.md:3 repeated-word: 1 lines repeat a word (\"is is\")"), "{out}");

    let (code, out) = home.feed(&["think", "idea", "add"], Some("Shorter intro | text | invert | 2 | 2 | 1 | safe\n"));
    assert!(code == 0 && out.contains("+1 ideas (total 3/10)"), "{out}");

    let (code, out) = home.run(&["think", "fix"]);
    assert!(code == 0 && out.contains("drop repeated words (1 files)"), "{out}");
    assert_eq!(fs::read_to_string(&readme).unwrap(), "# Demo\n\nThis is a test.\n\nSee [x](missing.md).\n");

    let (code, out) = home.run(&["think", "idea", "list"]);
    assert!(code == 0 && out.contains("(README.md:3) | docs | safe | 3 | done | wisply |"), "{out}");

    let (code, out) = home.run(&["think", "round"]);
    assert!(code != 0 && out.contains("LOCKED: no criteria"), "{out}");

    let (code, out) = home.run(&["think", "report"]);
    assert!(code == 0 && out.contains("**Critical thinking - demo**") && out.contains("| Shorter intro |"), "{out}");

    let java = proj.join("A.java");
    fs::write(&java, "class A {\n  void a() { e.printStackTrace(); }\n}\n").unwrap();
    let event =
        json!({"tool_name": "Write", "session_id": "s", "tool_input": {"file_path": java.display().to_string()}});
    let (code, out) = home.feed(&["hook", "post"], Some(&event.to_string()));
    assert_eq!(code, 0, "{out}");
    let reply: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("{out}"));
    let context = reply["hookSpecificOutput"]["additionalContext"].as_str().unwrap_or_default();
    assert!(context.contains("wisply think: 1 new in A.java") && context.contains("printStackTrace"), "{context}");
    let (_, again) = home.feed(&["hook", "post"], Some(&event.to_string()));
    assert_eq!(again, "");
}
