use std::sync::{Mutex, PoisonError};

use lt_core::{Call, Fail, local_parts};
use serde_json::{Map, Value, json};

use super::ctx::{Ctx, now};
use super::{Daemon, Out, routines, web};

#[cfg(windows)]
const MINUTES: u64 = 15;
const NOTES: usize = 5;
const WEB_CMDS: &[&str] = &["click", "fill", "press", "scroll", "reload", "type"];
const USAGE: &str = "wisply record start <name> | stop [name=value ...] | cancel | status";

struct Rec {
    name: String,
    t0: f64,
    web: bool,
    #[cfg(windows)]
    desk: Option<super::desk::Recorder>,
}

static REC: Mutex<Option<Rec>> = Mutex::new(None);

fn take() -> Option<Rec> {
    REC.lock().unwrap_or_else(PoisonError::into_inner).take()
}

pub fn tidy(steps: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for s in steps {
        let cmd = s["cmd"].as_str().unwrap_or("").to_string();
        let args = s["args"].as_array().cloned().unwrap_or_default();
        let mut flags = s["flags"].as_object().cloned().unwrap_or_default();
        if WEB_CMDS.contains(&cmd.as_str()) && !flags.contains_key("on") {
            flags.insert("on".into(), json!("web"));
        }
        if flags.get("secret").is_some_and(routines::truthy) {
            flags.shift_remove("secret");
        }
        let step = json!({"t": s["t"].as_f64().unwrap_or(0.0), "cmd": cmd, "args": args, "flags": flags});
        if let Some(last) = out.last_mut() {
            if cmd == "open" && last["cmd"] == "open" {
                *last = step;
                continue;
            }
            if cmd == "focus" && last["cmd"] == "focus" && last["args"] == step["args"] {
                continue;
            }
        }
        out.push(step);
    }
    out
}

fn default_name(at: f64) -> String {
    let [_, mo, d, h, mi, _] = local_parts(at);
    format!("rec-{mo:02}{d:02}-{h:02}{mi:02}")
}

#[cfg(windows)]
fn desk_start(c: &Call) -> Result<Option<super::desk::Recorder>, Fail> {
    if c.on("web") {
        return Ok(None);
    }
    let minutes = c.val("max").and_then(|m| m.parse().ok()).unwrap_or(MINUTES);
    super::desk::Recorder::start(minutes, c.on("injected")).map(Some)
}

fn start(d: &Daemon, ctx: &mut Ctx, c: &Call, rest: &[String]) -> Result<String, Fail> {
    if let Some(r) = REC.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        return Err(Fail::hint(format!("already recording \"{}\"", r.name), "wisply record stop"));
    }
    let name = rest.first().cloned().unwrap_or_else(|| default_name(now()));
    let web_line = if !c.on("desktop") && web::any_browser(d) {
        web::call(d, ctx, "record-start", Map::new(), web::CALL_TIMEOUT)
            .ok()
            .map(|m| m.get("out").and_then(Value::as_str).unwrap_or("").to_string())
    } else {
        None
    };
    #[cfg(windows)]
    let desk = desk_start(c)?;
    #[cfg(windows)]
    let has_desk = desk.is_some();
    #[cfg(not(windows))]
    let has_desk = false;
    if web_line.is_none() && !has_desk {
        return Err(Fail::hint("nothing to record", "open a page first (wisply open <url>) or leave out --web"));
    }
    let mut parts = Vec::new();
    if let Some(w) = &web_line {
        parts.push(format!("browser tab {w}"));
    }
    if has_desk {
        parts.push("desktop apps".to_string());
    }
    *REC.lock().unwrap_or_else(PoisonError::into_inner) = Some(Rec {
        name: name.clone(),
        t0: now(),
        web: web_line.is_some(),
        #[cfg(windows)]
        desk,
    });
    Ok(format!(
        "recording \"{name}\" ({}) -> do the steps now, then: wisply record stop [name=value ...]",
        parts.join(" + ")
    ))
}

