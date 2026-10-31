use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

const EXE: &str = env!("CARGO_BIN_EXE_wisply");
const TASK: &str = env!("CARGO_BIN_EXE_wisply-task");

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Home {
        let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
        let path = base.join(format!("ltd-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("routines")).unwrap();
        fs::write(path.join("v3"), "").unwrap();
        Home(path)
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(EXE)
            .args(args)
            .env("WISPLY_HOME", &self.0)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-schedule")
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
        for _ in 0..30 {
            if fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}

#[test]
fn the_task_runner_starts_wisply_without_a_window_and_logs() {
    let home = Home::new("task");
    let status = Command::new(TASK).arg(&home.0).arg("version").status().unwrap();
    assert!(status.success());
    let log = fs::read_to_string(home.0.join("schedule.log")).unwrap();
    assert_eq!(log.trim(), env!("CARGO_PKG_VERSION"));
    assert_eq!(Command::new(TASK).status().unwrap().code(), Some(2));
    assert_eq!(home.run(&["routine", "schedules"]).1.split(" -> ").next(), Some("no scheduled routines"));
    let (code, out) = home.run(&["routine", "unschedule", "nope"]);
    assert!(code == 1 && out.starts_with("err: nope has no schedule"), "{out}");
}

#[test]
fn a_routine_runs_from_the_task_scheduler() {
    if !cfg!(windows) || (std::env::var_os("CI").is_none() && std::env::var_os("LT_SCHEDULE_TEST").is_none()) {
        return;
    }
    let home = Home::new("schedule");
    let routine = json!({"name": "sched-check", "version": 1, "source": "saved", "created": "2026-10-30 18:00",
        "params": {"word": "x"}, "steps": [{"cmd": "remember", "args": ["scheduled {word}"], "flags": {}}],
        "check": null, "cost": 100, "stats": {"runs": 0, "ok": 0, "fail": 0, "ms": 0, "saved": 0, "seen": 1, "recent": []},
        "flaky": false});
    fs::write(home.0.join("routines").join("sched-check.json"), routine.to_string()).unwrap();
    let (code, out) = home.run(&["routine", "schedule", "sched-check", "every", "30m", "word=tea"]);
    assert_eq!((code, out.as_str()), (0, "scheduled sched-check every 30 min word=tea (Task Scheduler)"));
    assert_eq!(home.run(&["routine", "schedules"]).1, "sched-check  every 30 min word=tea");
    let ran = Command::new("schtasks").args(["/Run", "/TN", "Wisply sched-check"]).output().unwrap();
    assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    let t0 = Instant::now();
    let log = home.0.join("schedule.log");
    while t0.elapsed() < Duration::from_secs(60)
        && !fs::read_to_string(&log).unwrap_or_default().contains("ok sched-check")
    {
        thread::sleep(Duration::from_millis(300));
    }
    let said = fs::read_to_string(&log).unwrap_or_default();
    let (code, out) = home.run(&["routine", "rm", "sched-check"]);
    assert_eq!((code, out.as_str()), (0, "removed routine sched-check and its schedule"));
    assert!(said.contains("ok sched-check: 1 steps in "), "{said}");
    let gone = Command::new("schtasks").args(["/Query", "/TN", "Wisply sched-check"]).output().unwrap();
    assert!(!gone.status.success());
}
