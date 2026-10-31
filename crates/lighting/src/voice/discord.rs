use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use regex::{Regex, RegexBuilder};
use serde_json::{Map, Value, json};

use super::{Paths, alive, kill, run, text};

const QUICK: Duration = Duration::from_secs(40);
const SNAP_WAIT: Duration = Duration::from_secs(60);
const WATCH_EVERY: Duration = Duration::from_secs(3);
const ERR_CHARS: usize = 160;
const LEAVE: &str = "^Verbindung trennen$";

static WINDOW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(w\d+) (.*) - Discord\.exe").expect("window"));
static ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*(d\d+) (\w+) "([^"]*)"(.*)"#).expect("row"));

#[derive(Clone)]
pub struct Row {
    reference: String,
    kind: String,
    name: String,
    on: bool,
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn wisply(args: &[&str], limit: Duration) -> String {
    let Ok(exe) = std::env::current_exe() else { return String::new() };
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    crate::daemon::run_tool(&exe, &args, limit).map(|r| r.out + &r.err).unwrap_or_default()
}

fn act(args: &[&str]) {
    wisply(args, QUICK);
}

fn discord() -> Result<String, String> {
    wisply(&["windows"], QUICK)
        .lines()
        .filter_map(|l| WINDOW.captures(l.trim()))
        .find(|m| !m[2].contains("Updater"))
        .map(|m| m[1].to_string())
        .ok_or_else(|| "Discord is not open".into())
}

fn controls(win: &str) -> String {
    wisply(&["snap", win, "--full"], SNAP_WAIT)
}

fn rows(snap: &str) -> Vec<Row> {
    snap.lines()
        .filter_map(|l| ROW.captures(l))
        .map(|m| Row { reference: m[1].into(), kind: m[2].into(), name: m[3].into(), on: m[4].contains('*') })
        .collect()
}

fn find(snap: &str, pattern: &str, kind: Option<&str>) -> Option<Row> {
    let re = RegexBuilder::new(pattern).case_insensitive(true).build().ok()?;
    rows(snap).into_iter().find(|r| re.is_match(&r.name) && kind.is_none_or(|k| r.kind == k))
}

fn title(snap: &str) -> String {
    snap.lines().next().unwrap_or("").to_lowercase()
}

fn escape(times: usize) {
    for _ in 0..times {
        act(&["press", "escape"]);
    }
}

fn pick(p: &Paths, win: &str, menu: &str, item: &str, name: &str) -> Result<String, String> {
    let mut opener = None;
    for _ in 0..3 {
        act(&["snap", win]);
        if find(&controls(win), "", Some("menuitem")).is_some() {
            escape(2);
            thread::sleep(ms(400));
        }
        act(&["click", menu]);
        for _ in 0..4 {
            thread::sleep(ms(600));
            opener = find(&controls(win), &format!("^{item}"), None);
            if opener.is_some() {
                break;
            }
        }
        if opener.is_some() {
            break;
        }
        p.log(&format!("menu '{item}' not open yet, again"));
        escape(1);
        thread::sleep(ms(500));
    }
    let opener = opener.ok_or_else(|| format!("Discord menu '{item}' not found"))?;
    act(&["click", &opener.reference]);
    let mut snap = String::new();
    for _ in 0..4 {
        thread::sleep(ms(600));
        snap = controls(win);
        if find(&snap, "", Some("radio")).is_some() {
            break;
        }
    }
    let before = rows(&snap).into_iter().find(|r| r.kind == "radio" && r.on).map(|r| r.name).unwrap_or_default();
    if !name.is_empty() {
        let quoted = regex::escape(name);
        let target = find(&snap, &format!("^{quoted}$"), Some("radio")).or_else(|| find(&snap, &quoted, Some("radio")));
        if let Some(t) = target.filter(|t| !t.on) {
            act(&["click", &t.reference]);
        }
    }
    escape(2);
    thread::sleep(ms(400));
    Ok(before)
}

const MENUS: [(&str, &str, &str); 2] =
    [("Eingabeoptionen", "Eingabegerät", "mic"), ("Ausgabeoptionen", "Ausgabegerät", "out")];

fn devices(p: &Paths, win: &str, mic: &str, out: &str) -> Result<Map<String, Value>, String> {
    let mut prev = Map::new();
    for ((menu, item, key), want) in MENUS.into_iter().zip([mic, out]) {
        prev.insert(key.into(), json!(pick(p, win, menu, item, want)?));
    }
    Ok(prev)
}

fn restore(p: &Paths, win: &str, prev: &Map<String, Value>) {
    for (menu, item, key) in MENUS {
        let want = text(prev, key);
        for _ in 0..3 {
            if want.is_empty() {
                break;
            }
            let _ = pick(p, win, menu, item, &want);
            if pick(p, win, menu, item, "").is_ok_and(|now| now == want) {
                break;
            }
            p.log(&format!("restore {key} did not stick, again"));
        }
    }
}

fn chat(p: &Paths, win: &str, target: &str) -> Result<(), String> {
    let bare = target.trim_start_matches('@');
    let aliases = p.config().get("aliases").and_then(Value::as_object).cloned().unwrap_or_default();
    let name = aliases.get(&bare.to_lowercase()).and_then(Value::as_str).unwrap_or(bare).to_string();
    let low = name.to_lowercase();
    if title(&controls(win)).contains(&low) {
        return Ok(());
    }
    for _ in 0..3 {
        act(&["snap", win]);
        let mut found = find(&controls(win), "Quick Switcher", None);
        if found.is_none() {
            act(&["press", "ctrl+k"]);
            thread::sleep(ms(800));
            found = find(&controls(win), "Quick Switcher", None);
        }
        let Some(field) = found else { continue };
        act(&["type", &field.reference, &name]);
        thread::sleep(ms(1500));
        act(&["press", "enter"]);
        thread::sleep(ms(1500));
        if title(&controls(win)).contains(&low) {
            return Ok(());
        }
        escape(1);
    }
    Err(format!("could not open the Discord chat '{name}'"))
}

fn call(p: &Paths, win: &str, target: &str) -> Result<bool, String> {
    let (server, channel) = target.split_once('/').unwrap_or((target, ""));
    let mut snap = controls(win);
    if find(&snap, LEAVE, None).is_some() {
        let connected = find(&snap, "Sprachchat verbunden|Sprachverbindung", None).is_some();
        let place = if connected { snap.lines().next().unwrap_or("?").to_string() } else { "?".into() };
        p.log(&format!("already in a call ({place}), staying there"));
        return Ok(false);
    }
    if !server.trim().is_empty() {
        chat(p, win, server.trim())?;
        snap = controls(win);
    }
    let channel = channel.trim();
    let button = if channel.is_empty() {
        find(&snap, "^(Sprachanruf beitreten|Anruf beitreten|Beitreten|Sprachanruf starten)$", Some("button"))
            .or_else(|| find(&snap, r"\(Sprachkanal\), \S", Some("button")))
            .ok_or("no call button or busy voice channel in this chat")?
    } else {
        find(&snap, &format!(r"^{} \(Sprachkanal\)", regex::escape(channel)), Some("button"))
            .ok_or_else(|| format!("voice channel '{channel}' not found on this server"))?
    };
    p.log(&format!("joining: {}", button.name));
    act(&["click", &button.reference]);
    for _ in 0..4 {
        thread::sleep(ms(1200));
        let snap = controls(win);
        if find(&snap, LEAVE, None).is_some() {
            return Ok(true);
        }
        if let Some(enter) = find(&snap, "^(Sprachchat beitreten|Sprachkanal beitreten|Beitreten)$", Some("button")) {
            act(&["click", &enter.reference]);
        }
    }
    Err(format!("clicked '{}' but Discord is not connected", button.name))
}

fn ringing(win: &str) -> Option<Row> {
    let snap = controls(win);
    find(&snap, "(eingehend|ruft an|incoming)", None)?;
    find(&snap, "^(Annehmen|Anruf annehmen|Beitreten)$", Some("button"))
}

fn hangup(win: &str) {
    if let Some(b) = find(&controls(win), LEAVE, None) {
        act(&["click", &b.reference]);
    }
}

fn stop_older(p: &Paths) {
    let me = u64::from(std::process::id());
    let old = p.current().get("session_pid").and_then(Value::as_u64).unwrap_or(0);
    if old != me && alive(old) {
        kill(old);
        p.log(&format!("stopped an older session (pid {old})"));
    }
    p.state(json!({"session_pid": me}));
}

fn join(
    p: &Paths,
    win: &str,
    target: &str,
    answered: Option<&Row>,
    prev: &mut Map<String, Value>,
) -> Result<bool, String> {
    let cfg = p.config();
    *prev = devices(p, win, &text(&cfg, "discord_mic"), &text(&cfg, "discord_out"))?;
    let out_re = RegexBuilder::new(&format!("cable|{}", regex::escape(&text(&cfg, "discord_out"))))
        .case_insensitive(true)
        .build()
        .map_err(|e| e.to_string())?;
    if out_re.is_match(&text(prev, "out")) {
        prev.insert("out".into(), json!(text(&cfg, "home_out")));
    }
    if text(prev, "mic").to_lowercase().contains("cable") {
        prev.insert("mic".into(), json!(text(&cfg, "owner_mic")));
    }
    p.state(json!({"prev": prev, "win": win}));
    if let Some(row) = answered {
        act(&["click", &row.reference]);
        return Ok(true);
    }
    call(p, win, target)
}

pub fn session(p: &Paths, target: &str, answered: Option<&Row>) -> Result<(), String> {
    stop_older(p);
    let cfg = p.config();
    let win = discord()?;
    let mut prev = Map::new();
    prev.insert("mic".into(), json!(text(&cfg, "owner_mic")));
    prev.insert("out".into(), json!(text(&cfg, "home_out")));
    let mut joined = answered.is_some();
    let done = join(p, &win, target, answered, &mut prev).and_then(|j| {
        joined = j;
        thread::sleep(ms(1500));
        run::call(p, target)
    });
    if joined && p.current().get("hangup").and_then(Value::as_bool) != Some(false) {
        hangup(&win);
    }
    restore(p, &win, &prev);
    p.state(json!({"status": "stopped"}));
    done
}

pub fn watch(p: &Paths) -> Result<(), String> {
    p.state(json!({"watch_pid": std::process::id()}));
    loop {
        match discord().map(|w| ringing(&w)) {
            Ok(Some(hit)) => {
                p.log("incoming call, answering");
                if let Err(e) = session(p, "", Some(&hit)) {
                    p.log(&format!("watch: {}", e.chars().take(ERR_CHARS).collect::<String>()));
                }
            }
            Ok(None) => {}
            Err(e) => p.log(&format!("watch: {}", e.chars().take(ERR_CHARS).collect::<String>())),
        }
        thread::sleep(WATCH_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_rows_and_windows_are_read_like_2x() {
        let snap = "Freunde - Discord\n  d3 button \"Verbindung trennen\"\n  d4 radio \"CABLE Output\" *\n  d5 radio \"Mikrofon (USB)\"\nnoise";
        let all = rows(snap);
        assert_eq!(all.len(), 3);
        assert!(all[1].on && !all[2].on && all[0].kind == "button");
        assert_eq!(find(snap, LEAVE, None).map(|r| r.reference).as_deref(), Some("d3"));
        assert_eq!(find(snap, "mikrofon", Some("radio")).map(|r| r.reference).as_deref(), Some("d5"));
        assert!(find(snap, "mikrofon", Some("button")).is_none());
        assert_eq!(title(snap), "freunde - discord");
        let w = WINDOW.captures("w2 Freunde - Discord - Discord.exe").map(|m| m[1].to_string());
        assert_eq!(w.as_deref(), Some("w2"));
    }
}
