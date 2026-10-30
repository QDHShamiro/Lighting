use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use lt_core::{Call, Fail, OUT_CHARS};
use regex::Regex;

mod tree;

use self::tree::{Target, escape_text, parse_devices, snap_of, tree_xml};
use super::ctx::Ctx;
use super::media::tools::{self, which};
use super::web::{SNAP_LINES, capped, save_picture};
use super::words::{best_only, terms, tiers};
use super::{Daemon, Out};

const ADB_S: u64 = 20;
const DUMP_S: u64 = 30;
const SETTLE: Duration = Duration::from_millis(600);
const TAP_SETTLE: Duration = Duration::from_millis(300);
const LONG_MS: &str = "700";
const SWIPE_MS: &str = "250";
const SAID_MAX: usize = 40;
const URL_SHOWN: usize = 80;
const APPS_LINES: usize = 80;
const ERR_MAX: usize = 200;
const DEFAULT_SIZE: (i32, i32) = (1080, 2400);
const TOOLS: &str = "platform-tools";
const PLUG: &str = "plug it in with USB debugging on (Settings > Developer options)";
const ALLOW: &str = "unlock it and tap Allow on the USB debugging prompt";
const HELP: &str = "lighting phone devices | snap [-f x] | tap p3|\"text\" | type \"text\" [--enter] | key back|home|enter | swipe up|down|left|right | shot | launch <app> | open <url> | apps [-f x] | setup";
const KEYS: &[(&str, u32)] = &[
    ("back", 4),
    ("home", 3),
    ("recent", 187),
    ("recents", 187),
    ("apps", 187),
    ("menu", 82),
    ("enter", 66),
    ("tab", 61),
    ("delete", 67),
    ("backspace", 67),
    ("del", 67),
    ("space", 62),
    ("power", 26),
    ("volup", 24),
    ("volumeup", 24),
    ("voldown", 25),
    ("volumedown", 25),
    ("mute", 164),
    ("playpause", 85),
    ("next", 87),
    ("nexttrack", 87),
    ("prev", 88),
    ("prevtrack", 88),
    ("up", 19),
    ("down", 20),
    ("left", 21),
    ("right", 22),
    ("search", 84),
    ("camera", 27),
    ("wake", 224),
    ("sleep", 223),
    ("esc", 111),
    ("escape", 111),
];

static PREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^p\d+$").expect("pref"));
static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)x(\d+)").expect("size"));
static PHONES: LazyLock<Mutex<HashMap<String, Phone>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Default)]
struct Phone {
    serial: Option<String>,
    refs: Vec<(String, Target)>,
    size: Option<(i32, i32)>,
}

fn adb_path(home: &Path) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    let mut cands: Vec<PathBuf> = which("adb").into_iter().collect();
    cands.push(home.join(TOOLS).join(exe));
    if cfg!(windows) {
        cands.push(
            PathBuf::from(env::var_os("LOCALAPPDATA").unwrap_or_default())
                .join(r"Android\Sdk\platform-tools")
                .join(exe),
        );
    } else if let Some(h) = env::var_os("HOME").map(PathBuf::from) {
        cands.push(h.join("Library/Android/sdk/platform-tools/adb"));
        cands.push(h.join("Android/Sdk/platform-tools/adb"));
    }
    cands.into_iter().find(|p| p.is_file())
}

