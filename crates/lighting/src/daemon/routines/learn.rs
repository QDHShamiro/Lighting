use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use lt_core::{Fail, local_now, local_parts};
use regex::Regex;
use serde_json::{Value, json};

use super::super::Daemon;
use super::super::ctx::{Ctx, now};
use super::align::{ENTRY, Pairs, align, args, build, clean_step, cmd, entry_for, long_literal, put, templatize};
use super::episode::{Episode, episodes, site};
use super::store::{self, check_of, kv, load_all, save, slug, text, usage, words};

const NAV_ONLY: &[&str] =
    &["open", "launch", "focus", "scroll", "wait", "back", "forward", "reload", "tab", "expect", "dismiss"];
const PRUNE_DAYS: f64 = 14.0;
const TAGS: usize = 12;
const PROMPT_BEFORE_S: f64 = 1800.0;
const PROMPT_AFTER_S: f64 = 5.0;
const NAME_WORDS: usize = 4;
const SAVE_WORDS: usize = 6;
const CACHE: usize = 2000;
const RECENT_PAST: usize = 5;

static SLOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[a-z][a-z0-9_]*\}").expect("slot"));
static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:[a-z]+://)?(?:www\.)?([^/.]+)").expect("head"));
static BRACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\}").expect("braces"));
static STAMP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$").expect("stamp"));
static TEMPLATES: LazyLock<Mutex<HashMap<String, Regex>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn template_re(t: &str) -> Regex {
    let mut cache = TEMPLATES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = cache.get(t) {
        return r.clone();
    }
    let build = |named: bool| {
        let (mut rx, mut at) = (String::from("(?si)^"), 0);
        for m in SLOT.find_iter(t) {
            rx.push_str(&regex::escape(&t[at..m.start()]));
            let name = &m.as_str()[1..m.as_str().len() - 1];
            rx.push_str(&if named { format!("(?P<{name}>.+?)") } else { "(.+?)".to_string() });
            at = m.end();
        }
        rx.push_str(&regex::escape(&t[at..]));
        rx.push('$');
        Regex::new(&rx)
    };
    let r = build(true).or_else(|_| build(false)).expect("template");
    if cache.len() >= CACHE {
        cache.clear();
    }
    cache.insert(t.to_string(), r.clone());
    r
}

pub fn match_step(tpl: &Value, step: &Value) -> Option<BTreeMap<String, String>> {
    let (ta, sa) = (args(tpl), args(step));
    if cmd(tpl) != cmd(step) || ta.len() != sa.len() {
        return None;
    }
    let mut got = BTreeMap::new();
    for (t, s) in ta.iter().zip(&sa) {
        let re = template_re(t);
        let m = re.captures(s)?;
        for name in re.capture_names().flatten() {
            if let Some(v) = m.name(name) {
                got.insert(name.to_string(), v.as_str().to_string());
            }
        }
    }
    Some(got)
}

fn needed(r: &Value) -> Vec<Value> {
    r["steps"].as_array().into_iter().flatten().filter(|s| s["optional"] != true).cloned().collect()
}

fn covers(r: &Value, steps: &[Value]) -> Option<BTreeMap<String, String>> {
    let need = needed(r);
    let (mut k, mut got) = (0, BTreeMap::new());
    for s in steps {
        if k < need.len()
            && let Some(m) = match_step(&need[k], s)
        {
            got.extend(m);
            k += 1;
        }
    }
    (k == need.len()).then_some(got)
}

fn same_shape(r: &Value, steps: &[Value], start: &Value) -> bool {
    let Some(first) = r["steps"].get(0) else { return false };
    let a: Vec<&str> = r["steps"].as_array().into_iter().flatten().filter(|s| s["optional"] != true).map(cmd).collect();
    let b: Vec<&str> = steps.iter().map(cmd).collect();
    a == b && site(&first["where"]) == site(start)
}

fn signature(steps: &[Value]) -> Vec<(String, Vec<String>)> {
    steps.iter().filter(|s| s["optional"] != true).map(|s| (cmd(s).to_string(), args(s))).collect()
}

