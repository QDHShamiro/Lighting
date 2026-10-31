use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lt_core::{Call, Fail, Flag};
use regex::Regex;
use serde_json::{Value, json};

use super::super::ctx::now;
use super::super::web::{keys, parse_ms};
use super::super::{Out, Pending};
use super::imp::{BROWSER_EXES, CHAT_EXES};
use super::talk::{self, Base, DOC, EDIT, FIELD_NAME, ITEM, LIST, LIST_NAME, Msg, Row, SEARCHY, TREEITEM};
use super::uia::{self, El};
use super::{Desk, Res, win, windowish};

const WAIT_MS: u64 = 300_000;
const POLL: Duration = Duration::from_secs(1);
const GAP_S: f64 = 2.0;
const UNANSWERED: usize = 5;
const BASE_FRESH_S: f64 = 900.0;
const PARSED_MAX: usize = 400;
const SENT_KEPT: usize = 20;
const BACK_ROWS: usize = 10;
const CONFIRM: Duration = Duration::from_secs(3);
const CONFIRM_POLL: Duration = Duration::from_millis(250);
const TITLE: usize = 50;
const MEMO: &str = "chat.json";
const CHATS_SHOWN: usize = 3;
const CHAT_TITLE: usize = 40;

static SWITCHER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)quick switcher|find a channel|jump to").expect("switcher"));

#[derive(Default)]
struct Chat {
    sent: Vec<String>,
    unanswered: usize,
    sent_t: f64,
    base: Option<Base>,
    parsed: HashMap<String, Msg>,
}

static CHATS: LazyLock<Mutex<HashMap<isize, Chat>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn chat<T>(hwnd: isize, f: impl FnOnce(&mut Chat) -> T) -> T {
    let mut all = CHATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(all.entry(hwnd).or_default())
}

fn title(hwnd: isize) -> String {
    win::text_of(hwnd).chars().take(TITLE).collect()
}

fn exe(hwnd: isize) -> String {
    win::exe_of_window(hwnd).to_lowercase()
}

fn memo_me(home: &Path, exe: &str) -> Option<String> {
    let data: Value = serde_json::from_str(&fs::read_to_string(home.join(MEMO)).ok()?).ok()?;
    data[exe]["me"].as_str().map(String::from)
}

fn remember_me(home: &Path, exe: &str, name: &str) {
    let path = home.join(MEMO);
    let mut data: Value =
        fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
    if !data.is_object() {
        data = json!({});
    }
    if data[exe]["me"] == name {
        return;
    }
    if !data[exe].is_object() {
        data[exe] = json!({});
    }
    data[exe]["me"] = json!(name);
    let _ = fs::write(path, serde_json::to_string_pretty(&data).unwrap_or_default());
}

fn area(el: &El) -> i64 {
    let (l, t, r, b) = el.rect();
    i64::from((r - l).max(0)) * i64::from((b - t).max(0))
}

fn message_list(hwnd: isize) -> Result<El, Fail> {
    let root = uia::from_handle(hwnd).ok_or_else(|| Fail::new("the window has no UI tree"))?;
    let mut best: Option<(i64, El)> = None;
    for el in uia::find_all(&root, false, Some(LIST)) {
        let score = area(&el) * if LIST_NAME.is_match(&el.name()) { 10 } else { 1 };
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, el));
        }
    }
    best.map(|(_, el)| el).ok_or_else(|| {
        Fail::hint(
            format!("no message list in {}", title(hwnd)),
            "lighting focus w<N> (a minimized window may hide it), or lighting snap w<N>",
        )
    })
}

fn items(hwnd: isize) -> Result<Vec<Row<El>>, Fail> {
    let list = message_list(hwnd)?;
    Ok(uia::find_all(&list, true, Some(ITEM)).into_iter().map(|el| Row::new(&el.aid(), el.name(), el)).collect())
}

