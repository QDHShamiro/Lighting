mod audio;
mod brain;
mod discord;
mod ears;
mod mouth;
mod run;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use lt_core::{Fail, local_parts};
use regex::Regex;
use serde_json::{Map, Value, json};
use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
};

const NEEDED: &[&str] = &["listen_device", "discord_out"];
const KEYS: &[&str] = &["GROQ_API_KEY", "GEMINI_API_KEY"];
const LOG_TAIL: usize = 6;
const NEUTRAL: &str = "Du bist eine freundliche KI in einem Discord-Call.";
const RULES: &str = include_str!("rules.txt");
const OWNER: &str = include_str!("owner.txt");

static SLUG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("slug"));
static STATE_LOCK: Mutex<()> = Mutex::new(());

pub fn defaults() -> Map<String, Value> {
    let v = json!({
        "voice": "de-DE-FlorianMultilingualNeural",
        "rate": "-3%",
        "models": "groq:openai/gpt-oss-120b,gemini-2.5-flash,gemini-2.5-flash-lite,groq:openai/gpt-oss-20b,groq:qwen/qwen3.8-27b",
        "rpm": "groq:openai/gpt-oss-120b=10,gemini-2.5-flash=8,gemini-2.5-flash-lite=12,groq:openai/gpt-oss-20b=12,groq:qwen/qwen3.8-27b=10",
        "search_model": "openai/gpt-oss-120b",
        "search_filler": "Moment, ich schau kurz nach.",
        "listen_device": "",
        "speak_device": "CABLE Input",
        "monitor_device": "",
        "owner_mic": "",
        "home_out": "",
        "owner_name": "",
        "discord_mic": "CABLE Output",
        "discord_out": "",
        "greeting": "",
        "disclosure": "Hi, hier spricht eine KI. Ich höre mit, damit ich antworten kann, und speichere nichts davon. Sagt KI raus, dann gehe ich sofort.",
        "bye": "Okay, bin raus.",
        "stop": r"\b(ki|kai|bot)\W+(?:\w+\W+){0,2}?(raus|stopp?|verschwinde|leave)\b",
        "names": "Discord, KI",
        "wake": r"\b(ki|kai|k\.i\.|bot|lighting)\b",
        "silence_s": 20,
        "level": 0.012,
        "end_pause_s": 0.5,
        "settle_s": 0.35,
        "open_settle_s": 1.2,
        "barge_in_s": 0.8,
        "max_utterance_s": 15,
        "min_speech_s": 0.6,
        "local_asr": "base",
        "history": 16,
        "min_words": 4,
        "auto_answer": true,
        "aliases": {}
    });
    v.as_object().cloned().unwrap_or_default()
}

#[derive(Clone)]
pub struct Paths {
    pub dir: PathBuf,
    pub home: PathBuf,
}