fn compact(steps: Vec<Value>, rs: &[Value]) -> Vec<Value> {
    for r in rs {
        let need = needed(r);
        let n = need.len();
        if n < 2 || n >= steps.len() || r["flaky"] == true || need.iter().any(|s| cmd(s) == "run") {
            continue;
        }
        let params: Vec<&String> = r["params"].as_object().map(|p| p.keys().collect()).unwrap_or_default();
        for i in 0..=steps.len() - n {
            let Some(got) = covers(r, &steps[i..i + n]) else { continue };
            if !params.iter().all(|p| got.contains_key(*p)) {
                continue;
            }
            let mut call_args = vec![text(&r["name"])];
            call_args.extend(params.iter().map(|p| kv(p, &got[*p])));
            let call = json!({"cmd": "run", "args": call_args, "flags": {}, "where": steps[i + n - 1]["where"]});
            let mut next = steps[..i].to_vec();
            next.push(call);
            next.extend_from_slice(&steps[i + n..]);
            return compact(next, rs);
        }
    }
    steps
}

fn stamp(epoch: f64) -> String {
    let [y, mo, d, h, mi, _] = local_parts(epoch);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

pub fn prune(home: &Path) {
    let limit = stamp(now() - PRUNE_DAYS * 86_400.0);
    for r in load_all(home) {
        let created = text(&r["created"]);
        let ran = r["stats"]["runs"].as_u64().unwrap_or(0) > 0;
        if r["source"] != "learned" || ran || !STAMP.is_match(&created) {
            continue;
        }
        if created < limit {
            let _ = fs::remove_file(store::file(home, &text(&r["name"])));
        }
    }
}

fn new_routine(body: Value, source: &str, cost: usize) -> Value {
    let seen = if source == "learned" { 2 } else { 1 };
    json!({"name": "", "version": 1, "source": source, "created": local_now()[..16].to_string(),
           "params": body["params"], "steps": body["steps"], "check": body["check"], "cost": cost,
           "stats": {"runs": 0, "ok": 0, "fail": 0, "ms": 0, "saved": 0, "seen": seen, "recent": []},
           "flaky": false})
}

pub fn tags_for(prompts: &[(f64, Vec<String>)], t0: f64, params: &Value) -> Vec<String> {
    let vals: Vec<String> = params.as_object().into_iter().flatten().flat_map(|(_, v)| words(&text(v))).collect();
    let best = prompts.iter().rfind(|(t, _)| *t <= t0 + PROMPT_AFTER_S && t0 - t < PROMPT_BEFORE_S);
    let Some((_, best)) = best else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for w in best {
        if !vals.contains(w) && !out.contains(w) {
            out.push(w.clone());
        }
    }
    out.truncate(TAGS);
    out
}

fn auto_name(home: &Path, r: &Value) -> String {
    let steps = r["steps"].as_array().cloned().unwrap_or_default();
    let first = steps.first().cloned().unwrap_or(Value::Null);
    let a = args(&first);
    let head = if cmd(&first) == "open" {
        a.first().and_then(|x| HEAD.captures(x)).map(|m| m[1].to_string()).unwrap_or_default()
    } else {
        a.first().map(|x| x.replace("app:", "").split(':').next().unwrap_or("").to_string()).unwrap_or_default()
    };
    let mut tail = String::new();
    for s in steps.iter().rev() {
        if matches!(cmd(s), "click" | "type" | "fill" | "select" | "check")
            && let Some(x) = args(s).first()
        {
            tail = BRACES.replace_all(x.split('=').next().unwrap_or(""), " ").to_string();
            break;
        }
    }
    let low = head.to_lowercase();
    let said: Vec<String> = r["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .map(text)
        .filter(|t| !t.is_empty() && t.chars().all(char::is_alphabetic) && !t.contains(&low) && !low.contains(t))
        .collect();
    if !said.is_empty() {
        tail = said.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
    }
    let name = slug(&format!("{head} {tail}"), NAME_WORDS);
    let base = if name.is_empty() { "routine".to_string() } else { name };
    let (mut name, mut n) = (base.clone(), 2);
    while store::file(home, &name).exists() {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

fn repair(d: &Daemon, ep: &Value, rs: Vec<Value>) -> Result<Option<Value>, Fail> {
    let via = text(&ep["via"]);
    let Some(mut r) = rs.into_iter().find(|x| text(&x["name"]) == via) else { return Ok(None) };
    let raw = ep["steps"].as_array().cloned().unwrap_or_default();
    if ep["failed"].is_null() || raw.is_empty() {
        return Ok(None);
    }
    let steps: Vec<Value> = raw.iter().map(clean_step).collect();
    let mut values = Pairs::new();
    for (p, v) in ep["params"].as_object().into_iter().flatten() {
        if !text(v).is_empty() {
            put(&mut values, text(v), p.clone());
        }
    }
    r["steps"] = json!(templatize(&steps, &values));
    r["check"] = check_of(raw[raw.len() - 1]["where"].as_object(), &values);
    r["version"] = json!(r["version"].as_u64().unwrap_or(1) + 1);
    r["stats"]["recent"] = json!([]);
    r["flaky"] = json!(false);
    save(&d.home, &r)?;
    d.hosts.event(format!("repaired routine {via} (v{}) from your fix", r["version"]));
    Ok(Some(r))
}

pub fn learn(d: &Daemon, ctx: &Ctx, ep: &Value, past: &[Value]) -> Result<Option<Value>, Fail> {
    let rs = load_all(&d.home);
    if !ep["via"].is_null() {
        return repair(d, ep, rs);
    }
    let steps = ep["steps"].as_array().cloned().unwrap_or_default();
    if steps.iter().any(|s| super::align::truthy(&s["flags"]["yes"]))
        || steps.iter().all(|s| NAV_ONLY.contains(&cmd(s)))
    {
        return Ok(None);
    }
    for r in &rs {
        if covers(r, &steps).is_some() || same_shape(r, &steps, &ep["start"]) {
            let mut r = r.clone();
            r["stats"]["seen"] = json!(r["stats"]["seen"].as_u64().unwrap_or(0) + 1);
            save(&d.home, &r)?;
            return Ok(None);
        }
    }
    let mut best: Option<(super::align::Blk, &Value)> = None;
    for old in past.iter().rev() {
        if super::align::truthy(&old["via"]) || old["lossy"] == true {
            continue;
        }
        let old_steps = old["steps"].as_array().cloned().unwrap_or_default();
        if let Some(blk) = align(&old_steps, &steps)
            && best.as_ref().is_none_or(|(b, _)| blk.score > b.score)
        {
            best = Some((blk, old));
        }
    }
    let Some((blk, old)) = best else { return Ok(None) };
    let old_steps = old["steps"].as_array().cloned().unwrap_or_default();
    let mut body = build(&old_steps, &steps, &ep["start"], &blk);
    let made = compact(body["steps"].as_array().cloned().unwrap_or_default(), &rs);
    if made.iter().filter(|s| s["optional"] != true).all(|s| NAV_ONLY.contains(&cmd(s)))
        || made.iter().any(long_literal)
        || rs.iter().any(|r| signature(r["steps"].as_array().map_or(&[][..], Vec::as_slice)) == signature(&made))
    {
        return Ok(None);
    }
    body["steps"] = json!(made);
    let cost = (old["cost"].as_u64().unwrap_or(0) as usize + ep["cost"].as_u64().unwrap_or(0) as usize) / 2;
    let mut r = new_routine(body, "learned", cost);
    let t0 = ep["t0"].as_f64().filter(|t| *t != 0.0).unwrap_or_else(now);
    r["tags"] = json!(tags_for(&ctx.prompts, t0, &r["params"]));
    r["name"] = json!(auto_name(&d.home, &r));
    save(&d.home, &r)?;
    prune(&d.home);
    d.hosts.event(format!(
        "learned routine {} from 2 runs -> next time: {}",
        text(&r["name"]),
        usage(&r, &BTreeMap::new())
    ));
    Ok(Some(r))
}

pub struct Task<'a> {
    pub steps: &'a [Value],
    pub cost: usize,
    pub start: &'a Value,
}

pub fn from_steps(
    home: &Path,
    name: &str,
    pairs: &[String],
    source: &str,
    task: &Task,
    prompts: &[(f64, Vec<String>)],
) -> Result<Value, Fail> {
    let (raw, cost, start) = (task.steps, task.cost, task.start);
    let mut steps: Vec<Value> = raw.iter().map(clean_step).collect();
    if steps.first().is_some_and(|s| !ENTRY.contains(&cmd(s)))
        && let Some(e) = entry_for(start)
    {
        steps.insert(0, e);
    }
    let mut values = Pairs::new();
    for p in pairs {
        if let Some((k, v)) = p.split_once('=').filter(|(_, v)| !v.is_empty()) {
            let k: String = k.to_lowercase().chars().filter(|c| matches!(c, 'a'..='z' | '0'..='9' | '_')).collect();
            put(&mut values, v.to_string(), if k.is_empty() { "text".into() } else { k });
        }
    }
    let mut params = serde_json::Map::new();
    for (v, p) in &values {
        params.insert(p.clone(), json!(v));
    }
    let check = check_of(raw.last().and_then(|s| s["where"].as_object()), &values);
    let body = json!({"steps": templatize(&steps, &values), "params": params, "check": check});
    let mut r = new_routine(body, source, cost);
    let t0 = raw.first().and_then(|s| s["t"].as_f64()).filter(|t| *t != 0.0).unwrap_or_else(now);
    r["tags"] = json!(tags_for(prompts, t0, &r["params"]));
    let wanted = slug(name, SAVE_WORDS);
    r["name"] = json!(if wanted.is_empty() { name.to_string() } else { wanted });
    save(home, &r)?;
    Ok(r)
}

pub fn save_current(
    d: &Daemon,
    ctx: &mut Ctx,
    name: &str,
    pairs: &[String],
    last: Option<&str>,
) -> Result<String, Fail> {
    let (mut steps, mut cost, mut start) = (ctx.ep.steps.clone(), ctx.ep.cost, ctx.ep.start.clone());
    if steps.is_empty() {
        let past: Vec<Value> = episodes(&d.home, RECENT_PAST).into_iter().filter(|e| e["via"].is_null()).collect();
        let Some(e) = past.last() else {
            return Err(Fail::hint("no finished task to save", "do the task with lighting first"));
        };
        steps = e["steps"].as_array().cloned().unwrap_or_default();
        cost = e["cost"].as_u64().unwrap_or(0) as usize;
        start = e["start"].clone();
    }
    if let Some(n) = last {
        let n: usize =
            n.parse().map_err(|_| Fail::hint("--last needs a number", "lighting routine save NAME --last 3"))?;
        if n > 0 {
            steps = steps[steps.len().saturating_sub(n)..].to_vec();
        }
    }
    let task = Task { steps: &steps, cost, start: &start };
    let r = from_steps(&d.home, name, pairs, "saved", &task, &ctx.prompts)?;
    ctx.ep = Episode::default();
    Ok(format!(
        "saved routine {} ({} steps) -> {}",
        text(&r["name"]),
        r["steps"].as_array().map_or(0, Vec::len),
        usage(&r, &BTreeMap::new())
    ))
}

#[cfg(test)]
mod tests {
    use super::super::align::{Blk, truthy};
    use super::*;

    #[test]
    fn templates_match_steps_and_give_the_values() {
        let tpl = json!({"cmd": "type", "args": ["Search", "{q} now"]});
        let step = json!({"cmd": "type", "args": ["search", "green tea now"]});
        assert_eq!(match_step(&tpl, &step).unwrap()["q"], "green tea");
        assert!(match_step(&tpl, &json!({"cmd": "click", "args": ["Search", "x now"]})).is_none());
        let twice = json!({"cmd": "open", "args": ["{a}/{a}"]});
        assert!(match_step(&twice, &json!({"cmd": "open", "args": ["x/y"]})).unwrap().is_empty());
        let prompts = vec![(100.0, vec!["order".to_string(), "green".to_string(), "pizza".to_string()])];
        assert_eq!(tags_for(&prompts, 101.0, &json!({"q": "green"})), vec!["order", "pizza"]);
        assert!(tags_for(&prompts, 3000.0, &json!({})).is_empty());
    }

    #[test]
    #[ignore]
    fn parity_with_2x_on_real_episodes() {
        let (src, out) = (std::env::var("LT_PARITY_IN").unwrap(), std::env::var("LT_PARITY_OUT").unwrap());
        let eps: Vec<Value> = std::fs::read_to_string(src)
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|e| !truthy(&e["via"]) && e["lossy"] != true)
            .collect();
        let steps = |e: &Value| e["steps"].as_array().cloned().unwrap_or_default();
        let mut rows = Vec::new();
        for i in 1..eps.len() {
            let mut best: Option<(Blk, &Value)> = None;
            for old in eps[i.saturating_sub(40)..i].iter().rev() {
                if let Some(blk) = align(&steps(old), &steps(&eps[i]))
                    && best.as_ref().is_none_or(|(b, _)| blk.score > b.score)
                {
                    best = Some((blk, old));
                }
            }
            let Some((blk, old)) = best else {
                rows.push(json!({"i": i, "none": true}).to_string());
                continue;
            };
            let body = build(&steps(old), &steps(&eps[i]), &eps[i]["start"], &blk);
            let pairs: Vec<Value> = blk.pairs.iter().map(|(a, b)| json!([a, b])).collect();
            let long = body["steps"].as_array().unwrap().iter().any(long_literal);
            let score = (blk.score * 1e6).round() / 1e6;
            rows.push(json!({"i": i, "score": score, "pairs": pairs, "body": body, "long": long}).to_string());
        }
        std::fs::write(out, rows.join("\n") + "\n").unwrap();
    }
}
