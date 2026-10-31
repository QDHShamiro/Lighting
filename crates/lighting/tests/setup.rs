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
            .env("CLAUDE_CODE_SESSION_ID", "it-setup")
            .env_remove("MSYSTEM")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        (out.status.code().unwrap_or(-1), text.trim_end().to_string())
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["stop"]);
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn setup_fills_a_home_without_touching_the_browser_and_uninstall_clears_it() {
    let home = Home(std::env::temp_dir().join(format!("ltd-setup-{}", std::process::id())));
    let _ = fs::remove_dir_all(&home.0);
    let (code, out) = home.run(&["setup", "--manual"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("browser link and PATH left alone (WISPLY_HOME is set"), "{out}");
    assert!(out.contains("extension id gndlnbibcgfnkhgfipjpnpaadkghhgeg"), "{out}");
    let exe = if cfg!(windows) { "wisply.exe" } else { "wisply" };
    let host: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.0.join("dev.qdhshamiro.wisply.json")).unwrap()).unwrap();
    assert_eq!(host["path"].as_str().unwrap(), home.0.join("bin").join(exe).display().to_string());
    assert_eq!(host["allowed_origins"][0], "chrome-extension://gndlnbibcgfnkhgfipjpnpaadkghhgeg/");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.0.join("extension").join("manifest.json")).unwrap()).unwrap();
    assert!(manifest["version"].as_str().unwrap().chars().all(|c| c.is_ascii_digit() || c == '.'));
    assert!(
        home.0.join("v3").is_file() && home.0.join("blocklist.txt").is_file() && home.0.join("bin").join(exe).is_file()
    );
    assert_eq!(home.run(&["version"]).1, env!("CARGO_PKG_VERSION"));
    let fresh = "stopped the daemon that ran the old build, the next command starts the new one";
    assert_eq!(home.run(&["ping"]), (0, "pong".into()));
    let (_, again) = home.run(&["setup", "--manual"]);
    assert!(!again.contains(fresh), "{again}");
    let bin = home.0.join("bin").join(exe);
    fs::rename(&bin, bin.with_extension("was")).unwrap();
    fs::write(&bin, b"old build").unwrap();
    let (code, updated) = home.run(&["setup", "--manual"]);
    assert!(code == 0 && updated.contains(fresh), "{updated}");
    let (code, out) = home.run(&["install", "cursor"]);
    assert!(code == 1 && out.contains("add --register to point it here"), "{out}");
    let (code, out) = home.run(&["install", "nope"]);
    assert!(code == 1 && out.contains("usage: wisply install <antigravity|"), "{out}");
    let (code, out) = home.run(&["uninstall", "--purge"]);
    assert!(code == 0 && out.contains("memory and notes stay"), "{out}");
    assert!(!home.0.join("extension").exists() && !home.0.join("v3").exists());
    assert!(!home.0.join("dev.qdhshamiro.wisply.json").exists());
}

#[test]
fn the_skill_stays_within_its_byte_budget() {
    let skill = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/wisply/SKILL.md");
    let size = fs::metadata(&skill).unwrap().len();
    assert!(size <= 3_700, "SKILL.md grew to {size} bytes (budget 3700)");
}