fn parsed(hwnd: isize, row: &Row<El>) -> Msg {
    if let Some(m) = chat(hwnd, |st| st.parsed.get(&row.key).cloned()) {
        return m;
    }
    let desc: Vec<(i32, String, String)> =
        uia::find_all(&row.el, false, None).iter().map(|d| (d.ctype(), d.name(), d.aid())).collect();
    let m = talk::parse(&desc, &row.name);
    chat(hwnd, |st| {
        if st.parsed.len() >= PARSED_MAX {
            st.parsed.clear();
        }
        st.parsed.insert(row.key.clone(), m.clone());
    });
    m
}

fn sender_before(hwnd: isize, rows: &[Row<El>], i: usize) -> Option<String> {
    rows[i.saturating_sub(BACK_ROWS)..i].iter().rev().find_map(|r| parsed(hwnd, r).sender)
}

fn news(home: &Path, hwnd: isize, base: &Base, who: &str) -> Result<(Vec<Msg>, Vec<Row<El>>), Fail> {
    let rows = items(hwnd)?;
    let new = talk::fresh(base, &rows);
    let Some(first) = new.first() else { return Ok((Vec::new(), rows)) };
    let mut sender = sender_before(hwnd, &rows, *first);
    let me = memo_me(home, &exe(hwnd)).map(|m| m.to_lowercase());
    let sent = chat(hwnd, |st| st.sent.clone());
    let mut out = Vec::new();
    for i in new {
        let mut m = parsed(hwnd, &rows[i]);
        if m.sender.is_some() {
            sender.clone_from(&m.sender);
        }
        m.sender.clone_from(&sender);
        let named_me = me.as_ref().is_some_and(|me| talk::short_sender(sender.as_deref()).to_lowercase() == *me);
        let other = !who.is_empty() && !sender.as_deref().unwrap_or("").to_lowercase().contains(who);
        if m.own || named_me || talk::said_by_me(&sent, &m.text) || other {
            continue;
        }
        m.id = rows[i].id();
        out.push(m);
    }
    Ok((out, rows))
}

struct Watch {
    hwnd: isize,
    wref: String,
    base: Base,
    who: String,
    total: f64,
    prefix: String,
}

impl Watch {
    fn check(&self, home: &Path, last: bool) -> Result<Option<String>, Fail> {
        if !win::alive(self.hwnd) {
            return Err(Fail::hint("the chat window was closed", "lighting windows -f chat"));
        }
        let (msgs, rows) = news(home, self.hwnd, &self.base, &self.who)?;
        let wref = &self.wref;
        if let Some(newest) = msgs.last() {
            let base = talk::baseline(&rows, now());
            chat(self.hwnd, |st| {
                st.base = Some(base);
                st.unanswered = 0;
            });
            let head = format!("new [{wref}] {} #{}", title(self.hwnd), newest.id);
            return Ok(Some(self.prefix.clone() + &talk::show(&head, wref, &msgs)));
        }
        if last {
            let tail = rows.last().map_or_else(|| "-".to_string(), Row::id);
            return Ok(Some(format!("{}nothing new in {} (last #{tail})", self.prefix, talk::human(self.total))));
        }
        Ok(None)
    }

    fn start(self, x: &mut Desk) -> Res<Out> {
        if let Some(text) = self.check(&x.d.home, self.total <= 0.0)? {
            return Ok(Out::Text(text));
        }
        let limit = Duration::from_secs_f64(self.total);
        Ok(Out::Wait(Pending::new(limit, POLL, move |d, _, last| self.check(&d.home, last))))
    }
}

fn resolve(x: &mut Desk, spec: Option<&str>) -> Res<isize> {
    let Some(spec) = spec.filter(|s| !windowish(s)) else { return x.target(spec) };
    let want = spec.to_lowercase();
    let mut hits: Vec<win::Win> =
        win::windows().into_iter().filter(|w| w.title.to_lowercase().contains(&want)).collect();
    hits.sort_by_key(|w| (BROWSER_EXES.contains(&w.exe.to_lowercase().as_str()), !w.fg));
    let Some(w) = hits.first() else { return Err(no_chat(x, spec).into()) };
    choose(x, w.hwnd);
    Ok(w.hwnd)
}

