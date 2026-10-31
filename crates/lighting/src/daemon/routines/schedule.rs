use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use lt_core::Fail;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::store::{self, indent_one, text};

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const DAYS_DE: &[(&str, &str)] =
    &[("mo", "mon"), ("di", "tue"), ("mi", "wed"), ("do", "thu"), ("fr", "fri"), ("sa", "sat"), ("so", "sun")];
const HELP: &str =
    "wisply routine schedule <name> daily 08:00 | weekly mon 09:30 | hourly | every 30m [param=value ...]";
const MIN_EVERY: u64 = 5;
const HOUR: u64 = 60;
const ERR_CHARS: usize = 200;
const LABEL: &str = "dev.qdhshamiro.wisply";
const SESSION_ENV: &[&str] =
    &["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_SESSION_TYPE", "DBUS_SESSION_BUS_ADDRESS"];

static EVERY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)\s*(m|min|h)?$").expect("every"));
static CLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2}):(\d{2})$").expect("clock"));

fn file(home: &Path) -> PathBuf {
    home.join("schedules.json")
}

pub fn load(home: &Path) -> Map<String, Value> {
    fs::read_to_string(file(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn store_all(home: &Path, data: &Map<String, Value>) -> Result<(), Fail> {
    let _ = fs::create_dir_all(home);
    let pretty = serde_json::to_string_pretty(data).unwrap_or_default();
    fs::write(file(home), indent_one(&pretty)).map_err(|e| Fail::new(format!("cannot save the schedules ({e})")))
}

fn clock(part: Option<&String>) -> Result<String, Fail> {
    let bad = || Fail::hint("needs a time like 08:00", HELP);
    let m = CLOCK.captures(part.map_or("", String::as_str)).ok_or_else(bad)?;
    let (h, mi): (u32, u32) = (m[1].parse().map_err(|_| bad())?, m[2].parse().map_err(|_| bad())?);
    if h > 23 || mi > 59 {
        return Err(bad());
    }
    Ok(format!("{h:02}:{mi:02}"))
}

pub fn parse(words: &[String]) -> Result<(Value, Vec<String>), Fail> {
    let w: Vec<String> = words.iter().map(|x| x.to_lowercase()).collect();
    let Some(first) = w.first() else { return Err(Fail::hint("when should it run?", HELP)) };
    let rest = |n: usize| words.get(n..).unwrap_or(&[]).to_vec();
    match first.as_str() {
        "hourly" => Ok((json!({"kind": "every", "minutes": HOUR}), rest(1))),
        "every" if w.len() > 1 => {
            let m = EVERY.captures(&w[1]).ok_or_else(|| Fail::hint(format!("cannot read {}", words[1]), HELP))?;
            let n: u64 = m[1].parse().map_err(|_| Fail::hint(format!("cannot read {}", words[1]), HELP))?;
            let n = n * if m.get(2).is_some_and(|u| u.as_str() == "h") { HOUR } else { 1 };
            if n < MIN_EVERY {
                return Err(Fail::hint("at most every 5 minutes", HELP));
            }
            Ok((json!({"kind": "every", "minutes": n}), rest(2)))
        }
        "daily" | "täglich" | "taeglich" => Ok((json!({"kind": "daily", "at": clock(words.get(1))?}), rest(2))),
        "weekly" | "wöchentlich" if w.len() > 1 => {
            let short: String = w[1].chars().take(2).collect();
            let three: String = w[1].chars().take(3).collect();
            let day = DAYS_DE.iter().find(|(de, _)| *de == short).map_or(three.as_str(), |(_, en)| en).to_string();
            if !DAYS.contains(&day.as_str()) {
                return Err(Fail::hint(format!("unknown day {}", words[1]), HELP));
            }
            Ok((json!({"kind": "weekly", "day": day, "at": clock(words.get(2))?}), rest(3)))
        }
        _ => Err(Fail::hint(format!("unknown schedule {}", words[0]), HELP)),
    }
}

pub fn label(when: &Value) -> String {
    match when["kind"].as_str() {
        Some("every") if when["minutes"] == HOUR => "hourly".into(),
        Some("every") => format!("every {} min", when["minutes"].as_u64().unwrap_or(0)),
        Some("daily") => format!("daily {}", text(&when["at"])),
        _ => format!("weekly {} {}", text(&when["day"]), text(&when["at"])),
    }
}

fn refused(by: &str, out: std::io::Result<std::process::Output>) -> Result<(), Fail> {
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let why: String = String::from_utf8_lossy(&o.stderr).trim().chars().take(ERR_CHARS).collect();
            Err(Fail::new(format!("{by} refused: {why}")))
        }
        Err(e) => Err(Fail::new(format!("{by} refused: {e}"))),
    }
}

fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn client() -> Result<PathBuf, Fail> {
    env::current_exe().map_err(|e| Fail::new(format!("cannot find wisply itself ({e})")))
}

pub fn windows_line(task: &Path, home: &Path, name: &str, args: &[String]) -> String {
    let mut parts = vec![format!("\"{}\"", task.display()), format!("\"{}\"", home.display())];
    for a in ["run", name].iter().map(|s| s.to_string()).chain(args.iter().cloned()) {
        parts.push(format!("\"{}\"", a.replace('"', "")));
    }
    parts.join(" ")
}

fn add_windows(home: &Path, name: &str, when: &Value, args: &[String]) -> Result<(), Fail> {
    let task = client()?.with_file_name("wisply-task.exe");
    if !task.is_file() {
        return Err(Fail::hint(format!("{} is missing", task.display()), "install Wisply again (wisply setup)"));
    }
    let mut cmd = Command::new("schtasks");
    cmd.args(["/Create", "/F", "/TN", &format!("Wisply {name}"), "/TR", &windows_line(&task, home, name, args)]);
    match when["kind"].as_str() {
        Some("every") => cmd.args(["/SC", "MINUTE", "/MO", &when["minutes"].to_string()]),
        Some("daily") => cmd.args(["/SC", "DAILY", "/ST", &text(&when["at"])]),
        _ => cmd.args(["/SC", "WEEKLY", "/D", &text(&when["day"]).to_uppercase(), "/ST", &text(&when["at"])]),
    };
    refused("Task Scheduler", quiet(&mut cmd).output())
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn plist(exe: &Path, home: &Path, name: &str, when: &Value, args: &[String]) -> String {
    let log = xml(&home.join(format!("schedule-{name}.log")).display().to_string());
    let mut argv = vec![exe.display().to_string(), "run".into(), name.to_string()];
    argv.extend(args.iter().cloned());
    let argv: String = argv.iter().map(|a| format!("\t\t<string>{}</string>\n", xml(a))).collect();
    let timing = match when["kind"].as_str() {
        Some("every") => {
            format!(
                "\t<key>StartInterval</key>\n\t<integer>{}</integer>\n",
                when["minutes"].as_u64().unwrap_or(HOUR) * 60
            )
        }
        kind => {
            let at = text(&when["at"]);
            let (h, m) = at.split_once(':').unwrap_or(("0", "0"));
            let (h, m) = (h.parse::<u32>().unwrap_or(0), m.parse::<u32>().unwrap_or(0));
            let mut cal = format!(
                "\t\t<key>Hour</key>\n\t\t<integer>{h}</integer>\n\t\t<key>Minute</key>\n\t\t<integer>{m}</integer>\n"
            );
            if kind == Some("weekly") {
                let day = DAYS.iter().position(|d| *d == text(&when["day"])).unwrap_or(0);
                cal.push_str(&format!("\t\t<key>Weekday</key>\n\t\t<integer>{}</integer>\n", (day + 1) % 7));
            }
            format!("\t<key>StartCalendarInterval</key>\n\t<dict>\n{cal}\t</dict>\n")
        }
    };
    let home = xml(&home.display().to_string());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>{LABEL}.{name}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n{argv}\t</array>\n\t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>WISPLY_HOME</key>\n\t\t<string>{home}</string>\n\t</dict>\n\t<key>StandardOutPath</key>\n\t<string>{log}</string>\n\t<key>StandardErrorPath</key>\n\t<string>{log}</string>\n{timing}</dict>\n</plist>\n"
    )
}

fn user_home() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default())
}

