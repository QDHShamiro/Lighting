use lt_proto::home;
use lt_squeeze::{commas, squeeze};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::cmp::Reverse;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MIN_CHARS: usize = 2000;
const BUDGET: Duration = Duration::from_millis(50);
const HOLDOUT_EVERY: u64 = 10;
const ERRORS_MAX: usize = 3;
const ERRORS_WINDOW_S: u64 = 600;
const KEEP_FOR: Duration = Duration::from_secs(7 * 86_400);
const KEEP_BYTES: u64 = 200_000_000;
const TIDY_EVERY: Duration = Duration::from_secs(3600);
const LOG_MAX: u64 = 16_000_000;
const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;
const SEGMENT: &str = r"(?:^|[;\n(]|&&|\|\|)\s*(?:(?:then|do|else)\s+)?(?:rtk\s+)?";
const GIT: &str = r"git(?:\s+(?:-C\s+\S+|-c\s+\S+|--no-pager))*\s+";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("hook pattern")
}

static FILE_READ: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"{SEGMENT}(?:cat|head|tail|sed\s+-n)\b([^|;&\n]*)")));
static DIFF: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"{SEGMENT}(?:{GIT}(?:diff|show)|diff)\b")));
static LISTING: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"{SEGMENT}(?:ls|dir|tree|find|fd|{GIT}(?:log|ls-files|status|blame|rev-list|show-ref|branch|tag)|rg\s+(?:--files|-l)|grep\s+-[A-Za-z]*l)\b"
    ))
});
static OWN: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"{SEGMENT}(?:\S*[/\\])?lighting(?:\.exe)?\b")));
static LOGISH: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\.(?:log|jsonl|out|output|txt|csv)\b|/logs?/|results\.md|tasks/"));
static KEPT: LazyLock<Regex> = LazyLock::new(|| re(r"headroom[/\\]+out[/\\]+(hr-[0-9a-f]{10})"));

struct Paths {
    state: PathBuf,
    out: PathBuf,
}

impl Paths {
    fn new(root: &Path) -> Self {
        let state = root.join("headroom");
        Paths { out: state.join("out"), state }
    }
}

pub fn post() {
    let started = Instant::now();
    panic::set_hook(Box::new(|_| {}));
    let paths = Paths::new(&home());
    if paths.state.join("off").exists() {
        return;
    }
    let mut input = Vec::new();
    if io::stdin().lock().read_to_end(&mut input).is_err() {
        return;
    }
    let (reply, kept) = run(&input, &paths, started, BUDGET);
    if let Some(text) = reply {
        let mut out = io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
    if kept {
        tidy(&paths);
    }
}

fn run(input: &[u8], paths: &Paths, started: Instant, budget: Duration) -> (Option<String>, bool) {
    match panic::catch_unwind(|| handle(input, paths, started, budget)) {
        Ok(Ok(reply)) => {
            let kept = reply.is_some();
            (reply, kept)
        }
        Ok(Err(why)) => (failed(paths, &why), false),
        Err(_) => (failed(paths, "the hook crashed"), false),
    }
}

fn handle(input: &[u8], paths: &Paths, started: Instant, budget: Duration) -> Result<Option<String>, String> {
    let event: Value = serde_json::from_slice(input).map_err(|e| format!("hook input is not JSON ({e})"))?;
    let tool = event["tool_name"].as_str().unwrap_or_default();
    let mut entry = Map::new();
    entry.insert("t".into(), json!(unix_ms()));
    entry.insert("sid".into(), event["session_id"].clone());
    entry.insert("tool".into(), json!(tool));
    let reply = if tool == "Bash" {
        bash(&event, paths, started + budget, &mut entry)?
    } else {
        if let Some(name) = kept_ref(&event["tool_input"].to_string()) {
            entry.insert("act".into(), json!("reload"));
            entry.insert("ref".into(), json!(name));
        }
        None
    };
    if entry.contains_key("act") {
        entry.insert("ms".into(), json!(started.elapsed().as_millis() as u64));
        note(paths, &Value::Object(entry));
    }
    Ok(reply)
}

fn act(entry: &mut Map<String, Value>, why: &str) -> Result<Option<String>, String> {
    entry.insert("act".into(), json!(why));
    Ok(None)
}

fn bash(
    event: &Value,
    paths: &Paths,
    deadline: Instant,
    entry: &mut Map<String, Value>,
) -> Result<Option<String>, String> {
    let cmd = event["tool_input"]["command"].as_str().unwrap_or_default();
    let response = &event["tool_response"];
    let Some(stdout) = response.get("stdout").and_then(Value::as_str) else { return act(entry, "shape") };
    let chars = stdout.chars().count();
    entry.insert("in".into(), json!(chars));
    if response.get("isImage").and_then(Value::as_bool) == Some(true) {
        return act(entry, "image");
    }
    if let Some(name) = kept_ref(cmd) {
        entry.insert("ref".into(), json!(name));
        return act(entry, "reload");
    }
    if let Some(why) = untouchable(cmd) {
        return act(entry, why);
    }
    if chars <= MIN_CHARS {
        return act(entry, "small");
    }
    let text = stdout.to_owned();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(squeeze(&text));
    });
    let squeezed = match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(s) => s,
        Err(RecvTimeoutError::Timeout) => return act(entry, "slow"),
        Err(RecvTimeoutError::Disconnected) => return Err("squeeze crashed".into()),
    };
    let Some(s) = squeezed else { return act(entry, "same") };
    let name = format!("hr-{:010x}", fnv(&[event["tool_use_id"].as_str().unwrap_or_default(), s.plain.as_str()]) >> 24);
    let file = paths.out.join(format!("{name}.txt"));
    let new = format!(
        "lighting squeezed {} lines (~{} tokens) to {} lines (~{} tokens), all of it: {}\n{}",
        commas(s.lines_in),
        commas(s.chars_in / 4),
        s.lines_out,
        commas(s.chars_out / 4),
        slashed(&file),
        s.body
    );
    let small = new.chars().count();
    if small >= chars {
        return act(entry, "same");
    }
    entry.insert("out".into(), json!(small));
    entry.insert("kind".into(), json!(if s.json { "json" } else { "log" }));
    entry.insert("ref".into(), json!(name));
    if fnv(&[event["session_id"].as_str().unwrap_or_default()]).is_multiple_of(HOLDOUT_EVERY) {
        return act(entry, "holdout");
    }
    fs::create_dir_all(&paths.out)
        .and_then(|()| fs::write(&file, &s.plain))
        .map_err(|e| format!("cannot keep the full output ({e})"))?;
    entry.insert("act".into(), json!("cut"));
    let mut updated = response.clone();
    updated["stdout"] = Value::String(new);
    Ok(Some(json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "updatedToolOutput": updated}}).to_string()))
}

