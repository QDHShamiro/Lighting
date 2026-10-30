pub mod gemini;
mod listen;
mod scenes;
pub mod tools;
mod watch;

use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use lt_core::{Call, Fail, Flag, out_file};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::channels::ledger;
use super::ctx::Ctx;
use super::{Daemon, Out, Pending, config, route, web};
use listen::{Job as ListenJob, Live, Src};
#[cfg(windows)]
pub use listen::{multipart, spoken, whisper_service};
use watch::{Job, Kind};

const LISTEN_WAIT: Duration = Duration::from_secs(900);
const WATCH_WAIT: Duration = Duration::from_secs(900);
const ASK_WAIT: Duration = Duration::from_secs(300);
const TICK: Duration = Duration::from_millis(500);
const LISTEN_CHARS: usize = 1800;
const WATCH_CHARS: usize = 9000;
const CUES_TIMEOUT: f64 = 20.0;
const FRAMES_DEFAULT: i64 = 12;
const FRAMES_MIN: i64 = 4;
const FRAMES_MAX: i64 = 24;
const FPS_MAX: f64 = 24.0;
const SHORTER: &str = "try a shorter part with --from/--to";
const TIMES_HINT: &str = "lighting watch <link> --from 1:30 --to 2:00";
const ASK_PROMPT: &str =
    "Answer the question about this screenshot. Name visible labels exactly as written. Plain text, short.\nQuestion: ";

static SINGLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^https?://(?:www\.|m\.)?(?:youtube\.com/(?:watch|shorts/|live/)|youtu\.be/|tiktok\.com/@[^/]+/video/|vimeo\.com/\d|(?:x|twitter)\.com/[^/]+/status/|instagram\.com/(?:reels?|p)/|twitch\.tv/[^/]+/clip/|clips\.twitch\.tv/)",
    )
    .expect("single")
});
static VIDEO_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\.(mp4|mov|mkv|webm|avi|m4v|wmv|flv|mpe?g|ts|gif|mp3|m4a|wav|ogg|opus|flac)$").expect("file")
});
static LOCAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([a-zA-Z]:|\.{1,2}[\\/]|[\\/~])").expect("local"));

pub fn background<T: Send + 'static>(
    name: &'static str,
    limit: Duration,
    hint: &'static str,
    work: impl FnOnce() -> Result<T, Fail> + Send + 'static,
    mut finish: impl FnMut(&Daemon, &mut Ctx, T) -> Result<String, Fail> + Send + 'static,
) -> Pending {
    let (tx, rx) = mpsc::channel();
    let started = thread::Builder::new().name(name.into()).spawn(move || {
        let got = panic::catch_unwind(AssertUnwindSafe(work))
            .unwrap_or_else(|p| Err(Fail::new(format!("{name} failed: {}", super::panic_text(&*p)))));
        let _ = tx.send(got);
    });
    let mut broken = started.is_err();
    Pending::new(limit, TICK, move |d, ctx, last| {
        if broken {
            return Err(Fail::new(format!("{name} could not start a worker")));
        }
        match rx.try_recv() {
            Ok(Ok(v)) => finish(d, ctx, v).map(Some),
            Ok(Err(f)) => Err(f),
            Err(TryRecvError::Empty) if last => {
                Err(Fail::hint(format!("{name} took longer than {} s", limit.as_secs()), hint))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                broken = true;
                Err(Fail::new(format!("{name} failed: the worker stopped")))
            }
        }
    })
}

fn scratch(home: &Path, name: &str) -> PathBuf {
    let dir = out_file(&home.join("out"), name, "d");
    let _ = fs::create_dir_all(&dir);
    dir
}

type Times = (Option<f64>, Option<f64>, Vec<f64>);

