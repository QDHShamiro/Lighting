mod files;
mod pattern;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use regex::Regex;
use wisply_core::{Call, Fail, OUT_CHARS, cap, out_file};

use super::ctx::Ctx;
use super::{Daemon, Out, Pending};
use files::{MAX_BYTES, Scan, Shown, candidates, scan};
use pattern::{Opts, matcher, split_list};

const LINES: usize = 80;
const PER_FILE: usize = 8;
const BUDGET_S: u64 = 30;
const WAIT_S: u64 = BUDGET_S + 11;
const INLINE: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(250);
const MORE_FILES: usize = 30;
const ALL: usize = 1_000_000_000;
const USAGE: &str = "wisply grep <pattern> [folder|file ...] [-i] [-w] [-x] [-F|-E|-P] [-l] [-o] [-v] [-m 1] [-C 2] [--glob '*.py'] [--include '*.py'] [--exclude-dir build] [--all]";
const LETTERS: &str = "iwxFEPGlcaovrRnHIhs";

static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-([ABCm])(\d*)$").expect("numbered"));
static BARE_BAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|[^\\])\|").expect("bar"));
static SHORTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-[A-Za-z]+$").expect("shorts"));

fn number(val: &str, flag: &str) -> Result<usize, Fail> {
    val.parse::<usize>()
        .ok()
        .filter(|_| !val.is_empty() && val.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| Fail::hint(format!("{flag} needs a number ({val})"), format!("wisply grep {flag} 2 <pattern>")))
}

fn options(c: &Call) -> Result<(Opts, Vec<String>), Fail> {
    let f = |k: &str| c.on(k);
    let mut o = Opts {
        ignore: f("ignore-case"),
        word: f("word") || f("word-regexp"),
        line: f("line-regexp"),
        fixed: f("fixed") || f("fixed-strings"),
        ere: f("extended-regexp"),
        perl: f("perl-regexp"),
        files: f("files") || f("files-with-matches") || f("count"),
        text: f("text"),
        only: f("only-matching"),
        invert: f("invert-match"),
        hidden: f("hidden"),
        all: f("all"),
        before: 0,
        after: 0,
        max: None,
        patterns: Vec::new(),
    };
    if let Some(v) = c.val("context") {
        let n = number(v, "-C")?;
        (o.before, o.after) = (n, n);
    }
    if let Some(v) = c.val("max-count") {
        o.max = Some(number(v, "-m")?);
    }
    let (mut rest, mut i, mut done) = (Vec::new(), 0, false);
    let pos = &c.pos;
    while i < pos.len() {
        let a = pos[i].as_str();
        if done || a.chars().count() < 2 || !a.starts_with('-') {
            rest.push(a.to_string());
        } else if a == "--" {
            done = true;
        } else if a == "-e" {
            i += 1;
            o.patterns.push(pos.get(i).ok_or_else(|| Fail::hint("-e needs a pattern after it", USAGE))?.clone());
        } else if let Some(m) = NUMBERED.captures(a) {
            let mut val = m[2].to_string();
            if val.is_empty() && i + 1 < pos.len() {
                i += 1;
                val.clone_from(&pos[i]);
            }
            let n = number(&val, &format!("-{}", &m[1]))?;
            match &m[1] {
                "m" => o.max = Some(n),
                "B" => o.before = n,
                "A" => o.after = n,
                _ => (o.before, o.after) = (n, n),
            }
        } else if SHORTS.is_match(a) {
            if let Some(bad) = a[1..].chars().find(|ch| !LETTERS.contains(*ch)) {
                return Err(Fail::hint(
                    format!("grep option -{bad} is not supported"),
                    format!("{USAGE} (a pattern that starts with - goes after --)"),
                ));
            }
            for ch in a[1..].chars() {
                match ch {
                    'i' => o.ignore = true,
                    'w' => o.word = true,
                    'x' => o.line = true,
                    'F' => o.fixed = true,
                    'E' => o.ere = true,
                    'P' => o.perl = true,
                    'l' | 'c' => o.files = true,
                    'a' => o.text = true,
                    'o' => o.only = true,
                    'v' => o.invert = true,
                    _ => {}
                }
            }
        } else {
            rest.push(a.to_string());
        }
        i += 1;
    }
    if o.only && o.invert {
        return Err(Fail::hint("-o with -v shows nothing: the lines -v picks have no match", USAGE));
    }
    if o.only {
        (o.before, o.after) = (0, 0);
    }
    Ok((o, rest))
}

#[derive(Default)]
struct Found {
    results: Vec<(String, usize, Shown)>,
    binary: usize,
    big: usize,
    gone: usize,
    searched: usize,
    stopped: bool,
}

