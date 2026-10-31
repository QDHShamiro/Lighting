use std::sync::LazyLock;

use lt_core::{Call, Fail, Flag, parse};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::super::ctx::{Ctx, now};
use super::super::{Daemon, Out, config, route, web};
use super::{ELEMENT, LEARN_CMDS, count, learn, norm, on, save_soon, site, step_labels, with};

const TARGETED: &[&str] = &["click", "type", "select", "check", "hover"];
const BOOKS: usize = 6;
const REFS_MAX: usize = 3000;
const REFS_DROP: usize = 1000;
const FIX_SURE: u64 = 2;
const FIX_WINDOW_S: f64 = 90.0;
const FIX_ALIKE: f64 = 0.6;
const ALIKE_CHARS: usize = 200;
const WORD_MIN: usize = 3;

static HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[(t\d+)").expect("header"));
static GONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"is gone").expect("gone"));
static MISS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"nothing matches|nothing called|no field called").expect("miss"));
static OK_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ok ((?:f\d+\.)?e\d+)\b").expect("ok ref"));

pub struct Miss {
    key: String,
    cmd: String,
    asked: String,
    t: f64,
}

fn scope_of(ctx: &Ctx) -> String {
    ctx.place.as_ref().and_then(|w| w.get("tab")).and_then(Value::as_str).unwrap_or("").to_string()
}

fn remember_refs(ctx: &mut Ctx, out: &str) {
    if !out.contains('"') {
        return;
    }
    let lines: Vec<&str> = out.split('\n').collect();
    let scope = HEADER.captures(lines[0]).map(|m| m[1].to_string()).unwrap_or_else(|| scope_of(ctx));
    if scope.is_empty() {
        return;
    }
    let mut refs: Option<Map<String, Value>> = None;
    for line in lines {
        let Some(e) = ELEMENT.captures(line.trim_start_matches(['+', ' '])) else { continue };
        if e[1].contains('.') || !e[1].starts_with('e') {
            continue;
        }
        let refs = refs.get_or_insert_with(|| {
            let at = ctx.refnames.iter().position(|(s, _)| *s == scope);
            at.map(|i| ctx.refnames.remove(i).1).unwrap_or_default()
        });
        let name = e[3].to_string();
        let cut = name.ends_with("...") || name.ends_with('…');
        refs.insert(e[1].to_string(), json!([&e[2], name, cut]));
    }
    let Some(mut refs) = refs else { return };
    if refs.len() > REFS_MAX {
        refs = refs.into_iter().skip(REFS_DROP).collect();
    }
    ctx.refnames.push((scope, refs));
    if ctx.refnames.len() > BOOKS {
        let extra = ctx.refnames.len() - BOOKS;
        ctx.refnames.drain(..extra);
    }
}

fn ref_name(ctx: &Ctx, r: &str) -> Option<(String, String)> {
    let scope = scope_of(ctx);
    let got = ctx.refnames.iter().find(|(s, _)| *s == scope)?.1.get(r)?;
    let name = got[1].as_str().unwrap_or("");
    if got[2] == true || name.trim().is_empty() {
        return None;
    }
    Some((got[0].as_str().unwrap_or("").to_string(), name.to_string()))
}

fn common(a: &[char], b: &[char]) -> usize {
    let (mut best, mut at_a, mut at_b) = (0, 0, 0);
    for i in 0..a.len() {
        for j in 0..b.len() {
            let k = a[i..].iter().zip(&b[j..]).take_while(|(x, y)| x == y).count();
            if k > best {
                (best, at_a, at_b) = (k, i, j);
            }
        }
    }
    if best == 0 {
        return 0;
    }
    best + common(&a[..at_a], &b[..at_b]) + common(&a[at_a + best..], &b[at_b + best..])
}

fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| w.chars().count() >= WORD_MIN).collect()
}

fn alike(miss: &str, label: &str) -> bool {
    let (asked, label) = (norm(miss), norm(label));
    let asked = asked.trim_matches(['"', '\'']);
    let theirs = words(&label);
    if words(asked).iter().any(|w| theirs.contains(w)) {
        return true;
    }
    let a: Vec<char> = asked.chars().take(ALIKE_CHARS).collect();
    let b: Vec<char> = label.chars().take(ALIKE_CHARS).collect();
    let total = a.len() + b.len();
    total > 0 && 2.0 * common(&a, &b) as f64 / total as f64 >= FIX_ALIKE
}

fn fix_for(d: &Daemon, ctx: &Ctx, cmd: &str, text: &str) -> Option<(String, u64)> {
    let key = site(ctx);
    let k = format!("{cmd}|{}", norm(text));
    with(&d.home, |s| {
        let f = s.data["fixes"].get(&key)?.get(&k)?;
        Some((f[0].as_str().unwrap_or("").to_string(), f[1].as_u64().unwrap_or(0)))
    })
}

pub fn argv_of(c: &Call) -> Vec<String> {
    let mut out = vec![c.name.clone()];
    out.extend(c.pos.iter().cloned());
    for (k, v) in &c.flags {
        out.push(format!("--{k}"));
        if let Flag::Val(v) = v {
            out.push(v.clone());
        }
    }
    out
}

