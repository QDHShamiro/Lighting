use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_wisply");

struct Home(PathBuf);

impl Home {
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(EXE)
            .args(args)
            .env("WISPLY_HOME", &self.0)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-secure")
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
fn secure_finds_a_key_a_known_hole_and_joined_sql() {
    let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
    let path = base.join(format!("ltd-secure-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    let repo = path.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(path.join("v3"), "").unwrap();
    fs::write(
        repo.join("config.yml"),
        format!("api_key: AIza{}\npassword: changeme\n", "Bq7x".repeat(9).get(..35).unwrap()),
    )
    .unwrap();
    fs::write(repo.join("requirements.txt"), "requests==2.19.0\n").unwrap();
    fs::write(repo.join("Db.java"), "stmt.executeQuery(\"SELECT * FROM users WHERE name = '\" + name + \"'\");\n")
        .unwrap();
    let home = Home(path);
    let at = repo.display().to_string();
    let (code, out) = home.run(&["secure", &at]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("secrets (1):\n  config.yml:1 Google API key (AIza…)"), "{out}");
    assert!(out.contains("requirements.txt:1 requests 2.19.0 has ") && out.contains("update to 2."), "{out}");
    assert!(out.contains("risky spots to look at (1):\n  Db.java:1 SQL built from joined text"), "{out}");
    let (code, out) = home.run(&["secure", &at, "--no-deps"]);
    assert!(code == 0 && !out.contains("requests 2.19.0"), "{out}");
}