fn no_chat(x: &mut Desk, spec: &str) -> Fail {
    let chats: Vec<win::Win> =
        win::windows().into_iter().filter(|w| CHAT_EXES.contains(&w.exe.to_lowercase().as_str())).collect();
    let Some(first) = chats.first() else {
        return Fail::hint(format!("no window matches \"{spec}\""), "lighting windows -f chat");
    };
    let open: Vec<String> = chats
        .iter()
        .take(CHATS_SHOWN)
        .map(|w| format!("{} \"{}\"", x.wref(w.hwnd), w.title.chars().take(CHAT_TITLE).collect::<String>()))
        .collect();
    let at = x.wref(first.hwnd);
    let hint = match keys::key_for(&x.d.home, &first.exe.to_lowercase(), &SWITCHER) {
        Some(key) => format!(
            "open it first: lighting press {key} {at}, then lighting type focused \"{}\" --enter",
            spec.trim_start_matches(['@', '#'])
        ),
        None => "open that chat in the app first, then repeat".into(),
    };
    Fail::hint(format!("no chat window shows \"{spec}\" (open: {})", open.join(", ")), hint)
}

fn choose(x: &mut Desk, hwnd: isize) {
    x.st.hwnd = hwnd;
    x.ctx.target = Some(("app".into(), hwnd.to_string()));
}

fn timeout(c: &Call) -> Result<f64, Fail> {
    Ok(parse_ms(c.flags.get("timeout"), WAIT_MS)? as f64 / 1000.0)
}

fn who(c: &Call) -> String {
    c.val("from").unwrap_or("").to_lowercase()
}

fn inbox(x: &mut Desk, c: &Call, aim: Option<&str>) -> Res<Out> {
    let hwnd = resolve(x, aim.or(c.pos.first().map(String::as_str)))?;
    x.st.launched.remove(&hwnd);
    let total = timeout(c)?;
    let t = now();
    let base = match c.val("since").filter(|s| !s.is_empty()) {
        Some(s) => talk::since(s, t),
        None => match chat(hwnd, |st| st.base.clone()).filter(|b| t - b.t < BASE_FRESH_S) {
            Some(b) => b,
            None => talk::baseline(&items(hwnd)?, t),
        },
    };
    let wref = x.wref(hwnd);
    Watch { hwnd, wref, base, who: who(c), total, prefix: String::new() }.start(x)
}

fn unread(x: &mut Desk, c: &Call, aim: Option<&str>) -> Res<String> {
    let spec = aim.or(c.pos.first().map(String::as_str));
    let hwnd = match spec {
        Some(_) => resolve(x, spec)?,
        None => match win::windows().into_iter().find(|w| CHAT_EXES.contains(&w.exe.to_lowercase().as_str())) {
            Some(w) => {
                choose(x, w.hwnd);
                w.hwnd
            }
            None => x.target(None)?,
        },
    };
    let root = uia::from_handle(hwnd).ok_or_else(|| Fail::new("the window has no UI tree"))?;
    let names: Vec<String> =
        [TREEITEM, ITEM].into_iter().flat_map(|t| uia::find_all(&root, false, Some(t))).map(|el| el.name()).collect();
    let head = format!("unread [{}] {}", x.wref(hwnd), title(hwnd));
    Ok(talk::unread_text(&head, &names))
}

fn composer(hwnd: isize) -> Result<(El, String), Fail> {
    let root = uia::from_handle(hwnd).ok_or_else(|| Fail::new("the window has no UI tree"))?;
    let (_, wt, _, wb) = win::rect(hwnd);
    let mut best: Option<((u8, i32), El, String)> = None;
    for el in [EDIT, DOC].into_iter().flat_map(|t| uia::find_all(&root, false, Some(t))) {
        let (l, t, r, b) = el.rect();
        let name = el.name();
        let tall = el.ctype() == DOC && f64::from(b - t) > f64::from(wb - wt) * 0.5;
        if el.aid() == "RootWebArea" || (r - l) * (b - t) <= 0 || tall || SEARCHY.is_match(&name) {
            continue;
        }
        let score = (if FIELD_NAME.is_match(&name) { 2 } else { 0 }, b);
        if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
            best = Some((score, el, name));
        }
    }
    let Some((_, el, name)) = best else {
        return Err(Fail::hint(
            format!("no message field in {}", title(hwnd)),
            "lighting snap w<N> -f nachricht, then type d<N> text --enter",
        ));
    };
    Ok((el, if name.is_empty() { "message field".into() } else { name }))
}

