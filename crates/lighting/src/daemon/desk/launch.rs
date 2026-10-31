use std::collections::HashSet;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use lt_core::{Call, Fail};
use regex::Regex;
use serde_json::{Value, json};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW,
};
use windows::core::HSTRING;

use super::super::config::NAV_LINES;
use super::super::ctx::{Ctx, now};
use super::super::web::keys;
use super::super::words::tiers;
use super::snap::Look;
use super::state::State;
use super::view::hint_words;
use super::win::{self, Win};
use super::{Desk, Res};

const NAV_CHARS: usize = 1600;
const APPS_DAY_S: u64 = 86_400;
const FRESH_GAP_S: f64 = 5.0;
const NO_WINDOW: u32 = 0x0800_0000;
const RUNNABLE: &[&str] =
    &[".exe", ".bat", ".cmd", ".com", ".ps1", ".vbs", ".vbe", ".js", ".jse", ".wsf", ".msi", ".scr", ".lnk", ".url"];
const RISKY_SCHEMES: &[&str] = &[
    "file",
    "ms-msdt",
    "search-ms",
    "search",
    "ms-officecmd",
    "ms-word",
    "ms-excel",
    "ms-powerpoint",
    "ms-visio",
    "ms-access",
    "ms-appinstaller",
    "ms-cxh",
    "ms-cxh-full",
    "hcp",
    "javascript",
    "vbscript",
];
const START_APPS: &str = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress";

type App = (String, String);

static APPS: Mutex<(f64, Vec<App>)> = Mutex::new((0.0, Vec::new()));

fn app_list() -> Vec<App> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", START_APPS])
        .creation_flags(NO_WINDOW)
        .output();
    let Ok(out) = out else { return Vec::new() };
    let data: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let list = match data {
        Value::Array(a) => a,
        Value::Object(_) => vec![data],
        _ => Vec::new(),
    };
    list.iter()
        .filter_map(|a| {
            let id = a.get("AppID").and_then(Value::as_str).filter(|s| !s.is_empty())?;
            Some((a.get("Name").and_then(Value::as_str).unwrap_or("").to_string(), id.to_string()))
        })
        .collect()
}

fn start_apps(home: &Path, fresh: bool) -> Vec<App> {
    let mut apps = APPS.lock().unwrap_or_else(|e| e.into_inner());
    let cache = home.join("apps.json");
    if !fresh && apps.1.is_empty() {
        let age =
            fs::metadata(&cache).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
        if age.is_some_and(|a| a.as_secs() < APPS_DAY_S)
            && let Ok(text) = fs::read_to_string(&cache)
            && let Ok(list) = serde_json::from_str::<Vec<(String, String)>>(&text)
        {
            *apps = (now() - age.map_or(0.0, |a| a.as_secs_f64()), list);
        }
    }
    if fresh || apps.1.is_empty() {
        *apps = (now(), app_list());
        let _ = fs::write(&cache, json!(apps.1).to_string());
    }
    apps.1.clone()
}

fn find_app(home: &Path, name: &str) -> Option<App> {
    let n = name.to_lowercase();
    let rank = |entry: &App| -> u8 {
        let t = entry.0.to_lowercase();
        if t == n { 5 } else { tiers(std::slice::from_ref(&n), &[t.as_str()])[0] + u8::from(t.starts_with(&n)) }
    };
    for fresh in [false, true] {
        let refreshed = APPS.lock().map(|a| a.0).unwrap_or(0.0);
        if fresh && now() - refreshed < FRESH_GAP_S {
            break;
        }
        let best = start_apps(home, fresh).into_iter().max_by_key(|a| (rank(a), std::cmp::Reverse(a.0.len())));
        if let Some(best) = best.filter(|a| rank(a) > 0) {
            return Some(best);
        }
    }
    None
}

fn registered(exe: &str) -> bool {
    let path = HSTRING::from(format!("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{exe}"));
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE].into_iter().any(|hive| {
        let mut key = HKEY::default();
        let found = unsafe { RegOpenKeyExW(hive, &path, None, KEY_READ, &raw mut key) }.is_ok();
        if found {
            let _ = unsafe { RegCloseKey(key) };
        }
        found
    })
}

