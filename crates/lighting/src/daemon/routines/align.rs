use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::diff::{Matcher, Op};
use super::store::{check_of, slug, text};

pub const ENTRY: &[&str] = &["open", "launch", "focus"];
const SEP: &str = "\u{1f}";
const MATCH: f64 = 0.5;
const KEEP_SLOT: usize = 60;
const LONG: usize = 80;
const MIN_SCORE: f64 = 1.5;
const PARAM_BAD: &[&str] = &["https", "http", "www", "com", "de", "org", "net"];
const FILLER: &[&str] = &[
    "add", "a", "an", "the", "your", "enter", "type", "here", "please", "new", "optional", "required", "ein", "eine",
    "einen", "der", "die", "das", "dein", "deine", "hier", "eingeben", "to", "for", "fuer",
];

static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w+|[^\w\s\x1c-\x1f]").expect("token"));

pub type Pairs = Vec<(String, String)>;

pub fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn put(list: &mut Pairs, k: String, v: String) {
    match list.iter_mut().find(|(x, _)| *x == k) {
        Some(e) => e.1 = v,
        None => list.push((k, v)),
    }
}

pub fn cmd(s: &Value) -> &str {
    s["cmd"].as_str().unwrap_or("")
}

pub fn args(s: &Value) -> Vec<String> {
    s["args"].as_array().into_iter().flatten().map(text).collect()
}

fn py(v: &Value) -> String {
    match v {
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        other => text(other),
    }
}

