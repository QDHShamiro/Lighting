#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

pub const LIST: i32 = 50008;
pub const ITEM: i32 = 50007;
pub const EDIT: i32 = 50004;
pub const DOC: i32 = 50030;
pub const TREEITEM: i32 = 50024;
const BUTTON: i32 = 50000;
const TEXT: i32 = 50020;
const LINK: i32 = 50005;
const IMAGE: i32 = 50006;
const OWN_ACTIONS: &[&str] = &["bearbeiten", "edit", "modifier", "editar", "modifica", "bewerken", "edytuj"];
const LINES: usize = 5;
const TEXT_MAX: usize = 300;
const SHOWN_DMS: usize = 8;
const SHOWN_OTHERS: usize = 15;
const STAMP_CHARS: usize = 60;

fn re(p: &str) -> Regex {
    Regex::new(p).expect("chat regex")
}

pub static LIST_NAME: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(nachrichten|messages|chat|unterhaltung|conversation|verlauf|messaggi|mensajes)"));
pub static FIELD_NAME: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(nachricht|message|schreib|write|antwort|reply|mensaje|messaggio)"));
pub static SEARCHY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)such|search|filter|find"));
const TIME_P: &str = r"\b(\d{1,2}:\d{2})(?:\s*[ap]\.?m\.?)?\b";
static TIME: LazyLock<Regex> = LazyLock::new(|| re(&format!("(?i){TIME_P}")));
static TIME_ONLY: LazyLock<Regex> = LazyLock::new(|| re(&format!("(?i)^(?:{TIME_P})$")));
static DAY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\d{4}|heute|gestern|today|yesterday"));
static LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)https?://|www\.|discord\.gg/|\b[\w-]+\.(com|de|net|org|gg|io|me|ly|to)/"));
static UNREAD_DM: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(.*?)[\s,]*\b(\d+)\s+(?:ungelesene nachricht(?:en)?|unread messages?)\b"));
static UNREAD_AT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(\d+)\s+(?:erwähnung(?:en)?|mentions?)\W*\s*(.+)$"));
static UNREAD_ANY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:ungelesene nachrichten|unread messages?),\s*(.+)$"));
static STATUS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i),\s*(?:sprachanruf aktiv|bildschirmübertragung aktiv|voice call active|screen share active|live|streamt).*$",
    )
});
static NUM_END: LazyLock<Regex> = LazyLock::new(|| re(r"(\d{6,})$"));
static PAREN_END: LazyLock<Regex> = LazyLock::new(|| re(r"\s*\([^)]*\)\s*$"));

pub struct Row<E> {
    pub key: String,
    pub num: Option<u128>,
    pub name: String,
    pub el: E,
}

