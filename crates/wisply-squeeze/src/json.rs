use crate::pat::{IMPORTANT, NUMS, QUIET};
use crate::text::{clip, head, len, lstrip, strip};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;
use std::iter;

const ITEMS: usize = 6;
const FIRST: usize = 3;
const LAST: usize = 1;
const ITEM_CHARS: usize = 600;
const PICKS: usize = 6;
const STR: usize = 300;
const SIGNAL: usize = 120;
const SAME_MAX: usize = 5000;
const SHORT: usize = 40;
const PICK_CHARS: usize = 4000;
const SCAN_ROWS: usize = 50;
const OUTLIER_ROWS: usize = 10;
const WORD_ROWS: usize = 20;
const WORD_KINDS: usize = 5;
const COMMON: f64 = 0.8;
const SIGMAS: f64 = 3.0;

pub(crate) fn parse(text: &str) -> Option<(Value, bool)> {
    let s = strip(text);
    if (s.starts_with('[') || s.starts_with('{'))
        && let Ok(v) = serde_json::from_str::<Value>(s)
    {
        return Some((v, false));
    }
    let rows: Vec<&str> = s.split('\n').filter(|r| !strip(r).is_empty()).collect();
    if rows.len() < 2 || !rows.iter().take(SCAN_ROWS).all(|r| lstrip(r).starts_with('{')) {
        return None;
    }
    let mut items = Vec::with_capacity(rows.len());
    let mut bad = 0;
    for row in &rows {
        match serde_json::from_str::<Value>(row) {
            Ok(v) => items.push(v),
            Err(_) => bad += 1,
        }
    }
    (bad <= rows.len() / 10).then_some((Value::Array(items), true))
}

pub(crate) fn squeeze(obj: &Value, rows: bool) -> Vec<String> {
    let small = crush(obj);
    match &small {
        Value::Array(a) => {
            let lines = a.iter().map(compact);
            if rows {
                lines.collect()
            } else {
                iter::once("[".into()).chain(lines).chain(iter::once("]".into())).collect()
            }
        }
        Value::Object(m) => {
            let mut out = vec!["{".to_string()];
            for (k, v) in m {
                let mut key = String::new();
                string(&mut key, k, false);
                match v {
                    Value::Array(a) if a.len() > 1 => {
                        out.push(format!("{key}: ["));
                        out.extend(a.iter().map(|x| format!("  {}", compact(x))));
                        out.push("]".into());
                    }
                    _ => out.push(format!("{key}: {}", compact(v))),
                }
            }
            out.push("}".into());
            out
        }
        _ => vec![compact(&small)],
    }
}

fn crush(obj: &Value) -> Value {
    match obj {
        Value::Array(a) if a.len() > ITEMS => {
            let n = a.len();
            let same = constant(a);
            let (chosen, bad) = picks(a);
            let keep: BTreeSet<usize> = (0..FIRST).chain(n - LAST..n).chain(chosen).collect();
            let mut out = Vec::new();
            let mut next = 0;
            for i in keep {
                if i > next {
                    out.push(Value::String(format!("<... {} more items>", i - next)));
                }
                let item = match &a[i] {
                    Value::Object(m) if !same.is_empty() => Value::Object(
                        m.iter().filter(|(k, _)| !same.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect(),
                    ),
                    other => other.clone(),
                };
                out.push(crush(&item));
                next = i + 1;
            }
            let mut head = format!("<{n} items");
            if bad > 0 {
                let _ = write!(head, ", {bad} with error words");
            }
            if !same.is_empty() {
                let shown: Vec<String> = same
                    .iter()
                    .map(|(k, v)| {
                        if dumps(v, true, false).len() <= SHORT {
                            format!("{k}={}", dumps(v, false, false))
                        } else {
                            k.clone()
                        }
                    })
                    .collect();
                let _ = write!(head, ", the same in all: {}", shown.join(", "));
            }
            head.push('>');
            out.insert(0, Value::String(compact(&Value::String(head))));
            Value::Array(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(crush).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), crush(v))).collect()),
        Value::String(s) if len(s) > STR => Value::String(clip(s, STR)),
        other => other.clone(),
    }
}