pub fn retry(d: &Daemon, ctx: &mut Ctx, c: &Call, fail: &Fail) -> Option<(Out, Vec<String>)> {
    let first = c.pos.first()?;
    if !TARGETED.contains(&c.name.as_str()) || !on(d) {
        return None;
    }
    let text = fail.text();
    let kind = web::ref_kind(first);
    if kind == Some("e") && !first.contains('.') && GONE.is_match(&text) {
        let (role, label) = ref_name(ctx, first)?;
        let mut again = c.clone();
        again.pos[0] = label.clone();
        again.flags.insert("role".into(), Flag::Val(role.clone()));
        let out = route::route(d, ctx, &again).ok()?;
        ctx.last_target = Some(json!({"name": label, "role": role}));
        count(&d.home, "regained", 1);
        ctx.note(&format!("{first} was gone (the page changed); used the same {role} \"{label}\" instead"));
        return Some((out, argv_of(&again)));
    }
    if kind.is_none() && MISS.is_match(&text) {
        let (to, n) = fix_for(d, ctx, &c.name, first)?;
        if n < FIX_SURE || !alike(first, &to) {
            return None;
        }
        let mut again = c.clone();
        again.pos[0] = to.clone();
        let out = route::route(d, ctx, &again).ok()?;
        count(&d.home, "fixed", 1);
        ctx.note(&format!(
            "nothing called \"{first}\" here; used \"{to}\" (it worked here {n} times after the same miss)"
        ));
        return Some((out, argv_of(&again)));
    }
    None
}

pub fn hint_for(d: &Daemon, ctx: &Ctx, c: &Call, fail: Fail) -> Fail {
    let Some(first) = c.pos.first() else { return fail };
    if !TARGETED.contains(&c.name.as_str()) || web::ref_kind(first).is_some() || !MISS.is_match(&fail.text()) {
        return fail;
    }
    match fix_for(d, ctx, &c.name, first) {
        Some((to, _)) => {
            let msg = format!("{} (worked here before after the same miss: \"{to}\")", fail.msg);
            match fail.hint {
                Some(h) => Fail::hint(msg, h),
                None => Fail::new(msg),
            }
        }
        None => fail,
    }
}

fn target_labels(ctx: &Ctx, c: &Call, out: &str) -> Vec<(String, Value)> {
    if c.name == "fill" {
        return c.pos.iter().filter_map(|p| p.split_once('=').map(|(k, _)| (k.to_string(), Value::Null))).collect();
    }
    let t = ctx.last_target.clone().unwrap_or(Value::Null);
    if let Some(name) = t["name"].as_str().filter(|n| !n.is_empty()) {
        return vec![(name.to_string(), t["role"].clone())];
    }
    if let Some(m) = OK_REF.captures(out)
        && let Some((role, name)) = ref_name(ctx, &m[1])
    {
        return vec![(name, json!(role))];
    }
    step_labels(&c.name, &c.pos).into_iter().map(|l| (l, Value::Null)).collect()
}

pub fn observe(d: &Daemon, ctx: &mut Ctx, argv: &[String], out: &str, ok: bool) {
    if ok {
        remember_refs(ctx, out);
    }
    let c = parse(argv);
    let Some(first) = c.pos.first() else { return };
    if !LEARN_CMDS.contains(&c.name.as_str()) || !on(d) {
        return;
    }
    let key = site(ctx);
    let host = key.split('/').next().unwrap_or("");
    let blocked = config::blocklist(&d.home).iter().any(|b| host == b || host.ends_with(&format!(".{b}")));
    if key.is_empty() || blocked {
        return;
    }
    let t = now();
    if !ok {
        if TARGETED.contains(&c.name.as_str()) && web::ref_kind(first).is_none() && MISS.is_match(out) {
            ctx.brain_miss = Some(Miss { key, cmd: c.name.clone(), asked: first.clone(), t });
        }
        return;
    }
    let labels = target_labels(ctx, &c, out);
    let miss = ctx.brain_miss.take();
    with(&d.home, |s| {
        for (label, role) in &labels {
            learn(&mut s.data, &key, label, role, t);
        }
        if let (Some(m), Some((first_label, _))) = (miss, labels.first())
            && m.key == key
            && m.cmd == c.name
            && t - m.t < FIX_WINDOW_S
            && norm(first_label) != norm(&m.asked)
        {
            let fixes = s.data["fixes"].as_object_mut().expect("fixes").entry(key.clone()).or_insert_with(|| json!({}));
            let k = format!("{}|{}", c.name, norm(&m.asked));
            let old = &fixes[&k];
            let n = if old[0].as_str() == Some(first_label.as_str()) { old[1].as_u64().unwrap_or(0) + 1 } else { 1 };
            fixes[&k] = json!([first_label, n, t]);
        }
        s.dirty = true;
        save_soon(s);
    });
}

#[cfg(test)]
mod tests {
    use super::alike;

    #[test]
    fn a_learned_fix_is_only_used_for_an_alike_name() {
        for (miss, label) in [
            ("Save", "Save changes"),
            ("\"Sign in\"", "Sign-in"),
            ("log in", "Login"),
            ("Einstellungen", "Einstellung"),
            ("submit", "Sumbit"),
            ("accept all", "Accept"),
        ] {
            assert!(alike(miss, label), "{miss} ~ {label}");
        }
        for (miss, label) in [("Delete", "Cancel"), ("OK", "Abbrechen"), ("weiter", "Next"), ("Pay now", "Log out")] {
            assert!(!alike(miss, label), "{miss} !~ {label}");
        }
    }
}