fn times(c: &Call) -> Result<Times, Fail> {
    let bad = || Fail::hint("--from, --to and --at take seconds or m:ss", TIMES_HINT);
    let lo = tools::seconds(c.val("from")).map_err(|()| bad())?;
    let hi = tools::seconds(c.val("to")).map_err(|()| bad())?;
    let at = match c.val("at") {
        Some(list) => list
            .split(',')
            .filter(|x| !x.trim().is_empty())
            .map(|x| tools::seconds(Some(x.trim())).map(Option::unwrap_or_default).map_err(|()| bad()))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    Ok((lo, hi, at))
}

fn question(c: &Call) -> Option<String> {
    let q = c.pos.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
    (!q.trim().is_empty()).then(|| q.trim().to_string())
}

fn local_audio(d: &Daemon, c: &Call) -> bool {
    c.on("local") || config::text(&config::load(&d.home), "audio") == "local"
}

fn lang(c: &Call) -> Option<String> {
    c.val("lang").map(str::to_lowercase).filter(|l| !l.is_empty())
}

fn looks_like_url(target: &str) -> bool {
    !target.chars().any(char::is_whitespace)
        && (target.contains("://") || (target.contains('.') && !LOCAL.is_match(target) && !VIDEO_FILE.is_match(target)))
}

fn file_of(ctx: &Ctx, target: &str) -> Result<Option<PathBuf>, Fail> {
    if target.contains("://") {
        return Ok(None);
    }
    let path = web::local_file(ctx, target);
    if path.is_file() {
        return Ok(Some(path));
    }
    if VIDEO_FILE.is_match(target) && (LOCAL.is_match(target) || !target.contains(['/', '\\'])) {
        return Err(Fail::hint(
            format!("no file {target}"),
            "check the path (a relative path starts in the folder lighting runs from)",
        ));
    }
    Ok(None)
}

fn place(d: &Daemon, ctx: &mut Ctx) -> Result<Map<String, Value>, Fail> {
    let msg = web::call(d, ctx, "where", Map::new(), web::CALL_TIMEOUT)?;
    Ok(msg.get("where").and_then(Value::as_object).cloned().unwrap_or_default())
}

fn page_head(place: &Map<String, Value>) -> String {
    let title: String = web::text_of(place, "title").chars().take(60).collect();
    let url = web::text_of(place, "url");
    let url = url.split_once("://").map_or(url, |(_, r)| r);
    let url: String = url.strip_prefix("www.").unwrap_or(url).chars().take(70).collect();
    let tab = Some(web::text_of(place, "tab")).filter(|t| !t.is_empty()).unwrap_or("t?");
    format!("[{tab}] {title} - {url}")
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    match c.name.as_str() {
        "listen" => listen(d, ctx, c, String::new()),
        "frames" => with_sound(d, ctx, c),
        _ => watch(d, ctx, c),
    }
}

fn with_sound(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let mut quiet = c.clone();
    quiet.flags.remove("audio");
    let seen = match web::run(d, ctx, &quiet)? {
        Out::Text(text) => text,
        other => return Ok(other),
    };
    let mut call =
        Call { name: "listen".into(), pos: c.pos.first().cloned().into_iter().collect(), flags: c.flags.clone() };
    call.flags.remove("audio");
    match listen(d, ctx, &call, seen.clone()) {
        Ok(out) => Ok(out),
        Err(f) => Ok(Out::Text(format!("{seen}\naudio: {}", f.text()))),
    }
}

fn listen(d: &Daemon, ctx: &mut Ctx, c: &Call, before: String) -> Result<Out, Fail> {
    let (lo, hi, _) = times(c)
        .map_err(|_| Fail::hint("--from and --to take seconds or m:ss", "lighting listen --from 1:30 --to 2:00"))?;
    let target = c.pos.first().map(String::as_str).filter(|t| !t.is_empty());
    let mut job = ListenJob {
        src: Src::Url(String::new()),
        lo,
        hi,
        lang: lang(c),
        local: local_audio(d, c),
        live: c.on("live"),
        question: question(c),
    };
    let reference = target.filter(|t| web::REF_RE.is_match(t));
    let file = match target.filter(|_| reference.is_none()) {
        Some(t) => file_of(ctx, t)?,
        None => None,
    };
    let head = if let Some(file) = file {
        let head = format!("[file] {}", file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        job.src = Src::File(file);
        head
    } else if let Some(t) = target.filter(|t| reference.is_none() && looks_like_url(t)) {
        let url = web::normalize_url(t);
        web::leak_check(d, &url, c)?;
        job.src = Src::Url(url.clone());
        format!("[url] {}", url.chars().take(90).collect::<String>())
    } else {
        let here = place(d, ctx)?;
        let head = page_head(&here);
        let mut reference = reference.map(String::from);
        let mut duration = 0.0;
        if !job.live {
            let got =
                web::call(d, ctx, "media-audio", web::obj(json!({"ref": reference, "op": "cues"})), CUES_TIMEOUT)?;
            reference = got.get("ref").and_then(Value::as_str).map(String::from).or(reference);
            duration = got.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
            let segs: Vec<(f64, String)> = got
                .get("cues")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|cue| Some((cue.get(0)?.as_f64()?, cue.get(1)?.as_str()?.to_string())))
                .collect();
            if !segs.is_empty() && job.question.is_none() {
                let lines = tools::lines_of(&segs, 0.0, lo, hi).join("\n");
                let lang = got.get("lang").and_then(Value::as_str).unwrap_or("?");
                let text = format!(
                    "{before}{}{head}\ntranscript ({lang}, captions on the page):\n{lines}",
                    if before.is_empty() { "" } else { "\n" }
                );
                return Ok(Out::Text(web::capped(d, ctx, &text, "listen", None, LISTEN_CHARS)));
            }
        }
        let tab = Some(web::text_of(&here, "tab").to_string()).filter(|t| !t.is_empty());
        job.src = Src::Page(Live { host: web::ensure_host(d)?, tab, group: ctx.group.clone(), reference, duration });
        head
    };
    let home = d.home.clone();
    let dir = scratch(&home, "listen");
    let page = match &job.src {
        Src::Url(url) if !job.local && web::any_browser(d) => Some(url.clone()),
        _ => None,
    };
    let url_job = matches!(job.src, Src::Url(_));
    let fallback = page.is_some();
    let page_url = page.unwrap_or_default();
    let question = job.question.clone();
    let work = move || {
        let got = listen::run(&home, &job, &dir);
        let _ = fs::remove_dir_all(&dir);
        match got {
            Err(f) if fallback => Ok(Err(f)),
            other => other.map(Ok),
        }
    };
    let finish = move |d: &Daemon, ctx: &mut Ctx, got: Result<String, Fail>| {
        let text = match got {
            Ok(text) => {
                if url_job {
                    ledger::note(&d.home, "audio", "yt-dlp", true, "");
                }
                text
            }
            Err(f) => {
                ledger::note(&d.home, "audio", "yt-dlp", false, &f.msg);
                page_captions(d, ctx, &page_url, &f, (lo, hi), question.as_deref())?
            }
        };
        let lead = if before.is_empty() { String::new() } else { format!("{before}\n") };
        Ok(web::capped(d, ctx, &format!("{lead}{head}\n{text}"), "listen", None, LISTEN_CHARS))
    };
    Ok(Out::Wait(background("listen", LISTEN_WAIT, SHORTER, work, finish)))
}

fn page_captions(
    d: &Daemon,
    ctx: &mut Ctx,
    target: &str,
    failed: &Fail,
    (lo, hi): (Option<f64>, Option<f64>),
    question: Option<&str>,
) -> Result<String, Fail> {
    let t0 = std::time::Instant::now();
    let url = target;
    let live = Fail::hint(
        failed.msg.clone(),
        format!("lighting open {url}, then lighting listen --live (records the sound in the browser)"),
    );
    web::call(d, ctx, "open", web::obj(json!({"url": url, "new": true, "quiet": true})), CUES_TIMEOUT * 2.0)
        .map_err(|_| live.clone())?;
    let got = web::call(d, ctx, "media-audio", web::obj(json!({"ref": null, "op": "cues"})), CUES_TIMEOUT)
        .map_err(|_| live.clone())?;
    let segs: Vec<(f64, String)> = got
        .get("cues")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|cue| Some((cue.get(0)?.as_f64()?, cue.get(1)?.as_str()?.to_string())))
        .collect();
    if segs.is_empty() {
        ledger::note(&d.home, "audio", "browser", false, "no captions on the page");
        return Err(live);
    }
    ledger::note(&d.home, "audio", "browser", true, "");
    let lines = tools::lines_of(&segs, 0.0, lo, hi);
    if let Some(q) = question {
        return listen::ask_text(&d.home, q, &lines, t0);
    }
    let lang = got.get("lang").and_then(Value::as_str).unwrap_or("?");
    let why: String = failed.msg.chars().take(120).collect();
    Ok(format!("transcript ({lang}, captions on the page in the browser; yt-dlp: {why}):\n{}", lines.join("\n")))
}

