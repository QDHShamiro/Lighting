use lt_core::{Call, Fail, Flag, OUT_CHARS, ago, built_later, cap, listing, parse, stats, unknown, wants_help};

use super::{Daemon, Out, Req, VERSION};

const HELP: &str = "lighting 3.0 core (browser, desktop and memory follow in the next milestones)
  ping                     is the daemon there
  status                   version, pid and uptime of this daemon
  log [n] | log stats [n]  the last commands, or counts per command
  version                  version of this daemon
  stop | abort             stop the daemon | stop the running and queued commands
  more <ref>               a squeezed tool output in full (--lines a-b, --grep \"word|other\")";
const STATS_LINES: usize = 40;

pub fn run_one(d: &Daemon, req: &mut Req) -> Result<(Out, String), Fail> {
    let call = parse(&req.argv);
    if call.name.is_empty() {
        return Err(Fail::hint("no command", "lighting help"));
    }
    let asks_help = matches!(call.name.as_str(), "help" | "-h" | "--help");
    if !asks_help && wants_help(&req.argv).is_some() {
        return Ok((Out::Text(help(Some(&call.name))?), "help".into()));
    }
    if call.flags.get("f") == Some(&Flag::On) {
        return Err(Fail::hint(
            "-f needs the words to look for, like -f \"save|draft\"; nothing was done",
            format!("lighting {} ... -f \"<words>\"", call.name),
        ));
    }
    let out = match call.name.as_str() {
        "ping" => Out::Text("pong".into()),
        "version" => Out::Text(VERSION.into()),
        "stop" => Out::Stop("lighting daemon stopped".into()),
        "status" => Out::Text(status(d)),
        "log" => Out::Text(log(d, req, &call)),
        _ if asks_help => Out::Text(help(call.pos.first().map(String::as_str))?),
        name if built_later(name) => {
            return Err(Fail::hint(
                format!("{name} is not built into Lighting 3.0 yet"),
                "run it with Lighting 2.x (without LIGHTING_HOME)",
            ));
        }
        name => return Err(unknown(name)),
    };
    Ok((out, call.name))
}

fn help(name: Option<&str>) -> Result<String, Fail> {
    match name.map(str::to_lowercase) {
        Some(name) if !name.is_empty() && !built_later(&name) => Err(unknown(&name)),
        _ => Ok(HELP.into()),
    }
}

fn status(d: &Daemon) -> String {
    let home = d.home.to_string_lossy().replace('\\', "/");
    let up = ago(d.started.elapsed().as_secs());
    format!("lighting {VERSION} | daemon pid {} up {up} | hotkey unavailable\nhome {home}", d.pid)
}

fn log(d: &Daemon, req: &mut Req, call: &Call) -> String {
    let mut pos = call.pos.as_slice();
    let mut counts = call.on("stats");
    if pos.first().is_some_and(|p| p.eq_ignore_ascii_case("stats")) {
        pos = &pos[1..];
        counts = true;
    }
    let n = pos.first().filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).and_then(|p| p.parse().ok());
    if !counts {
        return listing(&d.home, n.unwrap_or(15));
    }
    let Some(text) = stats(&d.home, n.unwrap_or(2000)) else { return "log is empty".into() };
    let (text, cut) = cap(&d.home.join("out"), &text, "log-stats", Some(STATS_LINES), OUT_CHARS);
    req.cut += cut;
    text
}