#[derive(Clone, Default)]
pub struct Base {
    pub keys: HashSet<String>,
    pub top: Option<u128>,
    pub last: Option<String>,
    pub t: f64,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Msg {
    pub sender: Option<String>,
    pub when: Option<String>,
    pub text: String,
    pub own: bool,
    pub id: String,
}

impl<E> Row<E> {
    pub fn new(aid: &str, name: String, el: E) -> Row<E> {
        let num = NUM_END.captures(aid).and_then(|m| m[1].parse().ok());
        let key = if aid.is_empty() {
            let hex: String = Sha256::digest(name.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
            format!("h{}", &hex[..12])
        } else {
            aid.to_string()
        };
        Row { key, num, name, el }
    }

    pub fn id(&self) -> String {
        self.num.map_or_else(|| self.key.clone(), |n| n.to_string())
    }
}

pub fn baseline<E>(rows: &[Row<E>], t: f64) -> Base {
    Base {
        keys: rows.iter().map(|r| r.key.clone()).collect(),
        top: rows.iter().filter_map(|r| r.num).max(),
        last: rows.last().map(|r| r.key.clone()),
        t,
    }
}

pub fn since(spec: &str, t: f64) -> Base {
    let digits: String = spec.chars().filter(char::is_ascii_digit).collect();
    Base { keys: HashSet::new(), top: digits.parse().ok(), last: Some(spec.to_string()), t }
}

pub fn fresh<E>(base: &Base, rows: &[Row<E>]) -> Vec<usize> {
    if let Some(top) = base.top
        && rows.iter().all(|r| r.num.is_some())
    {
        return (0..rows.len()).filter(|i| rows[*i].num.is_some_and(|n| n > top)).collect();
    }
    let start = base.last.as_ref().and_then(|l| rows.iter().position(|r| r.key == *l)).map_or(0, |i| i + 1);
    (start..rows.len()).filter(|i| !base.keys.contains(&rows[*i].key)).collect()
}

pub fn is_stamp(text: &str) -> bool {
    text.chars().count() < STAMP_CHARS && TIME.is_match(text) && (DAY.is_match(text) || TIME_ONLY.is_match(text.trim()))
}

pub fn parse(desc: &[(i32, String, String)], row_name: &str) -> Msg {
    let (mut sender, mut when, mut parts) = (None::<String>, None::<String>, Vec::<String>::new());
    let (mut own, mut stamp, mut toolbar) = (false, false, false);
    for (ct, name, aid) in desc {
        if aid.starts_with("message-timestamp") {
            stamp = true;
            continue;
        }
        if *ct == BUTTON {
            own |= OWN_ACTIONS.contains(&name.to_lowercase().as_str());
            if stamp {
                toolbar = true;
            } else if sender.is_none() && !name.is_empty() {
                sender = Some(name.clone());
            }
            continue;
        }
        if name.is_empty() || toolbar {
            continue;
        }
        if !stamp || is_stamp(name) {
            if when.is_none() {
                when = TIME.captures(name).map(|m| m[1].to_string());
            }
            continue;
        }
        if (*ct == TEXT || *ct == LINK) && parts.last() != Some(name) {
            parts.push(name.clone());
        } else if *ct == IMAGE {
            parts.push(format!("[image {name}]"));
        }
    }
    if !stamp {
        return Msg { text: row_name.to_string(), own, ..Msg::default() };
    }
    Msg { sender, when, text: parts.join(" "), own, id: String::new() }
}

pub fn short_sender(name: Option<&str>) -> String {
    let s = PAREN_END.replace(name.unwrap_or(""), "").trim().to_string();
    if s.is_empty() { "?".into() } else { s }
}

pub fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn said_by_me(sent: &[String], text: &str) -> bool {
    let t = norm(text);
    sent.iter().any(|s| t == *s || (s.chars().count() >= 3 && t.ends_with(&format!(" {s}"))))
}

pub fn has_link(text: &str) -> bool {
    LINK_RE.is_match(text)
}

fn clean(s: &str) -> String {
    STATUS.replace_all(s, "").trim_matches([' ', ',']).to_string()
}

fn ranked(mut v: Vec<(usize, String)>) -> Vec<(usize, String)> {
    let mut seen = HashSet::new();
    v.retain(|x| seen.insert(x.clone()));
    v.sort_by_key(|x| std::cmp::Reverse(x.0));
    v
}

pub type Unread = (Vec<(usize, String)>, Vec<(usize, String)>, Vec<String>);

pub fn unread_of(names: &[String]) -> Unread {
    let (mut dms, mut mentions, mut others) = (Vec::new(), Vec::new(), Vec::<String>::new());
    for raw in names {
        let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(m) = UNREAD_AT.captures(&name) {
            mentions.push((m[1].parse().unwrap_or(0), clean(&m[2])));
        } else if let Some(m) = UNREAD_DM.captures(&name).filter(|m| !m[1].trim_matches([' ', ',']).is_empty()) {
            dms.push((m[2].parse().unwrap_or(0), clean(&m[1])));
        } else if let Some(m) = UNREAD_ANY.captures(&name) {
            let o = clean(&m[1]);
            if !others.contains(&o) {
                others.push(o);
            }
        }
    }
    (ranked(dms), ranked(mentions), others)
}

pub fn unread_text(head: &str, names: &[String]) -> String {
    let (dms, mentions, others) = unread_of(names);
    if dms.is_empty() && mentions.is_empty() && others.is_empty() {
        return format!("{head}\nnothing unread");
    }
    let mut lines = vec![head.to_string()];
    lines.extend(dms.iter().take(SHOWN_DMS).map(|(c, n)| format!("DM {n}: {c} new")));
    lines.extend(
        mentions.iter().take(SHOWN_DMS).map(|(c, n)| format!("@ {n}: {c} mention{}", if *c == 1 { "" } else { "s" })),
    );
    if !others.is_empty() {
        let more =
            if others.len() > SHOWN_OTHERS { format!(" (+{})", others.len() - SHOWN_OTHERS) } else { String::new() };
        lines.push(format!(
            "unread: {}{more}",
            others.iter().take(SHOWN_OTHERS).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    lines.join("\n")
}

pub fn show(head: &str, wref: &str, msgs: &[Msg]) -> String {
    let mut lines = Vec::new();
    if msgs.len() > LINES {
        lines.push(format!("... +{} older (wisply inbox {wref} --since {})", msgs.len() - LINES, msgs[0].id));
    }
    for m in &msgs[msgs.len().saturating_sub(LINES)..] {
        let text = if m.text.chars().count() <= TEXT_MAX {
            m.text.clone()
        } else {
            m.text.chars().take(TEXT_MAX - 3).collect::<String>() + "..."
        };
        let who = m.sender.as_deref().map_or(String::new(), |s| {
            format!("{}{}: ", short_sender(Some(s)), m.when.as_deref().map_or(String::new(), |w| format!(" {w}")))
        });
        lines.push(who + &text);
    }
    format!("{head}\n{}", lines.join("\n"))
}

pub fn human(seconds: f64) -> String {
    let s = seconds as i64;
    if seconds >= 120.0 && s % 60 == 0 && seconds.fract() == 0.0 { format!("{}m", s / 60) } else { format!("{s} s") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(spec: &[(&str, &str)]) -> Vec<Row<()>> {
        spec.iter().map(|(aid, name)| Row::new(aid, (*name).to_string(), ())).collect()
    }

    fn d(ct: i32, name: &str, aid: &str) -> (i32, String, String) {
        (ct, name.into(), aid.into())
    }

    #[test]
    fn fresh_rows_use_message_ids_or_the_last_seen_row() {
        let old = rows(&[("chat-messages-1-1000001", "a"), ("chat-messages-1-1000002", "b")]);
        let base = baseline(&old, 0.0);
        assert_eq!(base.top, Some(1_000_002));
        let now = rows(&[("chat-messages-1-1000002", "b"), ("chat-messages-1-1000003", "c")]);
        assert_eq!(fresh(&base, &now), vec![1]);
        assert_eq!(now[1].id(), "1000003");
        let plain = rows(&[("", "x"), ("", "y")]);
        let base = baseline(&plain, 0.0);
        let later = rows(&[("", "x"), ("", "y"), ("", "z")]);
        assert_eq!(fresh(&base, &later), vec![2]);
        assert!(later[2].id().starts_with('h') && later[2].id().len() == 13);
        assert_eq!(fresh(&since("#1000001", 0.0), &now), vec![0, 1]);
    }

    #[test]
    fn a_discord_row_gives_sender_time_and_text() {
        let desc = [
            d(BUTTON, "Anna (Mod)", ""),
            d(TEXT, "Heute um 14:05 Uhr", ""),
            d(TEXT, "", "message-timestamp-1"),
            d(TEXT, "14:05", "message-timestamp-1"),
            d(TEXT, "hallo du", ""),
            d(TEXT, "hallo du", ""),
            d(LINK, "discord.gg/x", ""),
            d(IMAGE, "cat.png", ""),
        ];
        let m = parse(&desc, "row");
        assert_eq!(m.sender.as_deref(), Some("Anna (Mod)"));
        assert_eq!(m.when.as_deref(), Some("14:05"));
        assert_eq!(m.text, "hallo du discord.gg/x [image cat.png]");
        assert!(!m.own);
        let mine = parse(&[d(TEXT, "", "message-timestamp-2"), d(BUTTON, "Bearbeiten", ""), d(TEXT, "x", "")], "r");
        assert!(mine.own && mine.text.is_empty());
        assert_eq!(parse(&[d(TEXT, "plain", "")], "the row").text, "the row");
        assert_eq!(short_sender(Some("Anna (Mod) ")), "Anna");
        assert_eq!(short_sender(None), "?");
    }

    #[test]
    fn stamps_links_and_own_texts_are_recognized() {
        assert!(is_stamp("14:05") && is_stamp("Gestern um 9:30 Uhr") && is_stamp("2:15 pm"));
        assert!(!is_stamp("see you at 14:05") && !is_stamp("hello"));
        assert!(has_link("look www.x.de") && has_link("example.com/a") && !has_link("ja klar."));
        let sent = vec![norm("Hallo   Welt"), norm("ok")];
        assert!(said_by_me(&sent, "hallo welt") && said_by_me(&sent, "Anna: hallo welt"));
        assert!(said_by_me(&sent, "OK") && !said_by_me(&sent, "alles ok"));
        assert_eq!((human(300.0), human(90.0), human(300.5)), ("5m".into(), "90 s".into(), "300 s".into()));
    }

    #[test]
    fn unread_lists_dms_mentions_and_channels() {
        let names: Vec<String> = [
            "Franz, 2 ungelesene Nachrichten",
            "Lukas 5 unread messages, voice call active",
            "Franz, 2 ungelesene Nachrichten",
            "3 Erwähnungen, Server X",
            "1 mention, Guild",
            "Ungelesene Nachrichten, allgemein",
            "plain row",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let (dms, at, others) = unread_of(&names);
        assert_eq!(dms, vec![(5, "Lukas".into()), (2, "Franz".into())]);
        assert_eq!(at, vec![(3, "Server X".into()), (1, "Guild".into())]);
        assert_eq!(others, vec!["allgemein".to_string()]);
        let text = unread_text("unread [w1] Discord", &names);
        assert_eq!(
            text,
            "unread [w1] Discord\nDM Lukas: 5 new\nDM Franz: 2 new\n@ Server X: 3 mentions\n@ Guild: 1 mention\nunread: allgemein"
        );
        assert_eq!(unread_text("h", &[]), "h\nnothing unread");
    }

    #[test]
    fn show_keeps_the_last_five_and_cuts_long_texts() {
        let msgs: Vec<Msg> = (1..=7)
            .map(|i| Msg {
                sender: (i != 7).then(|| "Anna (Mod)".to_string()),
                when: (i == 6).then(|| "9:01".to_string()),
                text: if i == 7 { "x".repeat(400) } else { format!("m{i}") },
                own: false,
                id: format!("{i}"),
            })
            .collect();
        let out = show("new [w2] chat #7", "w2", &msgs);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[1], "... +2 older (wisply inbox w2 --since 1)");
        assert_eq!(lines[2], "Anna: m3");
        assert_eq!(lines[5], "Anna 9:01: m6");
        assert_eq!(lines[6], format!("{}...", "x".repeat(297)));
    }
}