fn install_tools(home: &Path) -> Result<PathBuf, Fail> {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let data = tools::fetch(&format!("https://dl.google.com/android/repository/platform-tools-latest-{os}.zip"))?;
    fs::create_dir_all(home).map_err(|e| Fail::new(e.to_string()))?;
    let zip = home.join("platform-tools.zip");
    fs::write(&zip, data).map_err(|e| Fail::new(e.to_string()))?;
    let (exe, args): (&str, Vec<String>) = if cfg!(target_os = "linux") {
        ("unzip", vec!["-oq".into(), zip.display().to_string(), "-d".into(), home.display().to_string()])
    } else {
        ("tar", vec!["-xf".into(), zip.display().to_string(), "-C".into(), home.display().to_string()])
    };
    let system = PathBuf::from(env::var_os("SystemRoot").unwrap_or_default()).join(r"System32\tar.exe");
    let tool =
        if cfg!(windows) && system.is_file() { system } else { which(exe).unwrap_or_else(|| PathBuf::from(exe)) };
    let ran = tools::run(&tool, &args, Duration::from_secs(DUMP_S));
    let _ = fs::remove_file(&zip);
    if !ran.as_ref().is_ok_and(|r| r.ok) {
        return Err(Fail::hint("could not unpack platform-tools", format!("install {exe}, then lighting phone setup")));
    }
    #[cfg(unix)]
    for name in ["adb", "fastboot"] {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(home.join(TOOLS).join(name), fs::Permissions::from_mode(0o755));
    }
    adb_path(home).ok_or_else(|| Fail::new("platform-tools were unpacked but adb is missing"))
}

fn adb_raw(home: &Path, serial: Option<&str>, args: &[&str], secs: u64) -> Result<tools::Ran, Fail> {
    let exe = adb_path(home).ok_or_else(|| {
        Fail::hint(
            "phone control needs adb (Android platform-tools)",
            "lighting phone setup (downloads them from Google)",
        )
    })?;
    let mut all: Vec<String> = serial.map(|s| vec!["-s".to_string(), s.to_string()]).unwrap_or_default();
    all.extend(args.iter().map(|a| (*a).to_string()));
    tools::run(&exe, &all, Duration::from_secs(secs)).map_err(|_| {
        Fail::hint(
            format!("adb did not answer within {secs} s"),
            "unplug and plug the phone in, then lighting phone devices",
        )
    })
}

fn adb(home: &Path, serial: Option<&str>, args: &[&str], secs: u64) -> Result<String, Fail> {
    let ran = adb_raw(home, serial, args, secs)?;
    if !ran.ok && ran.out.is_empty() {
        let err = ran.err.trim();
        if err.contains("unauthorized") {
            return Err(Fail::hint("the phone has not allowed this computer yet", ALLOW));
        }
        if err.contains("no devices") || err.contains("not found") {
            return Err(Fail::hint("no phone connected", PLUG));
        }
        let last: String = err.lines().last().unwrap_or("exit 1").chars().take(ERR_MAX).collect();
        return Err(Fail::new(format!("adb: {last}")));
    }
    Ok(ran.out)
}

struct Job<'a> {
    d: &'a Daemon,
    ctx: &'a mut Ctx,
    c: &'a Call,
    ph: &'a mut Phone,
}