fn confirm(home: &Path, hwnd: isize, base: &Base, text: &str) -> Option<String> {
    let mine = [talk::norm(text)];
    let t0 = Instant::now();
    while t0.elapsed() < CONFIRM {
        let rows = items(hwnd).unwrap_or_default();
        for i in talk::fresh(base, &rows).into_iter().rev() {
            let m = parsed(hwnd, &rows[i]);
            if !talk::said_by_me(&mine, &m.text) {
                continue;
            }
            if m.own
                && let Some(sender) = m.sender.or_else(|| sender_before(hwnd, &rows, i))
            {
                remember_me(home, &exe(hwnd), &talk::short_sender(Some(&sender)));
            }
            return Some(rows[i].id());
        }
        thread::sleep(CONFIRM_POLL);
    }
    None
}

fn reply(x: &mut Desk, c: &Call, aim: Option<&str>, may_wait: bool) -> Res<Out> {
    let (spec, words) = match aim {
        Some(a) => (a, &c.pos[..]),
        None if c.pos.len() >= 2 => (c.pos[0].as_str(), &c.pos[1..]),
        None => {
            return Err(
                Fail::hint("reply needs the chat and the text", "lighting reply \"@Name\" \"text\" [--wait]").into()
            );
        }
    };
    let hwnd = resolve(x, Some(spec))?;
    let text = x.ctx.secret.clone().unwrap_or_else(|| words.join(" "));
    if text.trim().is_empty() {
        return Err(Fail::new("nothing to send").into());
    }
    let wref = x.wref(hwnd);
    let yes = c.on("yes");
    if talk::has_link(&text) && !yes {
        return Err(Fail::hint(
            "the message contains a link",
            "send it only if the user asked for it: lighting reply ... --yes",
        )
        .into());
    }
    let (unanswered, sent_t) = chat(hwnd, |st| (st.unanswered, st.sent_t));
    if unanswered >= UNANSWERED && !yes {
        return Err(Fail::hint(
            format!("{unanswered} messages in a row without an answer"),
            format!("wait for an answer (lighting inbox {wref}) or ask the user; --yes sends anyway"),
        )
        .into());
    }
    let gap = GAP_S - (now() - sent_t);
    if gap > 0.0 {
        thread::sleep(Duration::from_secs_f64(gap));
    }
    x.st.launched.remove(&hwnd);
    let base = talk::baseline(&items(hwnd)?, now());
    let (el, label) = composer(hwnd)?;
    let mut typed = Call { name: "type".into(), ..Call::default() };
    typed.flags.insert("enter".into(), Flag::On);
    if c.on("stay") {
        typed.flags.insert("stay".into(), Flag::On);
    }
    x.put_text(hwnd, &el, &label, &text, &typed)?;
    chat(hwnd, |st| {
        st.sent_t = now();
        st.unanswered += 1;
        st.sent.push(talk::norm(&text));
        let extra = st.sent.len().saturating_sub(SENT_KEPT);
        st.sent.drain(..extra);
        st.base = Some(base.clone());
    });
    let mid = confirm(&x.d.home, hwnd, &base, &text).map_or_else(|| " (not visible yet)".into(), |m| format!(" #{m}"));
    let head = format!("sent to [{wref}] {}{mid}", title(hwnd));
    if !c.on("wait") {
        return Ok(Out::Text(head));
    }
    if !may_wait {
        return Ok(Out::Text(format!("{head}\n-> lighting inbox {wref} waits for the answer")));
    }
    Watch { hwnd, wref, base, who: who(c), total: timeout(c)?, prefix: head + "\n" }.start(x)
}

pub fn run(x: &mut Desk, c: &Call, aim: Option<&str>, may_wait: bool) -> Res<Out> {
    match c.name.as_str() {
        "inbox" => inbox(x, c, aim),
        "reply" => reply(x, c, aim, may_wait),
        _ => unread(x, c, aim).map(Out::Text),
    }
}