fn number(c: &Call, key: &str) -> Result<Option<f64>, Fail> {
    match c.flags.get(key) {
        Some(Flag::Val(v)) => v
            .parse::<f64>()
            .map(Some)
            .map_err(|_| Fail::hint(format!("--{key} takes a number"), format!("lighting watch <link> --{key} 1"))),
        Some(Flag::On) => {
            Err(Fail::hint(format!("--{key} takes a number"), format!("lighting watch <link> --{key} 1")))
        }
        None => Ok(None),
    }
}

fn watch(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let (lo, hi, at) = times(c)?;
    let count = web::number(c, "count")?.unwrap_or(FRAMES_DEFAULT).clamp(FRAMES_MIN, FRAMES_MAX) as usize;
    let fps = number(c, "fps")?.map(|f| f.clamp(0.01, FPS_MAX));
    let target = c.pos.first().map(String::as_str).filter(|t| !t.is_empty());
    let kind = match target {
        Some(t) if web::REF_RE.is_match(t) => {
            let mut frames = Call { name: "frames".into(), pos: vec![t.to_string()], flags: c.flags.clone() };
            frames.flags.insert("scenes".into(), Flag::On);
            frames.flags.insert("audio".into(), Flag::On);
            return with_sound(d, ctx, &frames);
        }
        Some(t) => match file_of(ctx, t)? {
            Some(file) => Kind::File(file),
            None if looks_like_url(t) => {
                let url = web::normalize_url(t);
                web::leak_check(d, &url, c)?;
                Kind::Url(url)
            }
            None => {
                return Err(Fail::hint(
                    format!("no video file or link {t}"),
                    "lighting watch https://youtu.be/x | watch clip.mp4",
                ));
            }
        },
        None => {
            let url = web::text_of(&place(d, ctx)?, "url").to_string();
            if !SINGLE.is_match(&url) {
                return Err(Fail::hint(
                    "watch needs a link, a video file or a video on the page",
                    "lighting watch https://youtu.be/x | watch clip.mp4 | snap --media, then watch e40",
                ));
            }
            Kind::Url(url)
        }
    };
    let key = match &kind {
        Kind::Url(url) => url.clone(),
        Kind::File(file) => {
            let meta = fs::metadata(file).ok();
            let stamp = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            format!("{}|{}|{}", file.display(), meta.map_or(0, |m| m.len()), stamp.map_or(0, |s| s.as_secs()))
        }
    };
    let job = Job {
        kind,
        key,
        lo,
        hi,
        at,
        full: c.on("full"),
        text: c.on("text"),
        lang: lang(c),
        local: local_audio(d, c),
        count,
        question: question(c),
        fps,
        frames: c.on("frames"),
    };
    let home = d.home.clone();
    let pending = background(
        "watch",
        WATCH_WAIT,
        SHORTER,
        move || watch::run(&home, &job),
        |d, ctx, done| {
            let mut lines = done.lines;
            if let Some((img, desc, at)) = done.sheet {
                let saved = web::save_sheet(d, ctx, &img, "watch")?;
                lines.insert(at.min(lines.len()), format!("{}: {desc}", saved.replacen("shot ", "frames ", 1)));
            }
            Ok(web::capped(d, ctx, &lines.join("\n"), "watch", None, WATCH_CHARS))
        },
    );
    Ok(Out::Wait(pending))
}