impl Paths {
    pub fn new(home: &Path) -> Paths {
        Paths { dir: home.join("voice"), home: home.to_path_buf() }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn say_dir(&self) -> PathBuf {
        self.file("say")
    }

    pub fn config(&self) -> Map<String, Value> {
        let mut cfg = defaults();
        if let Some(saved) = fs::read_to_string(self.file("config.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
        {
            cfg.extend(saved);
        }
        cfg
    }

    fn save_config(&self, cfg: &Map<String, Value>) -> Result<(), Fail> {
        let base = defaults();
        let keep: Map<String, Value> =
            cfg.iter().filter(|(k, v)| base.get(*k) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect();
        let _ = fs::create_dir_all(&self.dir);
        fs::write(self.file("config.json"), serde_json::to_string_pretty(&Value::Object(keep)).unwrap_or_default())
            .map_err(|e| Fail::new(format!("could not save the voice settings: {e}")))
    }

    pub fn log(&self, line: &str) {
        let _ = fs::create_dir_all(&self.dir);
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        let [_, _, _, h, m, s] = local_parts(secs);
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(self.file("voice.log")) {
            let _ = writeln!(f, "{h:02}:{m:02}:{s:02} {line}");
        }
    }

    pub fn current(&self) -> Map<String, Value> {
        fs::read_to_string(self.file("state.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    pub fn state(&self, kv: Value) -> Map<String, Value> {
        let _guard = STATE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let mut cur = self.current();
        if let Some(obj) = kv.as_object() {
            cur.extend(obj.clone());
        }
        let _ = fs::create_dir_all(&self.dir);
        let _ = fs::write(self.file("state.json"), Value::Object(cur.clone()).to_string());
        cur
    }

    pub fn persona_file(&self, target: &str) -> PathBuf {
        let slug = SLUG.replace_all(&target.to_lowercase(), "-").trim_matches('-').to_string();
        self.dir.join("personas").join(format!("{}.md", if slug.is_empty() { "open-chat" } else { &slug }))
    }

    pub fn drop_line(&self, text: &str) {
        let dir = self.say_dir();
        let _ = fs::create_dir_all(&dir);
        let ns = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let _ = fs::write(dir.join(format!("{ns}.txt")), text);
    }
}

pub fn text(cfg: &Map<String, Value>, k: &str) -> String {
    match cfg.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

pub fn num(cfg: &Map<String, Value>, k: &str) -> f64 {
    cfg.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0)
}

pub fn rules(cfg: &Map<String, Value>) -> String {
    let owner = text(cfg, "owner_name");
    let names = if owner.is_empty() { String::new() } else { format!(", {owner}") };
    let mut out = RULES.replace("{names}", &names);
    if !owner.is_empty() {
        out += &OWNER.replace("{o}", &owner);
    }
    out
}

pub fn disclosure(cfg: &Map<String, Value>) -> String {
    Some(text(cfg, "disclosure").trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| text(&defaults(), "disclosure"))
}

pub fn neutral() -> &'static str {
    NEUTRAL
}

pub fn keys() -> Map<String, Value> {
    KEYS.iter().filter_map(|k| crate::daemon::api_key(k).map(|v| ((*k).to_string(), json!(v)))).collect()
}

pub fn alive(pid: u64) -> bool {
    let Ok(pid) = u32::try_from(pid) else { return false };
    if pid == 0 {
        return false;
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &raw mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(h);
        ok
    }
}

pub fn kill(pid: u64) {
    let Ok(pid) = u32::try_from(pid) else { return };
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(h, 1);
            let _ = CloseHandle(h);
        }
    }
}

fn spawn(p: &Paths, args: &[&str]) -> Result<u32, Fail> {
    let exe = std::env::current_exe().map_err(|e| Fail::new(e.to_string()))?;
    let _ = fs::create_dir_all(&p.dir);
    let before: Vec<u64> = vec![p.current().get("session_pid").and_then(Value::as_u64).unwrap_or(0)];
    if !crate::spawn::detached(&exe, args, &p.home, &p.file("stream.out")) {
        return Err(Fail::new("could not start the voice process"));
    }
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let key = if args.first() == Some(&"voice-watch") { "watch_pid" } else { "session_pid" };
        if let Some(pid) = p.current().get(key).and_then(Value::as_u64).filter(|pid| !before.contains(pid)) {
            return Ok(u32::try_from(pid).unwrap_or(0));
        }
    }
    Ok(0)
}

fn coerce(default: &Value, raw: &str) -> Option<Value> {
    match default {
        Value::Bool(_) => Some(json!(matches!(raw.to_lowercase().as_str(), "1" | "on" | "true" | "ja"))),
        Value::Number(n) if n.is_i64() => raw.parse::<i64>().ok().map(|v| json!(v)),
        Value::Number(_) => raw.parse::<f64>().ok().map(|v| json!(v)),
        Value::String(_) => Some(json!(raw)),
        _ => None,
    }
}

pub fn command(home: &Path, pos: &[String]) -> Result<String, Fail> {
    let p = Paths::new(home);
    let sub = pos.first().map_or("status", String::as_str);
    let cur = p.current();
    let pid_of = |k: &str| cur.get(k).and_then(Value::as_u64).unwrap_or(0);
    let running = alive(pid_of("pid")) && cur.get("status").and_then(Value::as_str) == Some("running");
    let target_of =
        || cur.get("target").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("the open chat").to_string();
    match sub {
        "join" | "here" => {
            if running {
                return Err(Fail::hint(format!("already in a call ({})", target_of()), "lighting voice leave"));
            }
            let have = keys();
            let missing: Vec<&str> = KEYS.iter().copied().filter(|k| !have.contains_key(*k)).collect();
            if !missing.is_empty() {
                return Err(Fail::hint(format!("missing {}", missing.join(", ")), "add it to ~/.config/watch/.env"));
            }
            let cfg = p.config();
            let unset: Vec<&str> = NEEDED.iter().copied().filter(|k| text(&cfg, k).is_empty()).collect();
            if !unset.is_empty() {
                return Err(Fail::hint(
                    format!("voice needs your audio devices first: {}", unset.join(", ")),
                    "lighting voice set listen_device=<what Discord plays to> discord_out=<the same device>",
                ));
            }
            mouth::tts_exe(home)?;
            let target = if sub == "join" { pos[1..].join(" ") } else { String::new() };
            if sub == "join" && target.is_empty() {
                return Err(Fail::hint("join needs a chat name", "lighting voice join \"Friends\""));
            }
            p.state(json!({"status": "starting", "target": target}));
            let pid = spawn(&p, &["voice-session", &target])?;
            let shown = if target.is_empty() { "the open chat".to_string() } else { target };
            Ok(format!("voice: joining {shown} (pid {pid}), check with: lighting voice status"))
        }
        "leave" => {
            if !running {
                return Ok("voice: not in a call".into());
            }
            p.drop_line("__leave__");
            let prev = cur.get("prev").cloned().unwrap_or(json!({}));
            let side = |k: &str| prev[k].as_str().unwrap_or("?").to_string();
            Ok(format!("voice: leaving, Discord goes back to {} / {}", side("mic"), side("out")))
        }
        "say" => {
            if !running {
                return Err(Fail::hint("not in a call", "lighting voice join \"Name\""));
            }
            p.drop_line(&pos[1..].join(" "));
            Ok("voice: queued".into())
        }
        "watch" => {
            let on = pos.get(1).map_or("on", String::as_str) != "off";
            if alive(pid_of("watch_pid")) {
                kill(pid_of("watch_pid"));
                p.state(json!({"watch_pid": 0}));
            }
            if !on {
                return Ok("voice: answering calls off".into());
            }
            Ok(format!("voice: answering calls on (pid {})", spawn(&p, &["voice-watch"])?))
        }
        "set" => {
            let base = defaults();
            let mut cfg = p.config();
            for kv in &pos[1..] {
                let (k, v) = kv.split_once('=').unwrap_or((kv.as_str(), ""));
                let Some(d) = base.get(k) else {
                    let all: Vec<&str> = base.keys().map(String::as_str).collect();
                    return Err(Fail::hint(format!("unknown setting '{k}'"), format!("settings: {}", all.join(", "))));
                };
                let value = coerce(d, v).ok_or_else(|| Fail::new(format!("'{v}' does not fit {k}")))?;
                cfg.insert(k.to_string(), value);
            }
            p.save_config(&cfg)?;
            Ok(format!("voice: saved {}", pos[1..].join(", ")))
        }
        "devices" => {
            let list = |kind: audio::Kind| audio::names(&kind).join("\n  ");
            Ok(format!(
                "speakers (listen_device, speak_device, monitor_device, home_out):\n  {}\nmicrophones (owner_mic):\n  {}",
                list(audio::Kind::Speaker),
                list(audio::Kind::Mic)
            ))
        }
        "persona" | "prompt" => {
            let call = pos[1..].join(" ");
            let path = p.persona_file(&call);
            Ok(format!(
                "voice persona for {}: {} ({}; it is used only in this call, other calls get a neutral one)",
                if call.is_empty() { "the open chat" } else { &call },
                path.display().to_string().replace('\\', "/"),
                if path.exists() { "exists" } else { "missing, write it there" }
            ))
        }
        _ => {
            let status = cur.get("status").and_then(Value::as_str).unwrap_or("");
            let head = if running {
                format!("in a call with {}", target_of())
            } else if status.starts_with("crash") {
                format!("not in a call ({status})")
            } else {
                "not in a call".into()
            };
            let watching = if alive(pid_of("watch_pid")) { ", answering calls" } else { "" };
            let mut lines = vec![format!("voice: {head}{watching}")];
            if running {
                let field =
                    |k: &str| cur.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("-").to_string();
                lines.push(format!("heard: {}", field("last_heard")));
                lines.push(format!("said: {}", field("last_said")));
            }
            if let Ok(raw) = fs::read(p.file("voice.log")) {
                let log = String::from_utf8_lossy(&raw);
                let all: Vec<&str> = log.lines().collect();
                lines.push("log:".into());
                lines.extend(all[all.len().saturating_sub(LOG_TAIL)..].iter().map(|l| (*l).to_string()));
            }
            Ok(lines.join("\n"))
        }
    }
}

pub fn main(args: &[String]) -> ExitCode {
    let p = Paths::new(&lt_proto::home());
    let done = match args.first().map(String::as_str) {
        Some("voice-watch") => discord::watch(&p),
        _ => discord::session(&p, args.get(1).map_or("", String::as_str), None),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            p.log(&format!("crashed: {e}"));
            p.state(json!({"status": format!("crashed: {}", e.chars().take(120).collect::<String>())}));
            ExitCode::FAILURE
        }
    }
}
