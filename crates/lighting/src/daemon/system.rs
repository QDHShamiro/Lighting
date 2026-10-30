use std::path::Path;
use std::sync::Arc;

use lt_core::{Call, Fail, Flag, OUT_CHARS, ago, built_later, listing, parse, stats, unknown, wants_help};

use super::ctx::Ctx;
use super::{Daemon, Out, VERSION, config, media, route, web};

const HELP: &str = "lighting 3.0 <command> [args]   browser, desktop apps, video and memory for AI agents
refs: e12 web element | t77 tab | w3 window | d5 app element

web     open <url> [-n] [-f \"a|b\"] [--text]   snap [-f \"a|b\"] [-s css|ref] [--all] [--full] [--diff] [--links]   text [-f txt] [--links]
        click <ref|\"text\"> [--yes] [--force]   fill \"Label=value\" ... [--submit]   search [site] \"words\"
        type <ref> <text> [--env VAR] [--append]   press <keys>   select <ref> <option>
        check <ref> [off]   hover <ref>   drag <ref> <ref>   scroll [ref|up|down] [--until \"text\"]
        wait <\"text\"|url:x|ms|ref> [--gone] [--reload 15s --timeout 10m]   expect <\"text\"|url:x> [--gone]
        fetch <url> [--pick a.b]   js <code|--file f>   dismiss   viewport <WxH|reset>
        upload <ref> <file>   tabs [-f x]   tab <id>   close [id]   back   forward   reload   table <ref|css>
        dialog accept|dismiss [text]   downloads   console   net [n] [-f x] (JSON the page loaded)
        tools   call <tool> '{json}' [--yes] (WebMCP)
apps    windows   focus <w3|app:Name>   launch <app|uri|file> [--yes]   close <w3>   clip [set \"text\"]
        snap|click|type|press|scroll|drag|read|shot ... w3 (or app:Name)   wait \"text\" --on app [--gone]
video   watch <link|file|e40> [\"question\"] [--from 1:00 --to 2:00] [--at 0:10,0:20] [--fps n] [--frames] [--text]
        listen <link|file|e40> [\"question\"] [--from --to] [--live] [--lang de] [--local]   shot --ask \"question\"
memory  recall \"words\" [--all] [--max n] | recall <id>   remember \"fact\" [--kind k]   forget <id>
        todo [--all] | todo add \"task\" [--list name] [--doing] | todo done|doing|drop|open <id> [note] | todo note <id> \"text\"
        memory [learn|on|off]   memory lessons | memory unlearn <n> (fixes learned from failed commands)
        wiki | wiki accept <n> [folder] | wiki drop <n> | wiki show <n> (drafts in the vault inbox)
keys    keys [site] [-f x]   keys add <site> \"ctrl+k=what\"   keys rm <site> <key>
cleanup done [--quiet]   keep [t]   config cleanup on|off
system  status   stop   abort   config [key value]   log [n] | log stats [n]   version
        more <ref> (a squeezed tool output in full, --lines a-b, --grep \"word|other\")
        secure [folder] [--history] [--no-deps] (keys, vulnerable dependencies, risky code; not while a game runs)

Any action takes -f \"a|b\" to show the matching lines of the result in the same call.
Output is plain text, one line per action. Big results go to a file (path printed).
Tabs Lighting opened close on done (Claude Code runs it after every reply); keep hands one to the user.
Safety: risky clicks and long URL data to new sites need --yes, lighting abort stops everything.";
const STATS_LINES: usize = 40;
const KEYS_LINES: usize = 60;

pub fn run_one(d: &Daemon, ctx: &mut Ctx, argv: &[String]) -> Result<(Out, String), Fail> {
    let call = parse(argv);
    if call.name.is_empty() {
        return Err(Fail::hint("no command", "lighting help"));
    }
    let asks_help = matches!(call.name.as_str(), "help" | "-h" | "--help");
    if !asks_help && wants_help(argv).is_some() {
        return Ok((Out::Text(help(Some(&call.name))?), "help".into()));
    }
    if call.flags.get("f") == Some(&Flag::On) {
        return Err(Fail::hint(
            "-f needs the words to look for, like -f \"save|draft\"; nothing was done",
            format!("lighting {} ... -f \"<words>\"", call.name),
        ));
    }
    ctx.last_target = None;
    let text = match call.name.as_str() {
        "ping" => "pong".into(),
        "version" => VERSION.into(),
        "stop" => return Ok((Out::Stop("lighting daemon stopped".into()), call.name)),
        "status" => status(d, ctx),
        "log" => log(d, ctx, &call),
        "config" => {
            let (text, changed) = config::command(&d.home, &call.pos)?;
            if changed {
                d.hosts.push_config(&d.home);
            }
            text
        }
        "done" => web::done(d, ctx, &call)?,
        "keep" => web::keep(d, ctx, &call)?,
        "keys" => {
            let text = web::keys::command(&d.home, ctx, &call)?;
            web::capped(d, ctx, &text, "keys", Some(KEYS_LINES), OUT_CHARS)
        }
        _ if asks_help => help(call.pos.first().map(String::as_str))?,
        _ => {
            let out = route::route(d, ctx, &call)?;
            return Ok((route::with_view(d, ctx, &call, out), call.name));
        }
    };
    Ok((Out::Text(text), call.name))
}

fn help(name: Option<&str>) -> Result<String, Fail> {
    let Some(name) = name.map(str::to_lowercase).filter(|n| !n.is_empty()) else { return Ok(HELP.into()) };
    if !built_later(&name) {
        return Err(unknown(&name));
    }
    Ok(about(&name).unwrap_or_else(|| HELP.into()))
}

fn about(name: &str) -> Option<String> {
    let rows = HELP.split("\n\n").nth(1)?;
    let hits: Vec<&str> = rows
        .lines()
        .map(|row| row.get(8..).unwrap_or("").trim())
        .filter(|row| {
            row.split("   ").any(|part| part.trim().split(' ').next().unwrap_or("").split('|').any(|word| word == name))
        })
        .collect();
    (!hits.is_empty()).then(|| hits.join("\n"))
}

fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn status(d: &Daemon, ctx: &Ctx) -> String {
    let up = ago(d.started.elapsed().as_secs());
    let mut lines = vec![format!("lighting {VERSION} | daemon pid {} up {up} | hotkey unavailable", d.pid)];
    web::any_browser(d);
    let mut hosts = d.hosts.alive();
    hosts.sort_by(|a, b| b.focused().max(b.connected).total_cmp(&a.focused().max(a.connected)));
    let pick = web::pick(d);
    for h in &hosts {
        let ext = h.info("ext").unwrap_or_default();
        let in_use = if pick.as_ref().is_some_and(|p| Arc::ptr_eq(p, h)) { " | in use" } else { "" };
        let update = match (!ext.is_empty() && ext != VERSION, d.hosts.reloaded(&h.brand)) {
            (false, _) => String::new(),
            (true, true) => format!(" | updating extension to {VERSION}"),
            (true, false) => " | OUTDATED, run: lighting setup".into(),
        };
        let browser = h.info("browserVersion").unwrap_or_else(|| "?".into());
        let ext = if ext.is_empty() { "?".into() } else { ext };
        lines.push(format!("browser {} {browser} | ext {ext}{in_use}{update}", h.brand));
    }
    if hosts.is_empty() {
        lines.push("browser none connected (extension not loaded or browser closed) -> lighting setup".into());
    }
    if let Some((kind, name)) = &ctx.target {
        lines.push(format!("target {kind} {name}"));
    }
    let cfg = config::load(&d.home);
    let onoff = |key: &str| if config::on(&cfg, key) { "on" } else { "off" };
    let focus = config::text(&cfg, "focus");
    lines.push(format!(
        "pointer {} | cleanup {} | own window {} | focus {} | browser pref {} | out {}",
        onoff("pointer"),
        onoff("cleanup"),
        onoff("window"),
        if focus.is_empty() { "never" } else { focus.as_str() },
        config::text(&cfg, "browser"),
        posix(&d.home.join("out")),
    ));
    lines.extend(media::usage(&d.home));
    lines.push(format!("home {}", posix(&d.home)));
    lines.join("\n")
}

fn log(d: &Daemon, ctx: &mut Ctx, call: &Call) -> String {
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
    web::capped(d, ctx, &text, "log-stats", Some(STATS_LINES), OUT_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_names_one_command_like_2x() {
        assert!(about("tabs").is_some_and(|row| row.starts_with("upload <ref> <file>   tabs [-f x]")));
        assert!(about("dismiss").is_some_and(|row| row.starts_with("fetch <url>")));
        assert!(about("abort").is_some_and(|row| row.starts_with("status ")));
        assert!(about("listen").is_some_and(|row| row.starts_with("listen <link|file|e40>")));
        assert_eq!(about("voice"), None);
        assert!(about("recall").is_some_and(|row| row.starts_with("recall \"words\"")));
        assert!(about("todo").is_some_and(|row| row.starts_with("todo [--all]")));
        assert!(about("memory").is_some_and(|row| row.starts_with("memory [learn|on|off]")));
        assert!(about("wiki").is_some_and(|row| row.starts_with("wiki | wiki accept")));
        assert_eq!(help(Some("voice")).ok().as_deref(), Some(HELP));
        assert_eq!(help(None).ok().as_deref(), Some(HELP));
        assert!(help(Some("zzz")).is_err());
    }
}
