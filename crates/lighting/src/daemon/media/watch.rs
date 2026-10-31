use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use image::DynamicImage;
use lt_core::Fail;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::super::channels::ledger;
use super::super::hosts::lock;
use super::super::web::sheet;
use super::gemini;
use super::listen::Heard;
use super::scenes::{self, Tile};
use super::tools::{clock, noise};

const CACHE_HOURS: u64 = 24;
const SKIM_CHARS: usize = 2400;
const FULL_CHARS: usize = 12000;
const TEXT_CHARS: usize = 1200;
const PARAGRAPH_S: f64 = 30.0;
const SKIM_ROW: usize = 150;
const SKIM_CUT: usize = 140;
const OVERLAP_WORDS: usize = 12;
const CHAPTERS: usize = 15;
const DESCRIBE: &str = "Describe this video for someone who cannot see or hear it. Plain text, no tables. Sections: \
Summary (2-3 sentences). Scenes: one line each, starting with m:ss. Text on screen: the important text, with m:ss. \
Speech: the key things said, with m:ss. Under 350 words.";
const ASK: &str = "Answer the question about this video. Cite m:ss times. Plain text, short.\nQuestion: ";

static YOUTUBE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^https?://(?:www\.|m\.)?(?:youtube\.com/(?:watch\?(?:.*&)?v=|shorts/|live/)|youtu\.be/)([\w-]{6,})",
    )
    .expect("youtube")
});
type Gates = Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>;

static BOXES: LazyLock<Gates> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub enum Kind {
    Url(String),
    File(PathBuf),
}

pub struct Job {
    pub kind: Kind,
    pub key: String,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub at: Vec<f64>,
    pub full: bool,
    pub text: bool,
    pub lang: Option<String>,
    pub local: bool,
    pub count: usize,
    pub question: Option<String>,
    pub fps: Option<f64>,
    pub frames: bool,
}

pub struct Done {
    pub lines: Vec<String>,
    pub sheet: Option<(DynamicImage, String, usize)>,
}

pub fn youtube(url: &str) -> Option<String> {
    YOUTUBE.captures(url).map(|m| format!("https://www.youtube.com/watch?v={}", &m[1]))
}

fn mime(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("avi") => "video/x-msvideo",
        Some("wmv") => "video/x-ms-wmv",
        Some("flv") => "video/x-flv",
        Some("mpg" | "mpeg") => "video/mpeg",
        Some("3gp") => "video/3gpp",
        _ => "video/mp4",
    }
}

fn short(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.strip_prefix("www.").unwrap_or(rest).chars().take(70).collect()
}