pub fn replace_word(hay: &str, old: &str, new: &str) -> String {
    if old.is_empty() {
        return hay.to_string();
    }
    let (mut out, mut at, mut from) = (String::new(), 0, 0);
    while let Some(off) = hay[from..].find(old) {
        let (s, e) = (from + off, from + off + old.len());
        let before = hay[..s].chars().next_back().is_some_and(word);
        let after = hay[e..].chars().next().is_some_and(word);
        if before || after {
            from = s + hay[s..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        out.push_str(&hay[at..s]);
        out.push_str(new);
        (at, from) = (e, e);
    }
    out.push_str(&hay[at..]);
    out
}

struct Shape {
    cmd: String,
    toks: Vec<String>,
}

fn shape(s: &Value) -> Shape {
    let mut t = args(s).join(SEP);
    if let Some(flags) = s["flags"].as_object().filter(|f| !f.is_empty()) {
        let mut kv: Vec<(&String, String)> = flags.iter().map(|(k, v)| (k, py(v))).collect();
        kv.sort();
        let shown: Vec<String> = kv.iter().map(|(k, v)| format!("--{k}={v}")).collect();
        t.push_str(SEP);
        t.push_str(&shown.join(" "));
    }
    Shape { cmd: cmd(s).to_string(), toks: TOKEN.find_iter(&t).map(|m| m.as_str().to_lowercase()).collect() }
}

fn sim(a: &Shape, b: &Shape) -> f64 {
    if a.cmd != b.cmd {
        return 0.0;
    }
    if a.toks.is_empty() && b.toks.is_empty() {
        return 1.0;
    }
    2.0 * Matcher::new(&a.toks, &b.toks).matches() as f64 / (a.toks.len() + b.toks.len()) as f64
}

type Path = Vec<(Option<usize>, usize)>;

fn local_align(a: &[Shape], b: &[Shape]) -> (f64, Path) {
    let (n, m) = (a.len(), b.len());
    let mut h = vec![vec![0.0f64; m + 1]; n + 1];
    let mut p = vec![vec![0u8; m + 1]; n + 1];
    let (mut best, mut bi, mut bj) = (0.0, 0, 0);
    for i in 1..=n {
        for j in 1..=m {
            let s = sim(&a[i - 1], &b[j - 1]);
            let diag = h[i - 1][j - 1] + if s >= MATCH { 2.0 * s - 0.6 } else { -1.0 };
            let up = h[i - 1][j] - 0.8;
            let left = h[i][j - 1] - 0.8;
            let v = 0.0f64.max(diag).max(up).max(left);
            h[i][j] = v;
            p[i][j] = match v {
                _ if v == diag && v > 0.0 => 1,
                _ if v == up && v > 0.0 => 2,
                _ if v == left && v > 0.0 => 3,
                _ => 0,
            };
            if v > best {
                (best, bi, bj) = (v, i, j);
            }
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (bi, bj);
    while i > 0 && j > 0 && h[i][j] > 0.0 {
        match p[i][j] {
            1 => {
                pairs.push((Some(i - 1), j - 1));
                (i, j) = (i - 1, j - 1);
            }
            2 => i -= 1,
            3 => {
                pairs.push((None, j - 1));
                j -= 1;
            }
            _ => break,
        }
    }
    pairs.reverse();
    (best, pairs)
}

pub fn slot_pairs(a: &Value, b: &Value) -> Vec<(String, String, String)> {
    let (sa, sb) = (args(a).join(SEP), args(b).join(SEP));
    let ta: Vec<(&str, usize, usize)> = TOKEN.find_iter(&sa).map(|m| (m.as_str(), m.start(), m.end())).collect();
    let tb: Vec<(&str, usize, usize)> = TOKEN.find_iter(&sb).map(|m| (m.as_str(), m.start(), m.end())).collect();
    let la: Vec<String> = ta.iter().map(|t| t.0.to_lowercase()).collect();
    let lb: Vec<String> = tb.iter().map(|t| t.0.to_lowercase()).collect();
    let ops = Matcher::new(&la, &lb).opcodes();
    let starts_word = |t: &str| t.chars().next().is_some_and(word);
    let (mut groups, mut cur): (Vec<[usize; 4]>, Option<[usize; 4]>) = (Vec::new(), None);
    for (k, &(op, i1, i2, j1, j2)) in ops.iter().enumerate() {
        if op != Op::Equal {
            match cur.as_mut() {
                None => cur = Some([i1, i2, j1, j2]),
                Some(c) => (c[1], c[3]) = (i2, j2),
            }
            continue;
        }
        let glue = j2 - j1 <= 2 && tb[j1..j2].iter().all(|t| !starts_word(t.0));
        if let Some(c) = cur.as_mut()
            && glue
            && ops.get(k + 1).is_some_and(|o| o.0 != Op::Equal)
        {
            (c[1], c[3]) = (i2, j2);
            continue;
        }
        groups.extend(cur.take());
    }
    groups.extend(cur);
    let mut out = Vec::new();
    for [i1, i2, j1, j2] in groups {
        if i2 <= i1 || j2 <= j1 {
            continue;
        }
        let va = &sa[ta[i1].1..ta[i2 - 1].2];
        let vb = &sb[tb[j1].1..tb[j2 - 1].2];
        if va.contains(SEP) || vb.contains(SEP) || !vb.chars().any(word) {
            continue;
        }
        let prev = if j1 >= 2 && matches!(tb[j1 - 1].0, ":" | "=") && starts_word(tb[j1 - 2].0) {
            tb[j1 - 2].0.to_string()
        } else {
            String::new()
        };
        out.push((va.to_string(), vb.to_string(), prev));
    }
    out
}

fn substitute(step: &Value, mapping: &Pairs) -> Value {
    if mapping.is_empty() {
        return step.clone();
    }
    let mut out = step.clone();
    let swapped: Vec<String> = args(step)
        .into_iter()
        .map(|a| mapping.iter().fold(a, |acc, (old, new)| replace_word(&acc, old, new)))
        .collect();
    out["args"] = json!(swapped);
    out
}

fn halves(x: &str) -> (&str, &str) {
    x.split_once('=').unwrap_or((x, ""))
}

pub fn typed_pairs(a: &Value, b: &Value) -> Option<Pairs> {
    if cmd(a) != cmd(b) || !matches!(cmd(a), "type" | "fill") {
        return None;
    }
    let (aa, ba) = (args(a), args(b));
    if cmd(a) == "type" {
        if aa.is_empty() || ba.is_empty() || aa[0] != ba[0] {
            return None;
        }
        let (va, vb) = (aa[1..].join(" "), ba[1..].join(" "));
        return Some(if !va.is_empty() && !vb.is_empty() && va != vb { vec![(va, vb)] } else { Vec::new() });
    }
    let (mut la, mut lb) = (Pairs::new(), Pairs::new());
    for x in &aa {
        let (k, v) = halves(x);
        put(&mut la, k.into(), v.into());
    }
    for x in &ba {
        let (k, v) = halves(x);
        put(&mut lb, k.into(), v.into());
    }
    let keys = |l: &Pairs| l.iter().map(|(k, _)| k.clone()).collect::<HashSet<_>>();
    if keys(&la) != keys(&lb) {
        return None;
    }
    let mut out = Pairs::new();
    for (k, va) in &la {
        let vb = &lb.iter().find(|(x, _)| x == k).expect("same keys").1;
        if !va.is_empty() && !vb.is_empty() && va != vb {
            put(&mut out, va.clone(), vb.clone());
        }
    }
    Some(out)
}

pub struct Blk {
    pub score: f64,
    pub pairs: Path,
}

pub fn align(a: &[Value], b: &[Value]) -> Option<Blk> {
    let sa: Vec<Shape> = a.iter().map(shape).collect();
    let sb: Vec<Shape> = b.iter().map(shape).collect();
    let (mut score, mut pairs) = local_align(&sa, &sb);
    if pairs.is_empty() {
        return None;
    }
    let mut mapping = Pairs::new();
    for &(i, j) in &pairs {
        let Some(i) = i else { continue };
        let s = sim(&sa[i], &sb[j]);
        if !(0.3..1.0).contains(&s) {
            continue;
        }
        if let Some(whole) = typed_pairs(&a[i], &b[j]) {
            for (k, v) in whole {
                put(&mut mapping, k, v);
            }
            continue;
        }
        for (va, vb, _) in slot_pairs(&a[i], &b[j]) {
            if va.chars().count() <= KEEP_SLOT && vb.chars().count() <= KEEP_SLOT {
                put(&mut mapping, va, vb);
            }
        }
    }
    let swapped: Vec<Shape> = a.iter().map(|s| shape(&substitute(s, &mapping))).collect();
    if !mapping.is_empty() {
        let (score2, pairs2) = local_align(&swapped, &sb);
        if score2 >= score {
            (score, pairs) = (score2, pairs2);
        }
    }
    if score < MIN_SCORE {
        return None;
    }
    let actions = pairs
        .iter()
        .filter_map(|&(i, j)| i.map(|i| (i, j)))
        .filter(|&(i, j)| sim(&swapped[i], &sb[j]) >= MATCH)
        .filter(|&(_, j)| !matches!(cmd(&b[j]), "wait" | "scroll"))
        .count();
    (actions >= 2).then_some(Blk { score, pairs })
}

pub fn long_literal(step: &Value) -> bool {
    let a = args(step);
    let vals: Vec<&str> = match cmd(step) {
        "type" => a.iter().skip(1).map(String::as_str).collect(),
        "fill" => a.iter().map(|x| halves(x).1).collect(),
        _ => return false,
    };
    vals.iter().any(|v| v.chars().count() > LONG && !v.contains('{'))
}

fn label_name(label: &str) -> String {
    let s = slug(label, 12);
    let words: Vec<&str> = s.split('-').filter(|w| !w.is_empty() && !FILLER.contains(w)).take(2).collect();
    words.join("_")
}

fn param_name(prev: &str, step: &Value, value: &str, used: &HashSet<String>) -> String {
    let a = args(step);
    let mut base = String::new();
    if cmd(step) == "fill" {
        if let Some((label, _)) = a.iter().map(|x| halves(x)).find(|(_, v)| v.contains(value)) {
            base = label_name(label);
        }
    } else if cmd(step) == "type" && a.len() > 1 && a[1..].join(" ").contains(value) {
        base = label_name(&a[0]);
    }
    let digits = !prev.is_empty() && prev.chars().all(|c| c.is_ascii_digit());
    if base.is_empty() && !prev.is_empty() && !PARAM_BAD.contains(&prev.to_lowercase().as_str()) && !digits {
        base = prev.to_lowercase();
    }
    if base.is_empty() {
        base = match cmd(step) {
            "click" => "item",
            "open" => "page",
            "launch" => "app",
            "select" => "option",
            _ => "text",
        }
        .into();
    }
    let mut base: String = base.chars().filter(|c| matches!(c, 'a'..='z' | '0'..='9' | '_')).collect();
    if base.is_empty() {
        base = "text".into();
    }
    if base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert(0, 'p');
    }
    let (mut name, mut n) = (base.clone(), 2);
    while used.contains(&name) {
        name = format!("{base}{n}");
        n += 1;
    }
    name
}

pub fn templatize(steps: &[Value], values: &Pairs) -> Vec<Value> {
    let mut order: Vec<&(String, String)> = values.iter().collect();
    order.sort_by_key(|(v, _)| std::cmp::Reverse(v.chars().count()));
    steps
        .iter()
        .map(|s| {
            let swapped: Vec<String> = args(s)
                .into_iter()
                .map(|a| order.iter().fold(a, |acc, (v, p)| replace_word(&acc, v, &format!("{{{p}}}"))))
                .collect();
            let mut out = s.clone();
            out["args"] = json!(swapped);
            out
        })
        .collect()
}

pub fn clean_step(s: &Value) -> Value {
    let flags = s["flags"].as_object().cloned().unwrap_or_default();
    let mut out = json!({"cmd": cmd(s), "args": args(s), "flags": flags});
    for k in ["role", "optional"] {
        if s.get(k).is_some_and(truthy) {
            out[k] = s[k].clone();
        }
    }
    out
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub fn entry_for(w: &Value) -> Option<Value> {
    let w = w.as_object()?;
    let get = |k: &str| w.get(k).map(text).unwrap_or_default();
    if get("kind") == "app" {
        let exe = get("exe");
        let exe = if exe.to_lowercase().ends_with(".exe") { exe[..exe.len() - 4].to_string() } else { exe };
        if exe.eq_ignore_ascii_case("applicationframehost") {
            return Some(json!({"cmd": "launch", "args": [get("title")], "flags": {}}));
        }
        return Some(json!({"cmd": "focus", "args": [format!("app:{exe}")], "flags": {}}));
    }
    let url = get("url");
    (!url.is_empty()).then(|| json!({"cmd": "open", "args": [url], "flags": {}}))
}

pub fn build(a: &[Value], b: &[Value], start: &Value, blk: &Blk) -> Value {
    let js: Vec<usize> = blk.pairs.iter().map(|p| p.1).collect();
    let (lo, hi) = (js.iter().copied().min().unwrap_or(0), js.iter().copied().max().unwrap_or(0));
    let aligned: HashSet<usize> = blk.pairs.iter().filter(|p| p.0.is_some()).map(|p| p.1).collect();
    let mut steps: Vec<Value> = (lo..=hi)
        .map(|j| {
            let mut s = clean_step(&b[j]);
            if !aligned.contains(&j) {
                s["optional"] = json!(true);
            }
            s
        })
        .collect();
    if !ENTRY.contains(&cmd(&steps[0])) {
        let from = if lo > 0 { &b[lo - 1]["where"] } else { start };
        if let Some(e) = entry_for(from) {
            steps.insert(0, e);
        }
    }
    let (mut values, mut used) = (Pairs::new(), HashSet::new());
    for &(i, j) in &blk.pairs {
        let Some(i) = i else { continue };
        let whole = typed_pairs(&a[i], &b[j]);
        let found: Vec<(String, String, String)> = match &whole {
            Some(w) => w.iter().map(|(va, vb)| (va.clone(), vb.clone(), String::new())).collect(),
            None => slot_pairs(&a[i], &b[j]),
        };
        for (_, vb, prev) in found {
            let n = vb.chars().count();
            if values.iter().any(|(v, _)| *v == vb) || n < 2 || (whole.is_none() && n > KEEP_SLOT) {
                continue;
            }
            let p = param_name(&prev, &b[j], &vb, &used);
            used.insert(p.clone());
            values.push((vb, p));
        }
    }
    let steps = templatize(&steps, &values);
    let mut params = Map::new();
    for (v, p) in &values {
        params.insert(p.clone(), json!(v));
    }
    json!({"steps": steps, "params": params, "check": check_of(b[hi]["where"].as_object(), &values)})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(c: &str, a: &[&str]) -> Value {
        json!({"cmd": c, "args": a, "flags": {}, "where": {"kind": "web", "url": "https://shop.example/x", "title": "Shop"}})
    }

    #[test]
    fn two_runs_with_other_values_become_one_routine_with_params() {
        assert_eq!(replace_word("tea teapot tea", "tea", "{x}"), "{x} teapot {x}");
        let a = vec![
            st("open", &["https://shop.example"]),
            st("type", &["Search", "green tea"]),
            st("click", &["Add to cart"]),
        ];
        let b = vec![
            st("open", &["https://shop.example"]),
            st("type", &["Search", "black coffee"]),
            st("click", &["Add to cart"]),
        ];
        let blk = align(&a, &b).expect("aligned");
        let r = build(&a, &b, &Value::Null, &blk);
        assert_eq!(r["params"], json!({"search": "black coffee"}));
        assert_eq!(r["steps"][1]["args"], json!(["Search", "{search}"]));
        assert_eq!(r["steps"].as_array().unwrap().len(), 3);
        let c = vec![st("open", &["https://other.example"]), st("wait", &["100"])];
        assert!(align(&a, &c).is_none());
        let slots =
            slot_pairs(&json!({"args": ["Search: green tea now"]}), &json!({"args": ["Search: black coffee now"]}));
        assert_eq!(slots, vec![("green tea".into(), "black coffee".into(), "Search".into())]);
    }
}