fn plist_path(name: &str) -> PathBuf {
    user_home().join("Library").join("LaunchAgents").join(format!("{LABEL}.{name}.plist"))
}

#[cfg(unix)]
fn uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(not(unix))]
fn uid() -> String {
    String::new()
}

fn add_macos(home: &Path, name: &str, when: &Value, args: &[String]) -> Result<(), Fail> {
    let p = plist_path(name);
    let _ = fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
    let domain = format!("gui/{}", uid());
    let _ = Command::new("launchctl").args(["bootout", &domain, &p.display().to_string()]).output();
    fs::write(&p, plist(&client()?, home, name, when, args))
        .map_err(|e| Fail::new(format!("cannot write {} ({e})", p.display())))?;
    refused("launchd", Command::new("launchctl").args(["bootstrap", &domain, &p.display().to_string()]).output())
}

fn unit_dir() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map_or_else(|| user_home().join(".config"), PathBuf::from)
        .join("systemd")
        .join("user")
}

pub fn units(exe: &Path, home: &Path, name: &str, when: &Value, args: &[String]) -> (String, String) {
    let mut env_lines = format!("Environment=WISPLY_HOME={}\n", home.display());
    for k in SESSION_ENV {
        if let Some(v) = env::var(k).ok().filter(|v| !v.is_empty()) {
            env_lines.push_str(&format!("Environment={k}={v}\n"));
        }
    }
    let quoted: Vec<String> = args.iter().map(|a| format!("'{}'", a.replace('\'', ""))).collect();
    let exec = [exe.display().to_string(), "run".into(), name.to_string()]
        .into_iter()
        .chain(quoted)
        .collect::<Vec<_>>()
        .join(" ");
    let service =
        format!("[Unit]\nDescription=Wisply routine {name}\n\n[Service]\nType=oneshot\n{env_lines}ExecStart={exec}\n");
    let at = text(&when["at"]);
    let timing = match when["kind"].as_str() {
        Some("every") => {
            let m = when["minutes"].as_u64().unwrap_or(HOUR);
            format!("OnActiveSec={m}min\nOnUnitActiveSec={m}min\n")
        }
        Some("daily") => format!("OnCalendar=*-*-* {at}:00\nPersistent=true\n"),
        _ => {
            let day = text(&when["day"]);
            let day = day[..1].to_uppercase() + &day[1..];
            format!("OnCalendar={day} *-*-* {at}:00\nPersistent=true\n")
        }
    };
    let timer =
        format!("[Unit]\nDescription=Wisply routine {name}\n\n[Timer]\n{timing}\n[Install]\nWantedBy=timers.target\n");
    (service, timer)
}