fn source(job: &Job) -> String {
    match &job.kind {
        Kind::Url(url) => short(url),
        Kind::File(file) => file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

fn prune(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else { return };
    let limit = Duration::from_secs(CACHE_HOURS * 3600);
    for entry in entries.flatten() {
        let old =
            entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
        if old.is_some_and(|age| age > limit) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn by_gemini(home: &Path, job: &Job, dir: &Path) -> Result<Vec<String>, Fail> {
    let g = gemini::gemini(home)?;
    let prompt = job.question.as_deref().map_or(DESCRIBE.to_string(), |q| format!("{ASK}{}", lt_memory::scrub(q)));
    let shape = format!("{:?}|{:?}|{:?}|{}", job.lo, job.hi, job.fps, job.text);
    let memo = dir.join(format!("answer-{:016x}.txt", crate::hook::fnv(&[&prompt, &shape, &g.model])));
    let head = format!("[watch] {}", source(job));
    if let Ok(text) = fs::read_to_string(&memo) {
        return Ok(vec![head, format!("answer (Gemini {}, cached):", g.model), text]);
    }
    let mut part = match &job.kind {
        Kind::Url(url) if youtube(url).is_some() => gemini::linked(None, &youtube(url).unwrap_or_default()),
        kind => {
            let path = match kind {
                Kind::File(file) => file.clone(),
                Kind::Url(_) => scenes::download(home, job, &scenes::meta_of(home, job, dir)?, dir)?,
            };
            g.file_part(&path, mime(&path), &dir.join("gemini-file.json"))?
        }
    };
    let mut cut = Map::new();
    if let Some(lo) = job.lo {
        cut.insert("start_offset".into(), json!(format!("{lo}s")));
    }
    if let Some(hi) = job.hi {
        cut.insert("end_offset".into(), json!(format!("{hi}s")));
    }
    if let Some(fps) = job.fps {
        cut.insert("fps".into(), json!(fps));
    }
    if !cut.is_empty() {
        part["video_metadata"] = Value::Object(cut);
    }
    let a = g.ask(home, vec![part, json!({"text": prompt})], job.text)?;
    let _ = fs::write(&memo, &a.text);
    Ok(vec![head, format!("answer (Gemini {}, {} tokens, call {} today):", g.model, a.tokens, a.calls), a.text])
}

pub fn run(home: &Path, job: &Job) -> Result<Done, Fail> {
    let root = home.join("watch");
    prune(&root);
    let dir = root.join(format!("{:016x}", crate::hook::fnv(&[&job.key])));
    fs::create_dir_all(&dir).map_err(|e| Fail::new(format!("cannot use {} ({e})", dir.display())))?;
    let gate = lock(&BOXES).entry(dir.clone()).or_default().clone();
    let _busy = lock(&gate);
    let mut notes = Vec::new();
    if !job.frames && !job.local && job.at.is_empty() {
        match by_gemini(home, job, &dir) {
            Ok(lines) => {
                ledger::note(home, "video", "gemini", true, "");
                return Ok(Done { lines, sheet: None });
            }
            Err(f) => {
                ledger::note(home, "video", "gemini", false, &f.msg);
                notes.push(format!("Gemini: {}; frames + transcript instead", f.text().trim_start_matches("err: ")))
            }
        }
    }
    let done = frames_and_words(home, job, &dir);
    ledger::note(home, "video", "frames", done.is_ok(), done.as_ref().err().map_or("", |f| f.msg.as_str()));
    let mut done = done?;
    done.lines.extend(notes.into_iter().map(|n| format!("! {n}")));
    Ok(done)
}

fn frames_and_words(home: &Path, job: &Job, dir: &Path) -> Result<Done, Fail> {
    let meta = scenes::meta_of(home, job, dir)?;
    let dur = meta["duration"].as_f64().unwrap_or(0.0);
    let lo = job.lo.unwrap_or(0.0).max(0.0);
    let hi = [job.hi, Some(dur)].into_iter().flatten().filter(|x| *x > 0.0).reduce(f64::min);
    let (heard, seen) = thread::scope(|s| {
        let heard = s.spawn(|| scenes::speech(home, job, &meta, dir, lo, hi));
        let seen = s.spawn(|| scenes::pictures(home, job, &meta, dir, lo, hi));
        let heard = heard.join().unwrap_or_else(|_| Err(Fail::new("worker stopped")));
        (heard, seen.join().unwrap_or_else(|_| Err(Fail::new("worker stopped"))))
    });
    let mut notes = Vec::new();
    let heard = heard.map_err(|f| notes.push(format!("sound: {}", f.text().trim_start_matches("err: ")))).ok();
    let (tiles, readable, how) = match seen {
        Ok(got) => got,
        Err(f) => {
            notes.push(format!("frames: {}", f.text().trim_start_matches("err: ")));
            (Vec::new(), false, "")
        }
    };
    let text = if readable { scenes::read_text(&tiles) } else { Vec::new() };
    Ok(compose(job, &meta, &tiles, how, &text, heard.as_ref(), lo, hi, notes))
}

fn merge(words: &mut Vec<String>, new: Vec<String>) {
    let most = words.len().min(new.len()).min(OVERLAP_WORDS);
    let n = (1..=most).rev().find(|&n| words[words.len() - n..] == new[..n]).unwrap_or(0);
    words.extend(new.into_iter().skip(n));
}

pub fn paragraphs(segs: &[(f64, String)], lo: f64, hi: Option<f64>, window: f64) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    let (mut cur, mut start): (Vec<String>, Option<f64>) = (Vec::new(), None);
    for (t, text) in segs {
        if *t < lo - 1.0 || hi.is_some_and(|hi| *t > hi) {
            continue;
        }
        let words: Vec<String> = text.split_whitespace().map(String::from).collect();
        if words.is_empty() || noise(&words.join(" ")) {
            continue;
        }
        if start.is_none_or(|s| t - s >= window) {
            if let Some(s) = start.filter(|_| !cur.is_empty()) {
                out.push((s, cur.join(" ")));
            }
            (cur, start) = (Vec::new(), Some(*t));
        }
        merge(&mut cur, words);
    }
    if let Some(s) = start.filter(|_| !cur.is_empty()) {
        out.push((s, cur.join(" ")));
    }
    out
}

pub fn transcript(segs: &[(f64, String)], lo: f64, hi: Option<f64>, budget: usize) -> (String, Option<f64>) {
    let paras = paragraphs(segs, lo, hi, PARAGRAPH_S);
    let text: Vec<String> = paras.iter().map(|(t, p)| format!("{} {p}", clock(*t))).collect();
    let text = text.join("\n");
    if text.len() <= budget || paras.is_empty() {
        return (text, None);
    }
    let span = (paras[paras.len() - 1].0 - paras[0].0).max(1.0);
    let window = (span / (budget / SKIM_ROW).max(4) as f64).max(PARAGRAPH_S);
    let rows: Vec<String> = paragraphs(segs, lo, hi, window)
        .into_iter()
        .map(|(t, p)| {
            let cut = if p.chars().count() <= SKIM_CUT {
                p
            } else {
                let head: String = p.chars().take(SKIM_CUT).collect();
                format!("{} ...", head.rsplit_once(' ').map_or(head.as_str(), |(a, _)| a))
            };
            format!("{} {cut}", clock(t))
        })
        .collect();
    (rows.join("\n"), Some(window))
}

#[allow(clippy::too_many_arguments)]
fn compose(
    job: &Job,
    meta: &Value,
    tiles: &[Tile],
    how: &str,
    text: &[String],
    heard: Option<&Heard>,
    lo: f64,
    hi: Option<f64>,
    notes: Vec<String>,
) -> Done {
    let title: String = meta["title"].as_str().unwrap_or("video").chars().take(90).collect();
    let by =
        meta["uploader"].as_str().map(|u| format!(" - {}", u.chars().take(40).collect::<String>())).unwrap_or_default();
    let link = if matches!(job.kind, Kind::Url(_)) { format!(" - {}", source(job)) } else { String::new() };
    let mut lines = vec![format!("[watch] {title}{by} - {}{link}", clock(meta["duration"].as_f64().unwrap_or(0.0)))];
    let chapters = meta["chapters"].as_array().cloned().unwrap_or_default();
    if !chapters.is_empty() {
        let shown: Vec<String> = chapters
            .iter()
            .take(CHAPTERS)
            .map(|c| {
                format!(
                    "{} {}",
                    clock(c["start_time"].as_f64().unwrap_or(0.0)),
                    c["title"].as_str().unwrap_or("").chars().take(40).collect::<String>()
                )
            })
            .collect();
        let more =
            if chapters.len() > CHAPTERS { format!(" | +{} more", chapters.len() - CHAPTERS) } else { String::new() };
        lines.push(format!("chapters: {}{more}", shown.join(" | ")));
    }
    let picture = (!tiles.is_empty()).then(|| {
        let imgs: Vec<DynamicImage> = tiles.iter().map(|(_, img)| DynamicImage::ImageRgb8(img.clone())).collect();
        let stamps: Vec<String> = tiles.iter().map(|(t, _)| clock(*t)).collect();
        let kind = match how {
            "storyboard" => "scenes from the video's preview images",
            "at" => "frames at the asked times",
            _ => "scenes",
        };
        let desc = format!("{} {kind} at {}", tiles.len(), stamps.join(" "));
        (DynamicImage::ImageRgb8(sheet(&imgs, &stamps)), desc, lines.len())
    });
    if !text.is_empty() {
        let mut shown = text.join("\n");
        if shown.len() > TEXT_CHARS {
            let cut = shown.char_indices().take_while(|(i, _)| *i < TEXT_CHARS).last().map_or(0, |(i, _)| i);
            shown = format!("{}\n...", shown[..cut].rsplit_once('\n').map_or(&shown[..cut], |(a, _)| a));
        }
        lines.extend(["text on screen:".to_string(), shown]);
    } else if how == "storyboard" {
        lines.push(
            "text on screen: not read from preview images (wisply watch ... --text reads the real frames)".into(),
        );
    }
    if let Some(h) = heard {
        let zoomed = job.lo.is_some() || job.hi.is_some() || !job.at.is_empty();
        let budget = if job.full || zoomed { FULL_CHARS } else { SKIM_CHARS };
        let (lo, hi) = match (job.at.iter().copied().reduce(f64::min), job.at.iter().copied().reduce(f64::max)) {
            (Some(a), Some(b)) => ((a - 15.0).max(0.0), Some(b + 15.0)),
            _ => (lo, hi),
        };
        let (said, window) = transcript(&h.segs, lo, hi, budget);
        let skim = window
            .map(|w| format!("; skim: start of every {}; all of it: --full or --from/--to", clock(w)))
            .unwrap_or_default();
        lines.push(format!("transcript ({}, {}{skim}):", h.lang, h.engine));
        lines.push(if said.is_empty() { "(no speech: only music or silence)".into() } else { said });
    }
    lines.extend(notes.into_iter().map(|n| format!("! {n}")));
    if job.lo.is_none() && job.hi.is_none() && job.at.is_empty() {
        let src = match &job.kind {
            Kind::Url(url) => url.clone(),
            Kind::File(_) => source(job),
        };
        lines.push(format!("closer: wisply watch \"{src}\" --from 1:00 --to 2:00 | exact frames: --at 1:23,2:45"));
    }
    Done { lines, sheet: picture }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_links_and_transcripts_read_like_2x() {
        assert_eq!(
            youtube("https://youtu.be/abcdefgh?t=3").as_deref(),
            Some("https://www.youtube.com/watch?v=abcdefgh")
        );
        assert_eq!(
            youtube("https://m.youtube.com/shorts/xyz123_-").as_deref(),
            Some("https://www.youtube.com/watch?v=xyz123_-")
        );
        assert_eq!(
            youtube("https://www.youtube.com/watch?list=1&v=QQQQQQ").as_deref(),
            Some("https://www.youtube.com/watch?v=QQQQQQ")
        );
        assert!(youtube("https://vimeo.com/123").is_none());
        let segs: Vec<(f64, String)> =
            vec![(0.0, "a b c".into()), (2.0, "b c d".into()), (40.0, "[Music]".into()), (41.0, "later".into())];
        assert_eq!(paragraphs(&segs, 0.0, None, 30.0), vec![(0.0, "a b c d".into()), (41.0, "later".into())]);
        assert_eq!(transcript(&segs, 0.0, None, 100), ("0:00 a b c d\n0:41 later".into(), None));
        let long: Vec<(f64, String)> = (0..40).map(|i| (f64::from(i) * 30.0, "word ".repeat(40))).collect();
        let (skim, window) = transcript(&long, 0.0, None, 600);
        assert!(window.is_some_and(|w| w > 30.0) && skim.lines().all(|l| l.ends_with(" ...")), "{skim}");
        assert_eq!(mime(Path::new("a.MOV")), "video/quicktime");
    }
}
