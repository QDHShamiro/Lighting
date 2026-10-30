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
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-code")
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
fn the_code_map_finds_definitions_callers_outlines_and_bodies() {
    let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
    let path = base.join(format!("ltd-code-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    let project = path.join("demo");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(path.join("v3"), "").unwrap();
    fs::write(project.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    fs::write(project.join("src").join("main.rs"), "fn main() {\n    helper();\n}\n\nfn helper() {}\n").unwrap();
    let home = Home(path);
    let at = project.display().to_string();
    let (code, out) = home.run(&["code", "--project", &at]);
    assert!(
        code == 0 && out.contains("1 rust files (6 lines)") && out.contains("entry points: src/main.rs:1"),
        "{out}"
    );
    let (code, out) = home.run(&["code", "helper", "--project", &at]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.starts_with("defined:\n  src/main.rs:5 function helper\ncalled from (1):\n  src/main.rs:2 in main"),
        "{out}"
    );
    let (_, out) = home.run(&["code", "outline", "main.rs", "--project", &at]);
    assert_eq!(out, "src/main.rs (6 lines, rust):\n  1-3 fn main()\n  5-5 fn helper() {}");
    let (_, out) = home.run(&["code", "body", "main", "--project", &at]);
    assert_eq!(out, "src/main.rs:1-3\nfn main() {\n    helper();\n}");
    let (_, out) = home.run(&["code", "nothing_here", "--project", &at]);
    assert!(out.starts_with("nothing named \"nothing_here\" in demo"), "{out}");
}