fn untouchable(cmd: &str) -> Option<&'static str> {
    if OWN.is_match(cmd) {
        Some("own")
    } else if DIFF.is_match(cmd)
        || LISTING.is_match(cmd)
        || FILE_READ.captures_iter(cmd).any(|c| !LOGISH.is_match(&c[1]))
    {
        Some("exact")
    } else {
        None
    }
}

fn kept_ref(text: &str) -> Option<String> {
    KEPT.captures(text).map(|c| c[1].to_string())
}

fn failed(paths: &Paths, why: &str) -> Option<String> {
    let now = unix_ms() / 1000;
    let file = paths.state.join("errors.txt");
    let mut times: Vec<u64> = fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .filter(|t| now.saturating_sub(*t) < ERRORS_WINDOW_S)
        .collect();
    times.push(now);
    let _ = fs::create_dir_all(&paths.state);
    let _ = fs::write(&file, times.iter().map(u64::to_string).collect::<Vec<_>>().join("\n") + "\n");
    note(paths, &json!({"t": unix_ms(), "act": "error", "why": why}));
    if times.len() < ERRORS_MAX {
        return None;
    }
    let off = paths.state.join("off");
    let _ = fs::write(&off, format!("{why}\n"));
    let message = format!(
        "Lighting stopped squeezing tool outputs after {ERRORS_MAX} errors in {} minutes (last: {why}). Outputs come through in full now; delete {} to turn it back on.",
        ERRORS_WINDOW_S / 60,
        slashed(&off)
    );
    Some(json!({ "systemMessage": message }).to_string())
}

fn note(paths: &Paths, entry: &Value) {
    let file = paths.state.join("log.jsonl");
    if fs::metadata(&file).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = fs::rename(&file, paths.state.join("log.1.jsonl"));
    }
    if fs::create_dir_all(&paths.state).is_err() {
        return;
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&file) {
        let _ = f.write_all(format!("{entry}\n").as_bytes());
    }
}