fn add_linux(home: &Path, name: &str, when: &Value, args: &[String]) -> Result<(), Fail> {
    let unit = format!("wisply-{name}");
    let (service, timer) = units(&client()?, home, name, when, args);
    let dir = unit_dir();
    let _ = fs::create_dir_all(&dir);
    let wrote = fs::write(dir.join(format!("{unit}.service")), service)
        .and_then(|_| fs::write(dir.join(format!("{unit}.timer")), timer));
    wrote.map_err(|e| Fail::new(format!("cannot write the systemd units ({e})")))?;
    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).output();
    refused("systemd", Command::new("systemctl").args(["--user", "enable", "--now", &format!("{unit}.timer")]).output())
}

fn remove_os(name: &str) {
    if cfg!(windows) {
        let _ = quiet(Command::new("schtasks").args(["/Delete", "/F", "/TN", &format!("Wisply {name}")])).output();
    } else if cfg!(target_os = "macos") {
        let p = plist_path(name);
        let _ =
            Command::new("launchctl").args(["bootout", &format!("gui/{}", uid()), &p.display().to_string()]).output();
        let _ = fs::remove_file(p);
    } else {
        let unit = format!("wisply-{name}");
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", &format!("{unit}.timer")]).output();
        for ext in ["service", "timer"] {
            let _ = fs::remove_file(unit_dir().join(format!("{unit}.{ext}")));
        }
        let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).output();
    }
}