fn run_name(name: &str) -> Option<String> {
    let plain = Regex::new(r"^[\w.-]+$").ok()?;
    if !plain.is_match(name) {
        return None;
    }
    let exe = if name.to_lowercase().ends_with(".exe") { name.to_string() } else { format!("{name}.exe") };
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let root = Path::new(&root);
    if [root.join("System32"), root.to_path_buf()].iter().any(|d| d.join(&exe).is_file()) || registered(&exe) {
        return Some(exe);
    }
    None
}

fn owner_of<'a>(wins: &'a [Win], hint: &str) -> Option<&'a Win> {
    let words = hint_words(hint);
    let mut ordered: Vec<&Win> = wins.iter().collect();
    ordered.sort_by_key(|w| !w.fg);
    let exe_hit = ordered.iter().find(|w| words.contains(&w.exe.to_lowercase().trim_end_matches(".exe").to_string()));
    exe_hit.or_else(|| ordered.iter().find(|w| words.iter().any(|word| w.title.to_lowercase().contains(word)))).copied()
}

fn wait_window(before: &HashSet<isize>, fg0: isize, ms: u64, hint: &str, running: bool) -> Option<isize> {
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_millis(ms);
    let mut pick: Option<(isize, Instant)> = None;
    while Instant::now() < deadline {
        let wins = win::windows();
        let new: Vec<&Win> = wins.iter().filter(|w| !before.contains(&w.hwnd)).collect();
        let fg = wins.iter().find(|w| w.fg);
        let mut cand = new.iter().find(|w| w.fg).or(new.last()).map(|w| w.hwnd);
        if cand.is_none() {
            cand = fg.filter(|w| w.hwnd != fg0).map(|w| w.hwnd);
        }
        if cand.is_none() && t0.elapsed().as_secs_f64() > if running { 0.5 } else { 1.5 } {
            cand = owner_of(&wins, hint).map(|w| w.hwnd);
        }
        if let Some(c) = cand {
            match pick {
                Some((p, at)) if p == c => {
                    if at.elapsed().as_secs_f64() > if running { 0.3 } else { 0.6 } {
                        return Some(c);
                    }
                }
                _ => pick = Some((c, Instant::now())),
            }
        }
        thread::sleep(Duration::from_millis(150));
    }
    pick.map(|(p, _)| p)
}

impl Desk<'_> {
    fn first_look(&mut self, hwnd: isize, c: &Call) -> String {
        let look = Look { f: c.val("f").filter(|f| !f.is_empty()).map(String::from), ..Look::default() };
        let Ok(text) = self.snapshot(hwnd, &look) else { return self.header(hwnd, "") };
        let rows: Vec<&str> = text.split('\n').collect();
        let mut out: Vec<String> = vec![rows[0].to_string()];
        let mut chars = 0;
        for r in &rows[1..] {
            if out.len() > NAV_LINES || chars + r.len() > NAV_CHARS {
                out.push(format!("... +{} more (wisply snap {})", rows.len() - out.len(), self.wref(hwnd)));
                break;
            }
            out.push((*r).to_string());
            chars += r.len() + 1;
        }
        out[0] += &keys::hint(&self.d.home, &win::exe_of_window(hwnd).to_lowercase());
        out.join("\n")
    }
}

