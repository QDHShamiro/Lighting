#[cfg(not(target_os = "macos"))]
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use lt_core::Fail;

use super::super::hosts::Host;
use super::super::{Daemon, config};
use super::secs;

const BROWSER_START_WAIT: f64 = 20.0;
const RUNNING_WAIT: f64 = 6.0;
const RECONNECT_GRACE: f64 = 2.5;

#[cfg(windows)]
const ORDER: &[&str] = &["brave", "chrome", "edge"];
#[cfg(not(windows))]
const ORDER: &[&str] = &["brave", "chrome", "edge", "chromium"];
#[cfg(windows)]
const EXES: &[(&str, &[&str])] = &[
    ("brave", &[r"BraveSoftware\Brave-Browser\Application\brave.exe"]),
    ("chrome", &[r"Google\Chrome\Application\chrome.exe"]),
    ("edge", &[r"Microsoft\Edge\Application\msedge.exe"]),
];
#[cfg(target_os = "macos")]
const EXES: &[(&str, &[&str])] = &[
    ("brave", &["/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"]),
    ("chrome", &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]),
    ("edge", &["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"]),
    ("chromium", &["/Applications/Chromium.app/Contents/MacOS/Chromium"]),
];
#[cfg(all(unix, not(target_os = "macos")))]
const EXES: &[(&str, &[&str])] = &[
    ("brave", &["brave-browser", "brave"]),
    ("chrome", &["google-chrome", "google-chrome-stable"]),
    ("edge", &["microsoft-edge", "microsoft-edge-stable"]),
    ("chromium", &["chromium", "chromium-browser"]),
];

fn browser_pref(d: &Daemon) -> String {
    let pref = config::text(&config::load(&d.home), "browser").to_lowercase();
    if pref.is_empty() { "auto".into() } else { pref }
}

pub fn pick(d: &Daemon) -> Option<Arc<Host>> {
    d.hosts.pick(&browser_pref(d), true).ok().flatten()
}

pub fn any_browser(d: &Daemon) -> bool {
    let end = d.started + secs(RECONNECT_GRACE);
    loop {
        if !d.hosts.alive().is_empty() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn ensure_host(d: &Daemon) -> Result<Arc<Host>, Fail> {
    let pref = browser_pref(d);
    if let Some(host) = d.hosts.pick(&pref, false)? {
        return Ok(host);
    }
    let order: Vec<&str> = if ORDER.contains(&pref.as_str()) { vec![pref.as_str()] } else { ORDER.to_vec() };
    for name in order {
        let Some(exe) = browser_exe(name) else { continue };
        if running(&exe) {
            return wait_host(d, &pref, RUNNING_WAIT).ok_or_else(|| {
                Fail::hint(
                    format!("{name} is open but the Lighting extension is not connected"),
                    "lighting setup (loads the extension), then lighting status",
                )
            });
        }
        start_browser(d, &exe);
        return wait_host(d, &pref, BROWSER_START_WAIT).ok_or_else(|| {
            Fail::hint(format!("started {name} but the Lighting extension did not connect"), "lighting setup")
        });
    }
    Err(Fail::hint("no browser with the Lighting extension is connected", "lighting setup"))
}

fn wait_host(d: &Daemon, pref: &str, wait: f64) -> Option<Arc<Host>> {
    let end = Instant::now() + secs(wait);
    while Instant::now() < end {
        if let Some(host) = d.hosts.pick(pref, true).ok().flatten() {
            return Some(host);
        }
        thread::sleep(Duration::from_millis(100));
    }
    None
}

fn exes(name: &str) -> &'static [&'static str] {
    EXES.iter().find(|(n, _)| *n == name).map_or(&[][..], |(_, rels)| *rels)
}

#[cfg(windows)]
pub fn browser_exe(name: &str) -> Option<PathBuf> {
    let base = |key: &str, default: &str| env::var(key).unwrap_or_else(|_| default.to_string());
    let bases = [
        base("PROGRAMFILES", r"C:\Program Files"),
        base("PROGRAMFILES(X86)", r"C:\Program Files (x86)"),
        base("LOCALAPPDATA", ""),
    ];
    exes(name)
        .iter()
        .flat_map(|rel| bases.iter().filter(|b| !b.is_empty()).map(move |b| Path::new(b).join(rel)))
        .find(|p| p.exists())
}

#[cfg(target_os = "macos")]
pub fn browser_exe(name: &str) -> Option<PathBuf> {
    exes(name).iter().map(PathBuf::from).find(|p| p.exists())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn browser_exe(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    exes(name).iter().find_map(|exe| env::split_paths(&path).map(|dir| dir.join(exe)).find(|p| p.is_file()))
}

pub fn installed_browsers() -> Vec<&'static str> {
    ORDER.iter().copied().filter(|n| browser_exe(n).is_some()).collect()
}

fn base_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

#[cfg(windows)]
fn running(exe: &Path) -> bool {
    let name = base_name(exe);
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", &format!("IMAGENAME eq {name}"), "/NH"]).stdin(Stdio::null()).stderr(Stdio::null());
    crate::v2::hide(&mut cmd);
    cmd.output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains(&name))
}

#[cfg(not(windows))]
fn running(exe: &Path) -> bool {
    let want = base_name(exe);
    let Ok(out) = Command::new("ps").args(["-axo", "command="]).stdin(Stdio::null()).stderr(Stdio::null()).output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        let first = line.trim().split(" --").next().unwrap_or("");
        base_name(Path::new(first)) == want
    })
}

fn start_browser(d: &Daemon, exe: &Path) {
    let log = d.home.join("daemon.log");
    let dir = exe.parent().unwrap_or(d.home.as_path());
    if cfg!(target_os = "macos") {
        let text = exe.to_string_lossy();
        let app = text.split_once(".app/").map_or(text.to_string(), |(base, _)| format!("{base}.app"));
        crate::spawn::detached(Path::new("/usr/bin/open"), &["-a", &app], &d.home, &log);
    } else {
        crate::spawn::detached(exe, &[], dir, &log);
    }
}