pub fn ask_shot(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    let Some(q) = c.val("ask").filter(|q| !q.trim().is_empty()).map(String::from) else {
        return Err(Fail::hint("--ask needs a question", "lighting shot --ask \"what does the error say?\""));
    };
    let g = gemini::gemini(&d.home)?;
    let mut plain = c.clone();
    plain.flags.remove("ask");
    plain.flags.remove("if-changed");
    plain.flags.remove("threshold");
    let shot = match route::route(d, ctx, &plain)? {
        Out::Text(text) => text,
        other => return Ok(other),
    };
    let path = PathBuf::from(std::mem::take(&mut ctx.image));
    let raw = fs::read(&path).map_err(|_| Fail::new(format!("the screenshot was not saved ({shot})")))?;
    let home = d.home.clone();
    let work = move || {
        let parts =
            vec![gemini::inline("image/jpeg", &raw), json!({"text": format!("{ASK_PROMPT}{}", lt_memory::scrub(&q))})];
        let model = g.model.clone();
        g.ask(&home, parts, true).map(|a| (a, model))
    };
    let pending = background("shot --ask", ASK_WAIT, "ask again in a minute", work, move |_, _, (a, model)| {
        Ok(format!("{shot}\nanswer (Gemini {model}, {} tokens, call {} today):\n{}", a.tokens, a.calls, a.text))
    });
    Ok(Out::Wait(pending))
}

pub fn usage(home: &Path) -> Option<String> {
    gemini::usage(home)
}
