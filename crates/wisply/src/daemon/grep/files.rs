use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::thread;

use fancy_regex::{Match, Regex};
use regex::Regex as Plain;

use super::pattern::Opts;

const GIT_WORKERS: usize = 8;
const SKIP: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    ".gradle",
    ".idea",
    "venv",
    ".venv",
    "__pycache__",
    ".next",
    ".cache",
];
pub const KEEP: usize = 2000;
const LONG: usize = 220;
const SIDE: usize = 80;
const HEAD: usize = 8000;
pub const MAX_BYTES: u64 = 300_000_000;

static GLOBS: LazyLock<Mutex<HashMap<String, Plain>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn fold(s: &str) -> String {
    if cfg!(windows) { s.to_lowercase().replace('/', "\\") } else { s.to_string() }
}

pub fn fnmatch(name: &str, glob: &str) -> bool {
    let mut cache = GLOBS.lock().unwrap_or_else(PoisonError::into_inner);
    if !cache.contains_key(glob) {
        let g: Vec<char> = fold(glob).chars().collect();
        let (mut re, mut i) = (String::from("(?s)^"), 0);
        while i < g.len() {
            let c = g[i];
            i += 1;
            match c {
                '*' => re.push_str(".*"),
                '?' => re.push('.'),
                '[' => {
                    let mut j = i + usize::from(g.get(i) == Some(&'!'));
                    j += usize::from(g.get(j) == Some(&']'));
                    while j < g.len() && g[j] != ']' {
                        j += 1;
                    }
                    if j >= g.len() {
                        re.push_str(r"\[");
                        continue;
                    }
                    let (neg, body) = match &g[i..j] {
                        ['!', rest @ ..] => (true, rest),
                        all => (false, all),
                    };
                    re.push_str(if neg { "[^" } else { "[" });
                    for &ch in body {
                        if "[]\\&~^".contains(ch) {
                            re.push('\\');
                        }
                        re.push(ch);
                    }
                    re.push(']');
                    i = j + 1;
                }
                c => re.push_str(&regex::escape(&c.to_string())),
            }
        }
        re.push('$');
        let rx =
            Plain::new(&re).unwrap_or_else(|_| Plain::new(&format!("^{}$", regex::escape(glob))).expect("literal"));
        cache.insert(glob.to_string(), rx);
    }
    cache.get(glob).is_some_and(|rx| rx.is_match(&fold(name)))
}

fn git_files(root: &Path) -> Option<Vec<PathBuf>> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotePath=false", "-C"]).arg(root).args(["ls-files", "-z", "-co", "--exclude-standard", "."]);
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    crate::v2::hide(&mut cmd);
    let out = cmd.output().ok()?;
    let text = if out.status.success() { String::from_utf8_lossy(&out.stdout).into_owned() } else { String::new() };
    let mut names: Vec<&str> = text.split('\0').filter(|n| !n.is_empty()).collect();
    names.sort_unstable();
    names.dedup();
    let nested: Vec<&str> = names.iter().copied().filter(|n| n.ends_with('/')).collect();
    let inner: HashMap<&str, Option<Vec<PathBuf>>> = if nested.is_empty() {
        HashMap::new()
    } else {
        let done = Mutex::new(HashMap::new());
        let next = std::sync::atomic::AtomicUsize::new(0);
        thread::scope(|s| {
            for _ in 0..GIT_WORKERS.min(nested.len()) {
                s.spawn(|| {
                    while let Some(&n) = nested.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)) {
                        let got = git_files(&root.join(n));
                        done.lock().unwrap_or_else(PoisonError::into_inner).insert(n, got);
                    }
                });
            }
        });
        done.into_inner().unwrap_or_else(PoisonError::into_inner)
    };
    let mut found = Vec::new();
    for n in names {
        if n.ends_with('/') {
            found.extend(inner.get(n).cloned().flatten().unwrap_or_default());
        } else {
            found.push(root.join(n));
        }
    }
    (!found.is_empty()).then_some(found)
}

fn walk(dir: &Path, hidden: bool, skipdirs: &[String], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            dirs.push(name);
        } else {
            files.push(name);
        }
    }
    files.sort();
    out.extend(files.into_iter().filter(|n| hidden || !n.starts_with('.')).map(|n| dir.join(n)));
    dirs.sort();
    for d in dirs {
        if SKIP.contains(&d.as_str()) || (!hidden && d.starts_with('.')) || skipdirs.iter().any(|g| fnmatch(&d, g)) {
            continue;
        }
        walk(&dir.join(d), hidden, skipdirs, out);
    }
}