fn tidy(paths: &Paths) {
    let mark = paths.state.join("tidied");
    let recent = fs::metadata(&mark).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
    if recent.is_some_and(|age| age < TIDY_EVERY) {
        return;
    }
    let _ = fs::create_dir_all(&paths.state);
    let _ = fs::write(&mark, b"");
    let Ok(dir) = fs::read_dir(&paths.out) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = dir
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("hr-"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    files.sort_by_key(|f| Reverse(f.0));
    let mut total = 0;
    for (when, size, path) in files {
        total += size;
        if total > KEEP_BYTES || when.elapsed().is_ok_and(|age| age > KEEP_FOR) {
            let _ = fs::remove_file(path);
        }
    }
}

fn fnv(parts: &[&str]) -> u64 {
    let mut h = FNV_START;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            h = (h ^ 0xff).wrapping_mul(FNV_PRIME);
        }
        for b in part.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(FNV_PRIME);
        }
    }
    h
}

fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs::File;

    const SLOW_CI: Duration = Duration::from_secs(20);

    fn scratch(name: &str) -> Paths {
        let root = env::temp_dir().join(format!("lt-hook-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        Paths::new(&root.join(".lighting"))
    }

    fn session(holdout: bool) -> String {
        (0..)
            .map(|i| format!("s{i}"))
            .find(|s| fnv(&[s.as_str()]).is_multiple_of(HOLDOUT_EVERY) == holdout)
            .expect("session id")
    }

    fn bash_event(sid: &str, cmd: &str, stdout: &str) -> Vec<u8> {
        json!({
            "session_id": sid, "tool_name": "Bash", "tool_use_id": "toolu_1", "tool_input": {"command": cmd},
            "tool_response": {"stdout": stdout, "stderr": "warn: x", "interrupted": false, "isImage": false}
        })
        .to_string()
        .into_bytes()
    }

    fn big_log() -> String {
        let mut lines: Vec<String> = (0..400).map(|i| format!("step {i} done in {} ms", i % 17)).collect();
        lines.insert(200, "ERROR upload failed: connection refused".into());
        lines.join("\n")
    }

    fn logged(paths: &Paths) -> Vec<Value> {
        fs::read_to_string(paths.state.join("log.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).expect("log line"))
            .collect()
    }

    #[test]
    fn a_long_log_is_squeezed_and_the_full_text_kept() {
        let paths = scratch("cut");
        let log = big_log();
        let (reply, kept) = run(&bash_event(&session(false), "npm test 2>&1", &log), &paths, Instant::now(), SLOW_CI);
        let reply: Value = serde_json::from_str(&reply.expect("reply")).expect("reply json");
        let out = &reply["hookSpecificOutput"];
        assert_eq!(out["hookEventName"], "PostToolUse");
        let updated = &out["updatedToolOutput"];
        let stdout = updated["stdout"].as_str().expect("stdout");
        assert!(stdout.starts_with("lighting squeezed 401 lines"), "{stdout}");
        assert!(stdout.contains("ERROR upload failed: connection refused"));
        assert!(stdout.chars().count() < log.chars().count());
        assert_eq!((updated["stderr"].as_str(), updated["interrupted"].as_bool()), (Some("warn: x"), Some(false)));
        let name = kept_ref(stdout).expect("reference in the header");
        assert_eq!(fs::read_to_string(paths.out.join(format!("{name}.txt"))).expect("full output"), log);
        assert!(kept);
        let entry = &logged(&paths)[0];
        assert_eq!((entry["act"].as_str(), entry["ref"].as_str()), (Some("cut"), Some(name.as_str())));
        assert_eq!(entry["in"].as_u64(), Some(log.chars().count() as u64));
        assert_eq!(entry["kind"], "log");
    }

    #[test]
    fn holdout_sessions_get_the_full_output_and_are_measured() {
        let paths = scratch("holdout");
        let (reply, kept) = run(&bash_event(&session(true), "npm test", &big_log()), &paths, Instant::now(), SLOW_CI);
        assert!(reply.is_none() && !kept);
        assert!(!paths.out.exists());
        let entry = &logged(&paths)[0];
        assert_eq!(entry["act"], "holdout");
        assert!(entry["out"].as_u64() < entry["in"].as_u64());
    }

    #[test]
    fn exact_reads_listings_and_own_commands_stay_whole() {
        for cmd in [
            "cat src/main.rs",
            "cd /e/x && rtk head -50 crates/a.rs",
            "rtk git diff",
            "git -C /e/Lighting --no-pager show HEAD:a.rs",
            "diff a b",
            "echo x; sed -n '1,80p' SPEC.md",
            "for f in *.rs; do cat $f; done",
            "find . -name '*.java'",
            "ls -la",
            "rtk git log --oneline -50",
            "grep -rl foo src",
            "echo hi\ncat notes.md",
        ] {
            assert_eq!(untouchable(cmd), Some("exact"), "{cmd}");
        }
        for cmd in ["lighting snap", "rtk lighting text -f x", "\"E:/Lighting/bin/lighting.exe\" recall x"] {
            assert_eq!(untouchable(cmd), Some("own"), "{cmd}");
        }
        for cmd in [
            "npm test",
            "tail -100 server.log",
            "cargo build 2>&1 | tail -80",
            "cat a.log && grep x b.log",
            "python x.py",
        ] {
            assert_eq!(untouchable(cmd), None, "{cmd}");
        }
        let paths = scratch("exact");
        let (reply, _) = run(&bash_event(&session(false), "cat big.rs", &big_log()), &paths, Instant::now(), SLOW_CI);
        assert!(reply.is_none());
        assert_eq!(logged(&paths)[0]["act"], "exact");
    }

    #[test]
    fn small_and_image_outputs_are_left_alone() {
        let paths = scratch("small");
        let (reply, _) = run(&bash_event("s", "npm test", &"x\n".repeat(900)), &paths, Instant::now(), SLOW_CI);
        assert!(reply.is_none());
        let image = json!({"tool_name": "Bash", "tool_input": {"command": "x"}, "tool_response": {"stdout": big_log(), "isImage": true}});
        assert!(run(image.to_string().as_bytes(), &paths, Instant::now(), SLOW_CI).0.is_none());
        let acts: Vec<Value> = logged(&paths).iter().map(|e| e["act"].clone()).collect();
        assert_eq!(acts, [json!("small"), json!("image")]);
    }

    #[test]
    fn reading_a_kept_output_counts_as_a_reload() {
        let paths = scratch("reload");
        let kept = "C:\\Users\\x\\.lighting\\headroom\\out\\hr-0123456789.txt";
        let read = json!({"tool_name": "Read", "session_id": "s", "tool_input": {"file_path": kept}});
        assert!(run(read.to_string().as_bytes(), &paths, Instant::now(), SLOW_CI).0.is_none());
        let grep = bash_event("s", "grep -n ERROR ~/.lighting/headroom/out/hr-abcdefabcd.txt", &big_log());
        assert!(run(&grep, &paths, Instant::now(), SLOW_CI).0.is_none());
        let refs: Vec<(Value, Value)> = logged(&paths).iter().map(|e| (e["act"].clone(), e["ref"].clone())).collect();
        assert_eq!(refs, [(json!("reload"), json!("hr-0123456789")), (json!("reload"), json!("hr-abcdefabcd"))]);
        let other = json!({"tool_name": "Read", "tool_input": {"file_path": "src/main.rs"}});
        assert!(run(other.to_string().as_bytes(), &paths, Instant::now(), SLOW_CI).0.is_none());
        assert_eq!(logged(&paths).len(), 2);
    }

    #[test]
    fn three_errors_in_ten_minutes_turn_it_off_once() {
        let paths = scratch("errors");
        assert!(run(b"not json", &paths, Instant::now(), SLOW_CI).0.is_none());
        assert!(run(b"{", &paths, Instant::now(), SLOW_CI).0.is_none());
        assert!(!paths.state.join("off").exists());
        let reply: Value =
            serde_json::from_str(&run(b"", &paths, Instant::now(), SLOW_CI).0.expect("message")).expect("json");
        assert!(reply["systemMessage"].as_str().is_some_and(|m| m.contains("3 errors in 10 minutes")));
        assert!(paths.state.join("off").exists());
    }

    #[test]
    fn tidy_drops_old_and_oversized_kept_outputs() {
        let paths = scratch("tidy");
        fs::create_dir_all(&paths.out).unwrap();
        let old = paths.out.join("hr-old0000000.txt");
        let fresh = paths.out.join("hr-new0000000.txt");
        let foreign = paths.out.join("153012-001-snap.txt");
        for f in [&old, &fresh, &foreign] {
            fs::write(f, "x").unwrap();
        }
        let age = |f: &Path| {
            let long_ago = SystemTime::now() - Duration::from_secs(8 * 86_400);
            File::options().write(true).open(f).unwrap().set_modified(long_ago).unwrap();
        };
        age(&old);
        age(&foreign);
        tidy(&paths);
        assert!(!old.exists() && fresh.exists() && foreign.exists());
        fs::write(&old, "x").unwrap();
        age(&old);
        tidy(&paths);
        assert!(old.exists(), "tidies at most once an hour");
    }
}