fn constant(items: &[Value]) -> Map<String, Value> {
    let Some(first) = items.first().and_then(Value::as_object) else { return Map::new() };
    if !(2..=SAME_MAX).contains(&items.len()) || !items.iter().all(Value::is_object) {
        return Map::new();
    }
    first
        .iter()
        .filter(|(k, v)| items.iter().all(|it| it.get(k.as_str()) == Some(*v)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn brief(x: &Value) -> Value {
    match x {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(_, v)| !matches!(v, Value::String(s) if len(s) > SIGNAL))
                .map(|(k, v)| (k.clone(), brief(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().take(ITEMS).map(brief).collect()),
        other => other.clone(),
    }
}

fn picks(items: &[Value]) -> (Vec<usize>, usize) {
    let mut found = Vec::new();
    let mut kinds = HashSet::new();
    let mut bad = 0;
    for (i, it) in items.iter().enumerate() {
        let dumped = dumps(&brief(it), false, false);
        let text = QUIET.replace_all(head(&dumped, PICK_CHARS), "");
        if !IMPORTANT.is_match(&text) {
            continue;
        }
        bad += 1;
        if kinds.insert(NUMS.replace_all(&text, "#").into_owned()) {
            found.push(i);
        }
    }
    if items.len() >= OUTLIER_ROWS && items.iter().all(Value::is_object) {
        found.extend(outliers(items));
    }
    let mut seen = HashSet::new();
    found.retain(|i| seen.insert(*i));
    found.truncate(PICKS);
    (found, bad)
}

#[derive(PartialEq, Eq, Hash)]
enum Word<'a> {
    Text(&'a str),
    Flag(bool),
}

fn word(v: Option<&Value>) -> Option<Word<'_>> {
    match v {
        Some(Value::String(s)) => Some(Word::Text(s)),
        Some(Value::Bool(b)) => Some(Word::Flag(*b)),
        _ => None,
    }
}

fn outliers(items: &[Value]) -> Vec<usize> {
    let dicts: Vec<&Map<String, Value>> = items.iter().filter_map(Value::as_object).collect();
    let n = dicts.len();
    let mut keys: Vec<&str> = Vec::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for d in &dicts {
        for k in d.keys() {
            let c = counts.entry(k.as_str()).or_insert(0);
            if *c == 0 {
                keys.push(k);
            }
            *c += 1;
        }
    }
    let mut found = Vec::new();
    for key in keys {
        if (counts[key] as f64) < COMMON * n as f64 {
            continue;
        }
        let vals: Vec<Option<&Value>> = dicts.iter().map(|d| d.get(key)).collect();
        let nums: Vec<(usize, f64)> = vals
            .iter()
            .enumerate()
            .filter_map(|(i, v)| match v {
                Some(Value::Number(x)) => x.as_f64().map(|f| (i, f)),
                _ => None,
            })
            .collect();
        if nums.len() >= OUTLIER_ROWS {
            let m = nums.len() as f64;
            let mean = sum(nums.iter().map(|(_, v)| *v)) / m;
            let sd = (sum(nums.iter().map(|(_, v)| (v - mean) * (v - mean))) / m).sqrt();
            if sd != 0.0 {
                found.extend(nums.iter().filter(|(_, v)| (v - mean).abs() > SIGMAS * sd).map(|(i, _)| *i));
            }
        }
        let mut words: HashMap<Word, usize> = HashMap::new();
        for w in vals.iter().filter_map(|v| word(*v)) {
            *words.entry(w).or_insert(0) += 1;
        }
        if n >= WORD_ROWS && (2..=WORD_KINDS).contains(&words.len()) {
            found.extend(
                vals.iter().enumerate().filter(|(_, v)| word(**v).is_some_and(|w| words[&w] == 1)).map(|(i, _)| i),
            );
        }
    }
    found
}

fn sum(xs: impl Iterator<Item = f64>) -> f64 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for x in xs {
        let t = s + x;
        c += if s.abs() >= x.abs() { (s - t) + x } else { (x - t) + s };
        s = t;
    }
    if c != 0.0 && c.is_finite() { s + c } else { s }
}

fn compact(x: &Value) -> String {
    if let Value::String(s) = x
        && s.starts_with('<')
        && s.ends_with('>')
    {
        return if len(s) <= ITEM_CHARS { s.clone() } else { format!("{}...>", head(s, ITEM_CHARS)) };
    }
    let mut s = String::new();
    match x {
        Value::Object(m) => {
            let mut pairs: Vec<(&String, &Value)> = m.iter().collect();
            pairs.sort_by_cached_key(|(_, v)| weight(v));
            object(&mut s, pairs.into_iter(), false, true);
        }
        _ => dump(&mut s, x, false, true),
    }
    clip(&s, ITEM_CHARS)
}

fn weight(v: &Value) -> (bool, usize) {
    let s = dumps(v, false, false);
    (s.starts_with("\"http"), len(&s))
}

fn dumps(v: &Value, ascii: bool, tight: bool) -> String {
    let mut out = String::new();
    dump(&mut out, v, ascii, tight);
    out
}

fn dump(out: &mut String, v: &Value, ascii: bool, tight: bool) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => number(out, n),
        Value::String(s) => string(out, s, ascii),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(if tight { "," } else { ", " });
                }
                dump(out, x, ascii, tight);
            }
            out.push(']');
        }
        Value::Object(m) => object(out, m.iter(), ascii, tight),
    }
}