pub fn launch(x: &mut Desk, c: &Call) -> Res<String> {
    if c.pos.is_empty() {
        return Err(Fail::hint(
            "launch needs an app name, URI or path",
            "wisply launch spotify | launch \"spotify:search:SOS\"",
        )
        .into());
    }
    let spec = c.pos.join(" ");
    let path = spec.trim_matches('"').to_string();
    let uri = Regex::new(r"(?i)^[a-z][a-z0-9+.-]+:").is_ok_and(|r| r.is_match(&spec));
    let wins0 = win::windows();
    let before: HashSet<isize> = wins0.iter().map(|w| w.hwnd).collect();
    let fg0 = win::foreground();
    let (what, hint) = if Path::new(&path).exists() {
        let low = path.to_lowercase();
        if RUNNABLE.iter().any(|r| low.ends_with(r)) && !c.on("yes") {
            let base = Path::new(&path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            return Err(Fail::hint(
                format!("{base} runs a program"),
                format!("wisply launch \"{path}\" --yes (only if the user asked for it)"),
            )
            .into());
        }
        win::open_target(&path);
        let stem = Path::new(&path).file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        (stem.clone(), stem)
    } else if uri {
        let scheme = spec.split(':').next().unwrap_or("").to_lowercase();
        if RISKY_SCHEMES.contains(&scheme.as_str()) && !c.on("yes") {
            let short: String = spec.chars().take(60).collect();
            return Err(Fail::hint(
                format!("{scheme}: links can run programs or open remote files"),
                format!("only if the user asked for it: wisply launch \"{short}\" --yes"),
            )
            .into());
        }
        win::open_target(&spec);
        (spec.clone(), scheme)
    } else {
        let app = find_app(&x.d.home, &spec);
        let exact = app.as_ref().is_some_and(|a| a.0.eq_ignore_ascii_case(&spec));
        match (if exact { None } else { run_name(&spec) }, app) {
            (Some(exe), _) => {
                win::open_target(&exe);
                (spec.clone(), spec.clone())
            }
            (None, Some(app)) => {
                win::open_target(&format!("shell:AppsFolder\\{}", app.1));
                (app.0.clone(), app.0)
            }
            (None, None) => {
                return Err(Fail::hint(
                    format!("no app named \"{spec}\" in the start menu"),
                    "wisply launch <name as in the start menu>, a URI or a path",
                )
                .into());
            }
        }
    };
    let ms = c.val("timeout").and_then(|t| t.parse().ok()).unwrap_or(10_000);
    let running = owner_of(&wins0, &hint).is_some();
    let Some(hwnd) = wait_window(&before, fg0, ms, &hint, running) else {
        return Ok(format!("ok (started {what}, no window yet) -> try: wisply windows"));
    };
    if x.bg(hwnd) && fg0 != 0 && win::alive(fg0) && win::foreground() != fg0 {
        win::put_behind(hwnd, fg0);
        win::set_foreground(fg0);
    }
    if !before.contains(&hwnd) {
        if x.st.launched.is_empty() {
            x.st.front0 = fg0;
        }
        let title = win::text_of(hwnd);
        x.st.launched.insert(hwnd, if title.is_empty() { what.clone() } else { title });
    }
    x.st.hwnd = hwnd;
    x.ctx.target = Some(("app".into(), hwnd.to_string()));
    Ok(format!("ok ({what}) -> {}", x.first_look(hwnd, c)))
}

pub fn cleanup(st: &mut State, ctx: &mut Ctx) -> (Vec<String>, Vec<String>) {
    let wins: Vec<(isize, String)> =
        st.launched.iter().filter(|(h, _)| win::alive(**h)).map(|(h, n)| (*h, n.clone())).collect();
    let front0 = st.front0;
    st.launched.clear();
    st.front0 = 0;
    let fg = win::foreground();
    for (h, _) in &wins {
        win::close(*h);
    }
    let deadline = Instant::now() + Duration::from_millis(1500);
    let open = |h: isize| win::alive(h) && win::visible(h);
    while Instant::now() < deadline && wins.iter().any(|(h, _)| open(*h)) {
        thread::sleep(Duration::from_millis(100));
    }
    let (left, closed): (Vec<_>, Vec<_>) = wins.iter().partition(|(h, _)| open(*h));
    if closed.iter().any(|(h, _)| *h == st.hwnd) {
        st.hwnd = 0;
        if ctx.kind() == Some("app") {
            ctx.target = None;
        }
    }
    let launched_front = wins.iter().any(|(h, _)| *h == fg);
    if !closed.is_empty() && front0 != 0 && win::alive(front0) && launched_front && !win::alive(fg) {
        win::set_foreground(front0);
    }
    (closed.into_iter().map(|(_, n)| n.clone()).collect(), left.into_iter().map(|(_, n)| n.clone()).collect())
}