fn stop(d: &Daemon, ctx: &mut Ctx, rest: &[String], keep: bool) -> Result<String, Fail> {
    let Some(rec) = take() else { return Err(Fail::hint("not recording", "wisply record start <name>")) };
    let mut steps: Vec<Value> = Vec::new();
    if rec.web
        && let Ok(m) = web::call(d, ctx, "record-stop", Map::new(), web::CALL_TIMEOUT)
    {
        steps.extend(m.get("steps").and_then(Value::as_array).cloned().unwrap_or_default());
    }
    #[cfg(windows)]
    if let Some(desk) = rec.desk {
        steps.extend(desk.stop());
    }
    if !keep {
        return Ok("recording cancelled, nothing saved".into());
    }
    steps.sort_by(|a, b| a["t"].as_f64().unwrap_or(0.0).total_cmp(&b["t"].as_f64().unwrap_or(0.0)));
    let is_note = |s: &Value| s["cmd"] == "#";
    let notes: Vec<String> =
        steps.iter().filter(|s| is_note(s)).filter_map(|s| s["args"][0].as_str().map(String::from)).collect();
    let real: Vec<Value> = steps.into_iter().filter(|s| !is_note(s)).collect();
    let steps = tidy(&real);
    if steps.is_empty() {
        return Ok(format!("recorded nothing{}", notes.first().map_or(String::new(), |n| format!(" ({n})"))));
    }
    let pairs: Vec<String> = rest.iter().filter(|p| p.contains('=')).cloned().collect();
    let name = rest.iter().find(|p| !p.contains('=')).cloned().unwrap_or(rec.name);
    let mut text = routines::recorded(d, ctx, &name, &steps, &pairs)?;
    for n in notes.iter().take(NOTES) {
        text += &format!("\n! {n}");
    }
    Ok(text)
}

fn status() -> String {
    let guard = REC.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(r) = guard.as_ref() else { return "not recording".into() };
    #[cfg(windows)]
    let n = r.desk.as_ref().map_or(0, super::desk::Recorder::count);
    #[cfg(not(windows))]
    let n = 0;
    format!("recording \"{}\" for {}s, {n} app steps so far", r.name, (now() - r.t0) as i64)
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let sub = c.pos.first().map_or_else(|| "status".to_string(), |p| p.to_lowercase());
    let rest = c.pos.get(1..).unwrap_or(&[]);
    let text = match sub.as_str() {
        "start" => start(d, ctx, c, rest)?,
        "stop" => stop(d, ctx, rest, true)?,
        "cancel" => stop(d, ctx, rest, false)?,
        "status" => status(),
        _ => return Err(Fail::hint(format!("unknown record command {sub}"), USAGE)),
    };
    Ok(Out::Text(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_steps_are_tidied_like_2x() {
        let steps = vec![
            json!({"t": 1.0, "cmd": "open", "args": ["https://a.example"], "flags": {}}),
            json!({"t": 2.0, "cmd": "open", "args": ["https://b.example"], "flags": {}}),
            json!({"t": 3.0, "cmd": "fill", "args": ["Name", "x"], "flags": {"secret": true}}),
            json!({"t": 4.0, "cmd": "focus", "args": ["app:Notepad"], "flags": {"on": "app"}}),
            json!({"t": 5.0, "cmd": "focus", "args": ["app:Notepad"], "flags": {"on": "app"}}),
            json!({"t": 6.0, "cmd": "click", "args": ["Save"], "flags": {"on": "app"}}),
        ];
        let out = tidy(&steps);
        let cmds: Vec<&str> = out.iter().map(|s| s["cmd"].as_str().unwrap()).collect();
        assert_eq!(cmds, ["open", "fill", "focus", "click"]);
        assert_eq!(out[0]["args"][0], "https://b.example");
        assert_eq!(out[1]["flags"], json!({"on": "web"}));
        assert_eq!(out[3]["flags"], json!({"on": "app"}));
        assert!(default_name(1.9e9).starts_with("rec-"));
    }
}