fn object<'a>(out: &mut String, pairs: impl Iterator<Item = (&'a String, &'a Value)>, ascii: bool, tight: bool) {
    out.push('{');
    for (i, (k, v)) in pairs.enumerate() {
        if i > 0 {
            out.push_str(if tight { "," } else { ", " });
        }
        string(out, k, ascii);
        out.push_str(if tight { ":" } else { ": " });
        dump(out, v, ascii, tight);
    }
    out.push('}');
}

fn number(out: &mut String, n: &Number) {
    match n.as_f64() {
        Some(f) if n.is_f64() => out.push_str(&float(f)),
        _ => {
            let _ = write!(out, "{n}");
        }
    }
}

fn float(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    if !f.is_finite() {
        return if f.is_nan() {
            "NaN"
        } else if f > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .into();
    }
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let point = exp + 1;
    let count = digits.len() as i32;
    let body = if !(-3..=16).contains(&point) {
        let (first, rest) = digits.split_at(1);
        let dot = if rest.is_empty() { String::new() } else { format!(".{rest}") };
        format!("{first}{dot}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.unsigned_abs())
    } else if point <= 0 {
        format!("0.{}{digits}", "0".repeat(point.unsigned_abs() as usize))
    } else if point >= count {
        format!("{digits}{}.0", "0".repeat((point - count) as usize))
    } else {
        let (int, frac) = digits.split_at(point as usize);
        format!("{int}.{frac}")
    };
    if f < 0.0 { format!("-{body}") } else { body }
}

fn string(out: &mut String, s: &str, ascii: bool) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c < ' ' || (ascii && c > '~') => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn floats_print_like_python_repr() {
        let cases = [
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (-0.5, "-0.5"),
            (100.0, "100.0"),
            (2.5e-5, "2.5e-05"),
            (123456789.125, "123456789.125"),
            (1.5e300, "1.5e+300"),
            (-0.0, "-0.0"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (0.1 + 0.2, "0.30000000000000004"),
        ];
        for (f, want) in cases {
            assert_eq!(float(f), want, "{f:e}");
        }
    }

    #[test]
    fn dumps_matches_python_json_dumps() {
        let v = json!({"a": [1, 2.0, null, true], "ä": "x\"\\\n\u{1}\u{7f}😀"});
        assert_eq!(dumps(&v, false, false), "{\"a\": [1, 2.0, null, true], \"ä\": \"x\\\"\\\\\\n\\u0001\u{7f}😀\"}");
        assert_eq!(
            dumps(&v, true, true),
            "{\"a\":[1,2.0,null,true],\"\\u00e4\":\"x\\\"\\\\\\n\\u0001\\u007f\\ud83d\\ude00\"}"
        );
    }

    #[test]
    fn compensated_sum_keeps_small_parts() {
        assert_eq!(sum([1.0, 1e100, 1.0, -1e100].into_iter()), 2.0);
        assert_eq!(sum(iter::empty()), 0.0);
    }

    #[test]
    fn json_lines_allow_one_bad_row_in_ten() {
        let mut rows: Vec<String> = (0..10).map(|i| format!("{{\"i\":{i}}}")).collect();
        rows.insert(3, "{broken".into());
        assert!(matches!(parse(&rows.join("\n")), Some((Value::Array(a), true)) if a.len() == 10));
        rows.insert(5, "{broken too".into());
        assert!(parse(&rows.join("\n")).is_none());
        assert!(parse("[1, 2").is_none());
    }
}