impl Job<'_> {
    fn adb(&self, args: &[&str]) -> Result<String, Fail> {
        adb(&self.d.home, self.ph.serial.as_deref(), args, ADB_S)
    }

    fn devices(&self) -> Result<Vec<(String, String, String)>, Fail> {
        Ok(parse_devices(&adb(&self.d.home, None, &["devices", "-l"], ADB_S)?))
    }

    fn pick(&mut self) -> Result<(), Fail> {
        if let Some(dev) = self.c.val("device").filter(|v| !v.is_empty()) {
            self.ph.serial = Some(dev.to_string());
            return Ok(());
        }
        let all = self.devices()?;
        let rows: Vec<&(String, String, String)> = all.iter().filter(|r| r.1 == "device").collect();
        if self.ph.serial.as_ref().is_some_and(|s| rows.iter().any(|r| r.0 == *s)) {
            return Ok(());
        }
        match rows.as_slice() {
            [] if all.iter().any(|r| r.1 == "unauthorized") => {
                Err(Fail::hint("the phone has not allowed this computer yet", ALLOW))
            }
            [] => Err(Fail::hint("no phone connected", PLUG)),
            [one] => {
                self.ph.serial = Some(one.0.clone());
                Ok(())
            }
            many => Err(Fail::hint(
                format!("{} phones connected", many.len()),
                format!("lighting phone snap --device {}", many[0].0),
            )),
        }
    }

    fn snapshot(&mut self, f: Option<&str>) -> Result<String, Fail> {
        let mut raw = self.adb(&["exec-out", "uiautomator", "dump", "/dev/tty"]);
        if raw.as_deref().ok().and_then(tree_xml).is_none() {
            let cmd = "uiautomator dump /sdcard/lighting.xml >/dev/null && cat /sdcard/lighting.xml";
            raw = adb(&self.d.home, self.ph.serial.as_deref(), &["shell", cmd], DUMP_S);
        }
        let raw = raw?;
        let xml = tree_xml(&raw).ok_or_else(|| {
            Fail::hint(
                "the phone gave no screen tree (screen off or a secure screen?)",
                "lighting phone key wake, then snap",
            )
        })?;
        let filt = terms(f.unwrap_or(""));
        let snap = snap_of(xml, &filt)?;
        self.ph.refs = snap.refs;
        let rows: Vec<&str> = if filt.is_empty() {
            snap.lines.iter().map(|(_, l)| l.as_str()).collect()
        } else {
            let pairs: Vec<(Vec<u8>, &str)> = snap.lines.iter().map(|(t, l)| (t.clone(), l.as_str())).collect();
            best_only(&pairs)
        };
        let body = if rows.is_empty() { "(nothing found)".to_string() } else { rows.join("\n") };
        let serial = self.ph.serial.clone().unwrap_or_default();
        let text = format!("[phone {serial}] {} ({} controls)\n{body}", snap.app, self.ph.refs.len());
        Ok(capped(self.d, self.ctx, &text, "phone-snap", Some(SNAP_LINES), OUT_CHARS))
    }

    fn find(&mut self, spec: &str) -> Result<Target, Fail> {
        if PREF.is_match(spec) {
            return self.ph.refs.iter().find(|(r, _)| r == spec).map(|(_, t)| t.clone()).ok_or_else(|| {
                Fail::hint(format!("{spec} is unknown or from an older snapshot"), "lighting phone snap")
            });
        }
        self.snapshot(None)?;
        let want = vec![spec.to_lowercase()];
        let ranked: Vec<(u8, &Target)> = self
            .ph
            .refs
            .iter()
            .map(|(_, t)| (if t.name.to_lowercase() == want[0] { 4 } else { tiers(&want, &[&t.name])[0] }, t))
            .collect();
        let top = ranked.iter().map(|(r, _)| *r).max().unwrap_or(0);
        ranked
            .into_iter()
            .find(|(r, _)| top > 0 && *r == top)
            .map(|(_, t)| t.clone())
            .ok_or_else(|| Fail::hint(format!("nothing called \"{spec}\" on the phone"), "lighting phone snap"))
    }

    fn after(&mut self, what: String) -> Result<Out, Fail> {
        thread::sleep(SETTLE);
        match self.c.val("f").filter(|f| !f.is_empty()) {
            Some(f) => Ok(Out::Text(format!("{what}\n{}", self.snapshot(Some(f))?))),
            None => Ok(Out::Text(what)),
        }
    }

    fn tap(&mut self, rest: &[String]) -> Result<Out, Fail> {
        if rest.is_empty() {
            return Err(Fail::hint("tap needs a ref or text", HELP));
        }
        let t = self.find(&rest.join(" "))?;
        let (x, y) = (t.x.to_string(), t.y.to_string());
        if self.c.on("long") {
            self.adb(&["shell", "input", "swipe", &x, &y, &x, &y, LONG_MS])?;
        } else {
            self.adb(&["shell", "input", "tap", &x, &y])?;
        }
        let name: String = t.name.chars().take(SAID_MAX).collect();
        self.after(format!("ok (tapped {} \"{name}\" at {},{})", t.role, t.x, t.y))
    }

    fn type_in(&mut self, mut rest: &[String]) -> Result<Out, Fail> {
        if let Some(first) = rest.first().filter(|f| PREF.is_match(f)) {
            let t = self.find(first)?;
            self.adb(&["shell", "input", "tap", &t.x.to_string(), &t.y.to_string()])?;
            thread::sleep(TAP_SETTLE);
            rest = &rest[1..];
        }
        let text = self.ctx.secret.clone().unwrap_or_else(|| rest.join(" "));
        if !text.is_empty() {
            if !text.is_ascii() {
                return Err(Fail::hint(
                    "adb can only type plain ASCII text",
                    "type it in parts, or paste it on the phone",
                ));
            }
            self.adb(&["shell", "input", "text", &escape_text(&text)])?;
        }
        let enter = self.c.on("enter");
        if enter {
            self.adb(&["shell", "input", "keyevent", "66"])?;
        }
        self.after(format!("ok (typed {} chars{})", text.chars().count(), if enter { " +enter" } else { "" }))
    }

    fn key(&mut self, rest: &[String]) -> Result<Out, Fail> {
        let names: Vec<String> = if rest.is_empty() { vec!["back".into()] } else { rest.to_vec() };
        for k in &names {
            let code = KEYS.iter().find(|(n, _)| *n == k.to_lowercase()).map(|(_, c)| c.to_string());
            let code = match code {
                Some(c) => c,
                None if !k.is_empty() && k.chars().all(|ch| ch.is_ascii_digit()) => k.clone(),
                None => {
                    return Err(Fail::hint(
                        format!("unknown phone key {k}"),
                        "back, home, recent, enter, delete, volup, voldown, power, playpause",
                    ));
                }
            };
            self.adb(&["shell", "input", "keyevent", &code])?;
        }
        self.after(format!("ok ({})", names.join(" ")))
    }

    fn swipe(&mut self, rest: &[String]) -> Result<Out, Fail> {
        if self.ph.size.is_none() {
            let out = self.adb(&["shell", "wm", "size"])?;
            let m = SIZE.captures(&out);
            self.ph.size =
                Some(m.map_or(DEFAULT_SIZE, |m| (m[1].parse().unwrap_or(1080), m[2].parse().unwrap_or(2400))));
        }
        let (w, h) = self.ph.size.unwrap_or(DEFAULT_SIZE);
        let way = rest.first().map_or_else(|| "up".to_string(), |r| r.to_lowercase());
        let (dx, dy) = match way.as_str() {
            "down" => (0, h / 3),
            "left" => (-w / 3, 0),
            "right" => (w / 3, 0),
            _ => (0, -h / 3),
        };
        let (cx, cy) = (w / 2, h / 2);
        let p = [cx - dx / 2, cy - dy / 2, cx + dx / 2, cy + dy / 2].map(|v| v.to_string());
        self.adb(&["shell", "input", "swipe", &p[0], &p[1], &p[2], &p[3], SWIPE_MS])?;
        self.after(format!("ok (swiped {way})"))
    }

    fn shot(&mut self) -> Result<Out, Fail> {
        let ran = adb_raw(&self.d.home, self.ph.serial.as_deref(), &["exec-out", "screencap", "-p"], DUMP_S)?;
        let img = image::load_from_memory(&ran.bytes)
            .map_err(|_| Fail::hint("the phone sent no picture", "unlock the phone, then lighting phone shot"))?;
        let key = format!("phone-{}", self.ph.serial.clone().unwrap_or_default());
        save_picture(self.d, self.ctx, self.c, &img, &key, "lighting phone shot --out shot.png").map(Out::Text)
    }

    fn packages(&self, third: bool) -> Result<Vec<String>, Fail> {
        let mut args = vec!["shell", "pm", "list", "packages"];
        if third {
            args.push("-3");
        }
        Ok(self.adb(&args)?.lines().filter_map(|l| l.strip_prefix("package:")).map(|p| p.trim().to_string()).collect())
    }

    fn launch(&mut self, rest: &[String]) -> Result<Out, Fail> {
        if rest.is_empty() {
            return Err(Fail::hint("launch needs an app", "lighting phone launch spotify"));
        }
        let mut name = rest.join(" ");
        if !name.contains('.') {
            let low = name.to_lowercase();
            let mut hits: Vec<String> =
                self.packages(false)?.into_iter().filter(|p| p.to_lowercase().contains(&low)).collect();
            hits.sort_by_key(|p| (!p.to_lowercase().ends_with(&format!(".{low}")), p.len()));
            name = hits.into_iter().next().ok_or_else(|| {
                Fail::hint(format!("no app matches \"{name}\""), format!("lighting phone apps -f {name}"))
            })?;
        }
        let out = self.adb(&["shell", "monkey", "-p", &name, "-c", "android.intent.category.LAUNCHER", "1"])?;
        if out.contains("No activities found") || out.to_lowercase().contains("aborted") {
            return Err(Fail::new(format!("{name} has no app to open")));
        }
        self.after(format!("ok (started {name})"))
    }

    fn apps(&mut self) -> Result<Out, Fail> {
        let mut pk = self.packages(true)?;
        pk.sort();
        if let Some(f) = self.c.val("f").filter(|f| !f.is_empty()) {
            let want = terms(f);
            pk.retain(|p| want.iter().any(|w| p.to_lowercase().contains(w)));
        }
        let text = if pk.is_empty() { "no apps found".to_string() } else { pk.join("\n") };
        Ok(Out::Text(capped(self.d, self.ctx, &text, "phone-apps", Some(APPS_LINES), OUT_CHARS)))
    }

    fn run(&mut self) -> Result<Out, Fail> {
        let sub = self.c.pos.first().map_or_else(|| "snap".to_string(), |p| p.to_lowercase());
        let rest: Vec<String> = self.c.pos.iter().skip(1).cloned().collect();
        match sub.as_str() {
            "setup" => {
                let said = match adb_path(&self.d.home) {
                    Some(p) => format!("adb found: {}", p.display()),
                    None => format!("downloaded Android platform-tools to {}", install_tools(&self.d.home)?.display()),
                };
                return Ok(Out::Text(said));
            }
            "devices" => {
                let rows = self.devices()?;
                if rows.is_empty() {
                    return Ok(Out::Text(format!("no phone connected -> {PLUG}")));
                }
                return Ok(Out::Text(
                    rows.iter().map(|r| format!("{} {} {}", r.0, r.1, r.2)).collect::<Vec<_>>().join("\n"),
                ));
            }
            "help" => return Ok(Out::Text(HELP.into())),
            _ => {}
        }
        self.pick()?;
        match sub.as_str() {
            "snap" => self.snapshot(self.c.val("f")).map(Out::Text),
            "tap" | "click" => self.tap(&rest),
            "type" => self.type_in(&rest),
            "key" | "press" => self.key(&rest),
            "swipe" | "scroll" => self.swipe(&rest),
            "shot" | "screenshot" => self.shot(),
            "launch" | "start" => self.launch(&rest),
            "open" => {
                let Some(url) = rest.first() else {
                    return Err(Fail::hint("open needs a link", "lighting phone open https://example.com"));
                };
                self.adb(&["shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", url])?;
                self.after(format!("ok (opened {} on the phone)", url.chars().take(URL_SHOWN).collect::<String>()))
            }
            "apps" => self.apps(),
            _ => Err(Fail::hint(format!("unknown phone command {sub}"), HELP)),
        }
    }
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let mut ph = PHONES.lock().unwrap_or_else(PoisonError::into_inner).remove(&ctx.sid).unwrap_or_default();
    let sid = ctx.sid.clone();
    let out = Job { d, ctx, c, ph: &mut ph }.run();
    PHONES.lock().unwrap_or_else(PoisonError::into_inner).insert(sid, ph);
    out
}