pub fn add(home: &Path, name: &str, words: &[String]) -> Result<String, Fail> {
    let r = store::load(home, name)?;
    let name = text(&r["name"]);
    let (when, rest) = parse(words)?;
    let args: Vec<String> = rest.into_iter().filter(|a| a.contains('=')).collect();
    let secret = r["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["args"].as_array().into_iter().flatten())
        .any(|a| text(a).contains("@secret"));
    if secret {
        return Err(Fail::new(format!("{name} types a secret, a schedule cannot pass one")));
    }
    let by = if cfg!(windows) {
        add_windows(home, &name, &when, &args)?;
        "Task Scheduler"
    } else if cfg!(target_os = "macos") {
        add_macos(home, &name, &when, &args)?;
        "launchd"
    } else {
        add_linux(home, &name, &when, &args)?;
        "systemd timer"
    };
    let mut data = load(home);
    data.insert(name.clone(), json!({"when": when, "args": args}));
    store_all(home, &data)?;
    let shown = if args.is_empty() { String::new() } else { format!(" {}", args.join(" ")) };
    Ok(format!("scheduled {name} {}{shown} ({by})", label(&when)))
}

pub fn remove(home: &Path, name: &str) -> Result<String, Fail> {
    let mut data = load(home);
    let Some(key) = data.keys().find(|k| k.to_lowercase() == name.to_lowercase()).cloned() else {
        return Err(Fail::hint(format!("{name} has no schedule"), "wisply routine schedules"));
    };
    remove_os(&key);
    data.shift_remove(&key);
    store_all(home, &data)?;
    Ok(format!("unscheduled {key}"))
}

pub fn listing(home: &Path) -> String {
    let data = load(home);
    if data.is_empty() {
        return format!("no scheduled routines -> {HELP}");
    }
    let mut keys: Vec<&String> = data.keys().collect();
    keys.sort();
    keys.iter()
        .map(|k| {
            let v = &data[*k];
            let args: Vec<String> = v["args"].as_array().into_iter().flatten().map(text).collect();
            let shown = if args.is_empty() { String::new() } else { format!(" {}", args.join(" ")) };
            format!("{k}  {}{shown}", label(&v["when"]))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn schedules_read_like_2x_and_become_os_jobs() {
        let (when, rest) = parse(&w("weekly Mo 9:05 q=tea x")).unwrap();
        assert_eq!((label(&when), rest), ("weekly mon 09:05".to_string(), w("q=tea x")));
        assert_eq!(label(&parse(&w("every 2h")).unwrap().0), "every 120 min");
        assert_eq!(label(&parse(&w("hourly")).unwrap().0), "hourly");
        assert_eq!(label(&parse(&w("täglich 23:59")).unwrap().0), "daily 23:59");
        assert!(parse(&w("every 4m")).unwrap_err().msg.contains("at most every 5 minutes"));
        assert!(parse(&w("daily 24:00")).unwrap_err().msg.contains("needs a time"));
        assert!(parse(&w("weekly xyz 08:00")).unwrap_err().msg.contains("unknown day"));
        let line = windows_line(Path::new(r"C:\L\wisply-task.exe"), Path::new(r"C:\H"), "mail", &w("q=a\"b"));
        assert_eq!(line, r#""C:\L\wisply-task.exe" "C:\H" "run" "mail" "q=ab""#);
        let p = plist(Path::new("/bin/wisply"), Path::new("/h"), "mail", &when, &w("q=a&b"));
        assert!(p.contains("<string>q=a&amp;b</string>") && p.contains("<key>Weekday</key>\n\t\t<integer>1</integer>"));
        assert!(p.contains("<key>WISPLY_HOME</key>\n\t\t<string>/h</string>"));
        let (service, timer) = units(Path::new("/bin/wisply"), Path::new("/h"), "mail", &when, &w("q=it's"));
        assert!(
            service.contains("Environment=WISPLY_HOME=/h\n")
                && service.contains("ExecStart=/bin/wisply run mail 'q=its'\n")
        );
        assert!(timer.contains("OnCalendar=Mon *-*-* 09:05:00\nPersistent=true\n"));
    }
}
