use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

const EXE: &str = env!("CARGO_BIN_EXE_lighting");
const KEYS: [&str; 3] = ["GEMINI_API_KEY", "GROQ_API_KEY", "OPENAI_API_KEY"];

struct Home(PathBuf);

impl Home {
    fn run(&self, args: &[&str]) -> (i32, String) {
        let mut cmd = Command::new(EXE);
        cmd.args(args)
            .env("LIGHTING_HOME", &self.0)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-media")
            .env_remove("MSYSTEM")
            .stdin(Stdio::null());
        for key in KEYS {
            cmd.env_remove(key);
        }
        let out = cmd.output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["stop"]);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn watch_reads_a_local_clip_twice_at_once_and_names_missing_services() {
    let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
    let path = base.join(format!("ltd-media-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("v3"), "").unwrap();
    let home = Home(path.clone());
    let (code, out) = home.run(&["watch", "missing.mp4"]);
    assert!(code == 1 && out.starts_with("err: no file missing.mp4"), "{out}");
    let (code, out) = home.run(&["shot", "--ask", "what is shown?"]);
    assert!(code == 1 && out.starts_with("err: no Gemini key"), "{out}");
    if !ffmpeg() {
        assert!(std::env::var_os("LT_NEED_FFMPEG").is_none(), "ffmpeg should be installed here");
        eprintln!("ffmpeg missing: skipped the clip part");
        return;
    }
    let clip = path.join("clip.mp4");
    let made = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=4:size=320x240:rate=10"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-shortest", "-pix_fmt", "yuv420p"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(made.success());
    let shown = clip.display().to_string();
    let (code, out) = home.run(&["listen", &shown]);
    assert!(code == 1 && out.contains("no speech-to-text service"), "{out}");
    let runs: Vec<_> = (0..2)
        .map(|_| {
            let (root, shown) = (path.clone(), shown.clone());
            thread::spawn(move || {
                let home = Home(root);
                let got = home.run(&["watch", &shown, "--count", "4"]);
                std::mem::forget(home);
                got
            })
        })
        .collect();
    for run in runs {
        let (code, out) = run.join().unwrap();
        assert_eq!(code, 0, "{out}");
        assert!(out.starts_with("[watch] clip.mp4 - 0:04"), "{out}");
        assert!(out.contains("frames ") && out.contains(" scenes at "), "{out}");
        assert!(out.contains("! sound: no speech-to-text service"), "{out}");
        assert!(out.contains("! Gemini: no Gemini key"), "{out}");
    }
    let (code, out) = home.run(&["watch", &shown, "--frames", "--at", "0:01,0:03"]);
    assert!(code == 0 && out.contains("2 frames at the asked times at 0:01 0:03"), "{out}");
}
