use std::sync::LazyLock;

use fancy_regex::Regex;
use regex::Regex as Plain;

#[derive(Clone)]
pub struct Opts {
    pub ignore: bool,
    pub word: bool,
    pub line: bool,
    pub fixed: bool,
    pub ere: bool,
    pub perl: bool,
    pub files: bool,
    pub text: bool,
    pub only: bool,
    pub invert: bool,
    pub hidden: bool,
    pub all: bool,
    pub before: usize,
    pub after: usize,
    pub max: Option<usize>,
    pub patterns: Vec<String>,
}

static INTERVAL: LazyLock<Plain> = LazyLock::new(|| Plain::new(r"^\{\d*(?:,\d*)?\}").expect("interval"));
static BRACES: LazyLock<Plain> = LazyLock::new(|| Plain::new(r"\{([^{}]*,[^{}]*)\}").expect("braces"));

const POSIX: &[(&str, &str)] = &[
    ("alpha", "a-zA-Z"),
    ("digit", "0-9"),
    ("alnum", "a-zA-Z0-9"),
    ("upper", "A-Z"),
    ("lower", "a-z"),
    ("space", r"\s"),
    ("blank", r" \t"),
    ("punct", r"!-/:-@\[-`{-~"),
    ("xdigit", "0-9A-Fa-f"),
    ("word", r"\w"),
    ("cntrl", r"\x00-\x1f\x7f"),
    ("print", r" -~"),
    ("graph", "!-~"),
];

fn esc(s: &str) -> String {
    regex::escape(s)
}

fn starts(p: &[char], at: usize, any: &[&str]) -> bool {
    any.iter().any(|s| s.chars().enumerate().all(|(k, c)| p.get(at + k) == Some(&c)))
}

fn find(p: &[char], what: [char; 2], from: usize) -> Option<usize> {
    (from..p.len().saturating_sub(1)).find(|&k| p[k] == what[0] && p[k + 1] == what[1])
}

fn bracket(p: &[char], i: usize) -> (Option<String>, usize) {
    let mut j = i + 1 + usize::from(p.get(i + 1) == Some(&'^'));
    j += usize::from(p.get(j) == Some(&']'));
    while j < p.len() && p[j] != ']' {
        if starts(p, j, &["[:", "[.", "[="]) {
            match find(p, [p[j + 1], ']'], j + 2) {
                Some(end) => j = end + 2,
                None => return (None, i + 1),
            }
        } else {
            j += if p[j] == '\\' { 2 } else { 1 };
        }
    }
    if j >= p.len() {
        return (None, i + 1);
    }
    let body = &p[i + 1..j];
    let (mut out, mut k) = (String::new(), 0);
    if body.first() == Some(&'^') {
        out.push('^');
        k = 1;
    }
    if body.get(k) == Some(&']') {
        out.push_str(r"\]");
        k += 1;
    }
    while k < body.len() {
        if starts(body, k, &["[:", "[.", "[="]) {
            let end = find(body, [body[k + 1], ']'], k + 2).unwrap_or(body.len());
            let name: String = body[k + 2..end].iter().collect();
            if body[k + 1] == ':' {
                out.push_str(
                    &POSIX.iter().find(|(n, _)| *n == name).map_or_else(|| esc(&name), |(_, v)| (*v).to_string()),
                );
            } else {
                out.push_str(&esc(&name));
            }
            k = end + 2;
        } else if body[k] == '\\' && k + 1 < body.len() {
            out.extend(&body[k..k + 2]);
            k += 2;
        } else {
            if "[&~|".contains(body[k]) {
                out.push('\\');
            }
            out.push(body[k]);
            k += 1;
        }
    }
    (Some(format!("[{out}]")), j + 1)
}

pub fn translate(raw: &str, ere: bool) -> String {
    let p: Vec<char> = raw.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        let start = out.last().is_none_or(|l| ["(", "|", "^"].contains(&l.as_str()));
        if c == '\\' && i + 1 < p.len() {
            let d = p[i + 1];
            i += 2;
            out.push(if "(){}|+?".contains(d) && !ere {
                d.to_string()
            } else if "<>".contains(d) {
                r"\b".into()
            } else if d == '`' {
                "^".into()
            } else if d == '\'' {
                "$".into()
            } else if "wWsSbBdD123456789".contains(d) {
                format!("\\{d}")
            } else {
                esc(&d.to_string())
            });
            continue;
        }
        i += 1;
        out.push(match c {
            '[' => {
                let (cls, next) = bracket(&p, i - 1);
                i = next;
                cls.unwrap_or_else(|| r"\[".into())
            }
            '*' if start => r"\*".into(),
            '+' | '?' if ere && !start => c.to_string(),
            '+' | '?' => format!("\\{c}"),
            '{' => {
                let rest: String = p[i - 1..].iter().collect();
                match INTERVAL.find(&rest).map(|m| m.as_str().to_string()) {
                    Some(m) if ere && !start && m != "{}" && m != "{,}" => {
                        i += m.chars().count() - 1;
                        m
                    }
                    _ => r"\{".into(),
                }
            }
            '(' | ')' | '|' | '}' if ere => c.to_string(),
            '(' | ')' | '|' | '}' => format!("\\{c}"),
            '^' if ere || start => "^".into(),
            '^' => r"\^".into(),
            '$' if ere || i == p.len() || starts(&p, i, &["\\)", "\\|"]) => "$".into(),
            '$' => r"\$".into(),
            _ => c.to_string(),
        });
    }
    out.concat()
}