pub fn posix(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn rel(p: &Path, base: &Path) -> Option<String> {
    let parts: Vec<String> =
        p.strip_prefix(base).ok()?.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    Some(parts.join("/"))
}

pub fn candidates(
    root: &Path,
    base: &Path,
    single: bool,
    o: &Opts,
    globs: &[String],
    skipdirs: &[String],
) -> Vec<(PathBuf, String)> {
    let nice = |p: &Path| rel(p, base).unwrap_or_else(|| posix(p));
    if root.is_file() {
        return vec![(root.to_path_buf(), nice(root))];
    }
    let listed = git_files(root).unwrap_or_else(|| {
        let mut all = Vec::new();
        walk(root, o.hidden, skipdirs, &mut all);
        all
    });
    let take: Vec<&String> = globs.iter().filter(|g| !g.starts_with('!')).collect();
    let drop: Vec<&str> = globs.iter().filter_map(|g| g.strip_prefix('!')).collect();
    let mut out = Vec::new();
    for p in listed {
        let r = rel(&p, root).unwrap_or_else(|| posix(&p));
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dirs: Vec<&str> = r.split('/').collect();
        if !skipdirs.is_empty()
            && dirs[..dirs.len().saturating_sub(1)].iter().any(|part| skipdirs.iter().any(|g| fnmatch(part, g)))
        {
            continue;
        }
        if !take.is_empty() && !take.iter().any(|g| fnmatch(&name, g) || fnmatch(&r, g)) {
            continue;
        }
        if drop.iter().any(|g| fnmatch(&name, g) || fnmatch(&r, g)) {
            continue;
        }
        let show = if single { r } else { nice(&p) };
        out.push((p, show));
    }
    out
}

fn chars_at(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().count()
}

fn cut(s: &str, from: usize, to: usize) -> String {
    s.chars().skip(from).take(to.saturating_sub(from)).collect()
}

fn snippet(line: &str, m: Option<&Match>, rx: &Regex, keep: bool) -> String {
    let text = if keep { line.trim_end() } else { line.trim() }.replace(['\r', '\0'], " ");
    let len = text.chars().count();
    if len <= LONG {
        return text;
    }
    let Some(m) = m else { return format!("{}...", cut(&text, 0, LONG)) };
    let shift = if keep { 0 } else { line.chars().count() - line.trim_start().chars().count() };
    let (start, end) = (chars_at(line, m.start()), chars_at(line, m.end()));
    let a = start.saturating_sub(shift + SIDE);
    let b = len.min(a.max(end.saturating_sub(shift)) + SIDE).min(a + 2 * LONG);
    let more = rx.find_iter(line).count().saturating_sub(1);
    format!(
        "{}{}{}{}",
        if a > 0 { "..." } else { "" },
        cut(&text, a, b),
        if b < len { "..." } else { "" },
        if more > 0 { format!(" (+{more} more in this line)") } else { String::new() }
    )
}

pub type Shown = Vec<(usize, String, char)>;

pub fn hits<'a>(lines: impl Iterator<Item = &'a str>, rx: &Regex, o: &Opts) -> (usize, Shown) {
    let (before, after) = (o.before, o.after);
    let keep = before > 0 || after > 0;
    let limit = o.max.unwrap_or(usize::MAX);
    let mut buf: VecDeque<(usize, &str)> = VecDeque::new();
    let (mut left, mut shown, mut count, mut picked, mut last) = (0, Vec::new(), 0, 0, 0);
    for (no, line) in lines.enumerate().map(|(n, l)| (n + 1, l)) {
        let m = rx.find(line).ok().flatten();
        if m.is_none() == o.invert && picked < limit {
            picked += 1;
            let parts: Vec<Match> = if o.only {
                rx.find_iter(line).flatten().filter(|x| !x.as_str().is_empty()).collect()
            } else {
                Vec::new()
            };
            count += if o.only { parts.len() } else { 1 };
            if shown.len() < KEEP {
                if before > 0 {
                    shown.extend(
                        buf.iter().filter(|(n, _)| *n > last).map(|(n, t)| (*n, snippet(t, None, rx, true), '-')),
                    );
                }
                if o.only {
                    shown.extend(parts.iter().map(|x| (no, snippet(x.as_str(), None, rx, true), ':')));
                } else {
                    shown.push((no, snippet(line, m.as_ref(), rx, keep), ':'));
                }
                last = no;
            }
            buf.clear();
            left = after;
        } else if left > 0 {
            left -= 1;
            if shown.len() < KEEP {
                shown.push((no, snippet(line, None, rx, true), '-'));
                last = no;
            }
        } else if picked >= limit {
            break;
        } else if before > 0 {
            if buf.len() == before {
                buf.pop_front();
            }
            buf.push_back((no, line));
        }
    }
    (count, shown)
}

fn decode(raw: &[u8]) -> Option<String> {
    let head = &raw[..raw.len().min(HEAD)];
    if head.starts_with(b"\xff\xfe") || head.starts_with(b"\xfe\xff") {
        let le = head.starts_with(b"\xff\xfe");
        let units: Vec<u16> = raw[2..]
            .chunks(2)
            .map(|c| {
                if c.len() == 2 {
                    if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }
                } else {
                    0xFFFD
                }
            })
            .collect();
        return Some(String::from_utf16_lossy(&units));
    }
    if head.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(raw)).into_owned())
}

pub enum Scan {
    Found(usize, Shown),
    Binary,
    Big,
    Gone,
}

pub fn scan(path: &Path, rx: &Regex, pre: Option<&Regex>, o: &Opts) -> Scan {
    let Ok(meta) = fs::metadata(path) else { return Scan::Gone };
    if meta.len() > MAX_BYTES {
        return Scan::Big;
    }
    let Ok(raw) = fs::read(path) else { return Scan::Gone };
    let text = match decode(&raw) {
        Some(t) => t,
        None if o.text => String::from_utf8_lossy(&raw).into_owned(),
        None => return Scan::Binary,
    };
    let text = text.replace("\r\n", "\n");
    if !o.invert && pre.is_some_and(|p| !p.is_match(&text).unwrap_or(true)) {
        return Scan::Found(0, Vec::new());
    }
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let (n, shown) = hits(body.split('\n'), rx, o);
    Scan::Found(n, shown)
}
