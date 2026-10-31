mod agents;
mod claude_env;
mod place;
#[cfg(windows)]
mod reg;

use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::client::ask;
use crate::daemon::MARKER;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PURGE_DIRS: &[&str] = &["out", "extension", "bin", "routines"];
const PURGE_FILES: &[&str] =
    &["key", "log.jsonl", "config.json", "daemon.pid", "episodes.jsonl", "apps.json", "bench-real.json", MARKER];

pub fn run(home: &Path, args: &[String]) -> ExitCode {
    let first = args.first().map(String::as_str).unwrap_or("");
    let flag = |f: &str| args.iter().any(|a| a == f);
    let global = flag("--register") || std::env::var_os("LIGHTING_HOME").is_none_or(|h| h.is_empty());
    let done = match first {
        "uninstall" => uninstall(home, flag("--purge"), global),
        "install" => install(home, args.get(1).map(String::as_str).unwrap_or(""), global),
        _ => setup(
            home,
            flag("--manual"),
            global,
            args.iter().skip(1).find(|a| !a.starts_with("--")).map(String::as_str),
        ),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lighting {first}: {e}");
            ExitCode::from(1)
        }
    }
}

fn refresh(home: &Path, global: bool) -> Result<(Option<String>, std::path::PathBuf, bool), String> {
    let text = |e: std::io::Error| e.to_string();
    fs::create_dir_all(home.join("out")).map_err(text)?;
    fs::write(home.join(MARKER), "").map_err(text)?;
    place::blocklist(home).map_err(text)?;
    let root = place::source_root();
    let ext_id = place::copy_extension(home, root.as_deref()).map_err(text)?;
    let (exe, daemon_new) = place::copy_exes(home).map_err(text)?;
    place::write_host(home, &exe, ext_id.as_deref()).map_err(text)?;
    if global {
        agents::refresh_skills(root.as_deref());
    }
    Ok((ext_id, exe, daemon_new))
}

fn installed(home: &Path, exe: &Path, argv: &[&str]) -> Result<String, String> {
    let out = Command::new(exe)
        .args(argv)
        .env("LIGHTING_HOME", home)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot start {} ({e})", exe.display()))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

pub fn refresh_extension(home: &Path) -> std::io::Result<Option<String>> {
    place::copy_extension(home, place::source_root().as_deref())
}

fn connected(home: &Path, exe: &Path) -> Vec<String> {
    let Ok(out) = installed(home, exe, &["status"]) else { return Vec::new() };
    out.lines().filter(|l| l.starts_with("browser ") && !l.contains("none connected")).map(String::from).collect()
}

fn setup(home: &Path, manual: bool, global: bool, browser: Option<&str>) -> Result<(), String> {
    let (ext_id, exe, daemon_new) = refresh(home, global)?;
    let browsers = crate::daemon::installed_browsers();
    println!("lighting {VERSION} set up in {}", home.display());
    if daemon_new && installed(home, &exe, &["stop"]).is_ok_and(|out| out.ends_with("stopped")) {
        println!("stopped the daemon that ran the old build, the next command starts the new one");
    }
    if global {
        println!("native host registered for: {}", place::register(home, &browsers).join(", "));
        match place::add_to_path(&exe) {
            Ok(Some(at)) => {
                println!("added {} to your PATH (new terminals and other AIs find `lighting`)", at.display())
            }
            Ok(None) => {}
            Err(e) => println!("could not add {} to PATH: {e}", exe.display()),
        }
        let file = claude_env::settings_file();
        match claude_env::point(&file, &exe) {
            Ok(true) => println!(
                "set LIGHTING_BIN in {}: new Claude sessions start the MCP server from this drive",
                file.display()
            ),
            Ok(false) => {}
            Err(e) => println!("{e}"),
        }
    } else {
        println!("browser link and PATH left alone (LIGHTING_HOME is set; --register writes them for this folder)");
    }
    println!("browsers found: {}", if browsers.is_empty() { "none".into() } else { browsers.join(", ") });
    if !cfg!(windows)
        && let Ok(access) = installed(home, &exe, &["access-prompt"])
    {
        println!("{access}");
    }
    let mut lines = connected(home, &exe);
    if lines.is_empty() && !manual && global && !browsers.is_empty() {
        println!("loading the extension by itself (takes ~10 s) ...");
        let mut argv = vec!["autoload"];
        argv.extend(browser);
        match installed(home, &exe, &argv) {
            Ok(out) => println!("{out}"),
            Err(e) => println!("automatic load failed: {e}"),
        }
        lines = connected(home, &exe);
    }
    if !lines.is_empty() {
        println!("{}", lines.join("\n"));
        println!("ready: try `lighting open https://example.com`");
        return Ok(());
    }
    println!("one manual step left, once per browser:");
    println!("  1. open brave://extensions (or chrome://extensions, edge://extensions)");
    println!("  2. switch on Developer mode");
    println!("  3. Load unpacked -> {}", home.join("extension").display());
    println!("extension id {} | then run: lighting status", ext_id.as_deref().unwrap_or("?"));
    Ok(())
}

fn install(home: &Path, name: &str, global: bool) -> Result<(), String> {
    if !agents::NAMES.contains(&name) {
        return Err(format!("usage: lighting install <{}>", agents::NAMES.join("|")));
    }
    if !global {
        return Err(format!(
            "{name} keeps one Lighting for every folder; LIGHTING_HOME is set, add --register to point it here"
        ));
    }
    let (_, exe, _) = refresh(home, global)?;
    agents::install(name, &exe, place::source_root().as_deref()).map_err(|e| e.to_string())?;
    println!("lighting MCP server added to {name}, restart it to load the tool");
    Ok(())
}

fn uninstall(home: &Path, purge: bool, global: bool) -> Result<(), String> {
    if home.join("key").is_file() {
        let _ = ask(home, &["stop"]);
    }
    if global {
        place::unregister();
        let file = claude_env::settings_file();
        match claude_env::unpoint(&file) {
            Ok(true) => println!("removed LIGHTING_BIN from {}", file.display()),
            Ok(false) => {}
            Err(e) => println!("{e}"),
        }
    }
    if !purge {
        println!(
            "removed the browser link, stopped the daemon (data kept in {}, add --purge to remove)",
            home.display()
        );
        return Ok(());
    }
    for dir in PURGE_DIRS {
        let _ = fs::remove_dir_all(home.join(dir));
    }
    let host = place::host_json(home);
    for file in PURGE_FILES.iter().map(|f| home.join(f)).chain([host]) {
        let _ = fs::remove_file(file);
    }
    println!("removed the browser link and the Lighting files in {} (memory and notes stay)", home.display());
    Ok(())
}