pub struct Matcher {
    pub rx: Regex,
    pub pre: Option<Regex>,
    pub warn: String,
}

pub fn matcher(patterns: &[String], o: &Opts, fixed: bool) -> Result<Matcher, String> {
    let texts: Vec<String> = patterns
        .iter()
        .map(|p| {
            if fixed {
                esc(p)
            } else if o.perl {
                p.clone()
            } else {
                translate(p, o.ere)
            }
        })
        .collect();
    let mut text = if texts.len() == 1 {
        texts[0].clone()
    } else {
        texts.iter().map(|t| format!("(?:{t})")).collect::<Vec<_>>().join("|")
    };
    if o.word {
        text = format!(r"(?<!\w)(?:{text})(?!\w)");
    }
    if o.line {
        text = format!("^(?:{text})$");
    }
    let flags = if o.ignore { "(?i)" } else { "" };
    match Regex::new(&format!("{flags}{text}")) {
        Ok(rx) => {
            let pre = if text.contains(r"\A") || text.contains(r"\Z") {
                None
            } else {
                Regex::new(&format!("(?m{}){text}", if o.ignore { "i" } else { "" })).ok()
            };
            Ok(Matcher { rx, pre, warn: String::new() })
        }
        Err(e) if !fixed => {
            let mut m = matcher(patterns, o, true)?;
            m.warn = format!("not a regex ({e}), searched as plain text");
            Ok(m)
        }
        Err(e) => Err(format!("cannot read the pattern: {e}")),
    }
}

fn braces(glob: &str) -> Vec<String> {
    let Some(m) = BRACES.captures(glob) else { return vec![glob.to_string()] };
    let whole = m.get(0).expect("whole");
    m[1].split(',')
        .flat_map(|part| braces(&format!("{}{part}{}", &glob[..whole.start()], &glob[whole.end()..])))
        .collect()
}

pub fn split_list(val: Option<&str>) -> Vec<String> {
    let Some(val) = val else { return Vec::new() };
    let (mut parts, mut cur, mut depth) = (Vec::new(), String::new(), 0i32);
    for ch in val.chars() {
        if ch == ',' && depth == 0 {
            parts.push(std::mem::take(&mut cur));
            continue;
        }
        depth = (depth
            + match ch {
                '{' => 1,
                '}' => -1,
                _ => 0,
            })
        .max(0);
        cur.push(ch);
    }
    parts.push(cur);
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).flat_map(braces).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_patterns_translate_like_2x() {
        let cases = [
            (r"foo\|bar", false, "foo|bar"),
            (r"a\(b\)*", false, "a(b)*"),
            ("a+b?", false, r"a\+b\?"),
            ("a+b?", true, "a+b?"),
            ("*x", false, r"\*x"),
            ("x{2,3}", true, "x{2,3}"),
            ("x{2,3}", false, r"x\{2,3\}"),
            ("[[:digit:]]+", true, "[0-9]+"),
            ("[]a]", false, r"[\]a]"),
            ("[a&&b]", false, r"[a\&\&b]"),
            ("a^b$c", false, r"a\^b\$c"),
            ("^ab$", false, "^ab$"),
            (r"\<word\>", false, r"\bword\b"),
            ("a.b", false, "a.b"),
            ("[unclosed", false, r"\[unclosed"),
            (r"(a)\1", true, r"(a)\1"),
        ];
        for (raw, ere, want) in cases {
            assert_eq!(translate(raw, ere), want, "{raw}");
        }
        assert_eq!(split_list(Some("*.{rs,py}, !x,")), ["*.rs", "*.py", "!x"]);
        assert!(split_list(None).is_empty());
    }

    #[test]
    fn word_and_unicode_matches_work() {
        let o = Opts {
            ignore: true,
            word: true,
            line: false,
            fixed: false,
            ere: false,
            perl: false,
            files: false,
            text: false,
            only: false,
            invert: false,
            hidden: false,
            all: false,
            before: 0,
            after: 0,
            max: None,
            patterns: Vec::new(),
        };
        let m = matcher(&["grüße".into()], &o, false).expect("compiles");
        assert!(m.rx.is_match("Viele GRÜSSE grüße!").unwrap_or(false));
        assert!(!m.rx.is_match("xgrüßex").unwrap_or(true));
        let bad = matcher(&["a(".into()], &Opts { word: false, ere: true, ..o }, false).expect("falls back");
        assert!(bad.warn.starts_with("not a regex") && bad.rx.is_match("xa(y").unwrap_or(false));
    }
}