fn search(
    roots: &[PathBuf],
    base: &Path,
    o: &Opts,
    globs: &[String],
    skipdirs: &[String],
    m: &pattern::Matcher,
) -> Found {
    let single = roots.len() == 1 && roots[0].is_dir();
    let deadline = Instant::now() + Duration::from_secs(BUDGET_S);
    let (mut f, mut seen) = (Found::default(), HashSet::new());
    'roots: for root in roots {
        for (path, show) in candidates(root, base, single, o, globs, skipdirs) {
            if Instant::now() > deadline {
                f.stopped = true;
                break 'roots;
            }
            let key = if cfg!(windows) { files::posix(&path).to_lowercase() } else { files::posix(&path) };
            if !seen.insert(key) {
                continue;
            }
            match scan(&path, &m.rx, m.pre.as_ref(), o) {
                Scan::Binary => f.binary += 1,
                Scan::Big => f.big += 1,
                Scan::Gone => f.gone += 1,
                Scan::Found(n, shown) => {
                    f.searched += 1;
                    if n > 0 {
                        f.results.push((show, n, shown));
                    }
                }
            }
        }
    }
    f
}

fn render(results: &[(String, usize, Shown)], o: &Opts, budget: usize, bare: bool) -> (Vec<String>, usize) {
    if o.files {
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for (show, count, _) in results {
            let (folder, name) = show.rsplit_once('/').unwrap_or(("", show));
            let entry = format!("{name} ({count})");
            match groups.iter_mut().find(|(f, _)| f == folder) {
                Some((_, names)) => names.push(entry),
                None => groups.push((folder.to_string(), vec![entry])),
            }
        }
        let lines = groups
            .into_iter()
            .map(|(f, names)| if f.is_empty() { names.join(", ") } else { format!("{f}/  {}", names.join(", ")) })
            .collect();
        return (lines, 0);
    }
    let per_file = PER_FILE.max(budget / results.len().max(1));
    let (mut out, mut used, mut rest) = (Vec::new(), 0, Vec::new());
    let context = o.before > 0 || o.after > 0;
    let pad = if bare { "" } else { "  " };
    for (show, count, shown) in results {
        if used >= budget {
            rest.push(format!("{show} ({count})"));
            continue;
        }
        let (allow, mut n, mut prev) = (per_file.min(budget - used), 0, None::<usize>);
        if !bare {
            out.push(if *count > 1 { format!("{show} ({count})") } else { show.clone() });
        }
        for (no, text, kind) in shown {
            if *kind == ':' {
                if n >= allow {
                    break;
                }
                n += 1;
            }
            if context && prev.is_some_and(|p| *no > p + 1) {
                out.push(format!("{pad}--"));
            }
            out.push(format!("{pad}{no}{kind} {text}").trim_end().to_string());
            prev = Some(*no);
        }
        if *count > n {
            out.push(format!("{pad}... +{} more in this file", count - n));
        }
        used += n;
    }
    if !rest.is_empty() {
        let more = if rest.len() > MORE_FILES { format!(", +{} more", rest.len() - MORE_FILES) } else { String::new() };
        out.push(format!("more files: {}{more}", rest[..rest.len().min(MORE_FILES)].join(", ")));
    }
    (out, results.iter().map(|r| r.1).sum::<usize>().saturating_sub(used))
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn answer(d: &Daemon, ctx: &mut Ctx, f: &Found, ask: &Ask) -> String {
    let o = &ask.o;
    let mut notes = Vec::new();
    for (n, what) in [
        (f.binary, "binary".to_string()),
        (f.big, format!("over {} MB", MAX_BYTES / 1_000_000)),
        (f.gone, "unreadable".into()),
    ] {
        if n > 0 {
            notes.push(format!("{n} {what}"));
        }
    }
    let mut tail = String::new();
    if !notes.is_empty() {
        tail += &format!(", skipped {}", notes.join(", "));
    }
    if f.stopped {
        tail += &format!("; stopped after {BUDGET_S} s, narrow the folder or add --glob");
    }
    if !ask.warn.is_empty() {
        tail += &format!("; {}", ask.warn);
    }
    if f.results.is_empty() {
        if !(o.fixed || o.ere || o.perl || o.invert) && BARE_BAR.is_match(&ask.pattern) {
            tail += r#"; | is a plain character here, use \| or -E for "or""#;
        }
        let head = if o.invert { "every line matches" } else { "no line matches" };
        return format!("{head} \"{}\" in {} file{}{tail}", ask.pattern, f.searched, plural(f.searched));
    }
    let total: usize = f.results.iter().map(|r| r.1).sum();
    let budget = if o.all { ALL } else { ask.max.unwrap_or(LINES) };
    let (mut lines, hidden) = render(&f.results, o, budget, ask.bare);
    if hidden > 0 || !tail.is_empty() {
        let files = if f.results.len() > 1 { format!(" in {} files", f.results.len()) } else { String::new() };
        let searched = if f.searched > 1 { format!(" ({} searched)", f.searched) } else { String::new() };
        lines.push(format!("{total} line{}{files}{searched}{tail}", plural(total)));
    }
    let text = lines.join("\n");
    let dir = d.home.join("out");
    if o.all || hidden == 0 {
        let (text, cut) = cap(&dir, &text, "grep", None, OUT_CHARS);
        ctx.cut += cut;
        return text;
    }
    let plain = Opts { files: false, patterns: Vec::new(), ..ask.o.clone() };
    let full = render(&f.results, &plain, ALL, ask.bare).0.join("\n");
    let path = out_file(&dir, "grep", "txt");
    let _ = fs::create_dir_all(&dir).and_then(|()| fs::write(&path, &full));
    ctx.cut += full.chars().count().saturating_sub(text.chars().count());
    format!("{text}\nfull list: {}", files::posix(&path))
}

struct Ask {
    o: Opts,
    pattern: String,
    warn: String,
    max: Option<usize>,
    bare: bool,
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let (o, rest) = options(c)?;
    let pats: Vec<String> =
        if o.patterns.is_empty() { rest.iter().take(1).cloned().collect() } else { o.patterns.clone() };
    if pats.is_empty() {
        return Err(Fail::hint("what should I search for?", USAGE));
    }
    let names: Vec<String> = if o.patterns.is_empty() { rest[1..].to_vec() } else { rest };
    let m = matcher(&pats, &o, o.fixed).map_err(|e| Fail::hint(e, USAGE))?;
    let mut globs = split_list(c.val("glob"));
    globs.extend(split_list(c.val("include")));
    globs.extend(split_list(c.val("exclude")).into_iter().map(|g| format!("!{g}")));
    let skipdirs = split_list(c.val("exclude-dir"));
    let base = if ctx.cwd.is_empty() { std::env::current_dir().unwrap_or_default() } else { PathBuf::from(&ctx.cwd) };
    let mut roots = Vec::new();
    for name in if names.is_empty() { vec![".".to_string()] } else { names } {
        let p = super::web::local_file(ctx, &name);
        if !p.exists() {
            return Err(Fail::hint(format!("no file or folder {name}"), USAGE));
        }
        roots.push(p);
    }
    let ask = Ask {
        bare: roots.len() == 1 && roots[0].is_file(),
        pattern: pats.join("\" or \""),
        warn: m.warn.clone(),
        max: c.val("max").filter(|v| v.bytes().all(|b| b.is_ascii_digit())).and_then(|v| v.parse().ok()),
        o,
    };
    let o = ask.o.clone();
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("grep".into())
        .spawn(move || {
            let _ = tx.send(search(&roots, &base, &o, &globs, &skipdirs, &m));
        })
        .map_err(|e| Fail::new(format!("grep could not start: {e}")))?;
    if let Ok(found) = rx.recv_timeout(INLINE) {
        return Ok(Out::Text(answer(d, ctx, &found, &ask)));
    }
    Ok(Out::Wait(Pending::new(Duration::from_secs(WAIT_S), POLL, move |d, ctx, last| match rx.try_recv() {
        Ok(found) => Ok(Some(answer(d, ctx, &found, &ask))),
        Err(TryRecvError::Empty) if last => {
            Err(Fail::hint("the search did not end in time", "narrow the folder or add --glob"))
        }
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) => Err(Fail::hint("search failed", USAGE)),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(args: &[&str]) -> Call {
        wisply_core::parse(&args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn grep_options_read_like_2x() {
        let (o, rest) = options(&call(&["grep", "-iw", "-C", "2", "-m3", "foo", "src", "--", "-x"])).expect("ok");
        assert!(o.ignore && o.word && !o.line);
        assert_eq!((o.before, o.after, o.max), (2, 2, Some(3)));
        assert_eq!(rest, ["foo", "src", "-x"]);
        let bad = options(&call(&["grep", "-q", "x"])).err().expect("refused");
        assert!(bad.text().starts_with("err: grep option -q is not supported"));
        assert!(options(&call(&["grep", "-ov", "x"])).is_err());
        let (o, _) = options(&call(&["grep", "-o", "-A", "4", "x"])).expect("ok");
        assert_eq!((o.before, o.after), (0, 0));
    }
}
