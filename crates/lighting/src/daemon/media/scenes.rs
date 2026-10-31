use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use image::{DynamicImage, RgbImage};
use lt_core::Fail;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::super::web::{distinct, thumb};
use super::listen::{self, Heard};
use super::tools::{self, Seg};
use super::watch::{Job, Kind};

const KEEP: &[&str] = &["title", "uploader", "duration", "chapters", "webpage_url", "width", "height", "language"];
const PROBE_WAIT: Duration = Duration::from_secs(120);
const FILE_PROBE_WAIT: Duration = Duration::from_secs(60);
const VIDEO_WAIT: Duration = Duration::from_secs(300);
const FRAME_WAIT: Duration = Duration::from_secs(90);
const STREAM_S: f64 = 10800.0;
const SHORT_S: f64 = 180.0;
const DOWNLOAD_MAX_S: f64 = 900.0;
const READ_WIDTH: u32 = 960;
const MAX_HEIGHT: i64 = 720;
const GRABBERS: usize = 4;
const VIDEO_FORMAT: &str = "bv*[height<=720][ext=mp4]/bv*[height<=720]/b[height<=720]/b";
const TRY_PAGE: &str = "wisply open <url>, then wisply watch e40 (plays it in the browser)";

static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Duration: (\d+):(\d\d):(\d\d(?:\.\d+)?)").expect("duration"));
static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Video:.*?(\d{2,5})x(\d{2,5})").expect("size"));

pub type Tile = (f64, RgbImage);

fn load(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn save(path: &Path, data: &Value) {
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, data.to_string()).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

fn board_of(info: &Value) -> Value {
    let formats = info["formats"].as_array().cloned().unwrap_or_default();
    let best = formats
        .iter()
        .filter(|f| {
            f["format_note"] == "storyboard" && f["fragments"].is_array() && f["columns"].is_i64() && f["rows"].is_i64()
        })
        .max_by_key(|f| f["width"].as_i64().unwrap_or(0));
    let Some(best) = best else { return Value::Null };
    let base = best["fragment_base_url"].as_str().or(best["url"].as_str()).unwrap_or("").to_string();
    let frags: Vec<Value> = best["fragments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|fr| {
            let url = fr["url"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("{base}{}", fr["path"].as_str().unwrap_or("")));
            json!([url, fr["duration"].as_f64().unwrap_or(0.0)])
        })
        .collect();
    json!({"rows": best["rows"], "cols": best["columns"], "fps": best["fps"], "frags": frags})
}

fn subs_of(info: &Value) -> Value {
    let want: Vec<&str> = [info["language"].as_str(), Some("de"), Some("en")].into_iter().flatten().collect();
    let mut out = Map::new();
    for (kind, tag) in [("subtitles", ""), ("automatic_captions", "~auto")] {
        for (code, tracks) in info[kind].as_object().into_iter().flatten() {
            if !tag.is_empty() && !want.contains(&code.as_str()) && !code.ends_with("-orig") {
                continue;
            }
            let url =
                tracks.as_array().into_iter().flatten().find(|t| t["ext"] == "vtt").and_then(|t| t["url"].as_str());
            if let Some(url) = url {
                out.insert(format!("{code}{tag}"), json!(url));
            }
        }
    }
    Value::Object(out)
}

pub fn sub_order(tracks: &[String], lang: Option<&str>, original: Option<&str>) -> Vec<String> {
    let first: Vec<&str> = [lang, original, Some("de"), Some("en")].into_iter().flatten().collect();
    let manual: Vec<&String> = tracks.iter().filter(|k| !k.ends_with("~auto")).collect();
    let auto: Vec<&String> = tracks.iter().filter(|k| k.ends_with("~auto")).collect();
    let mut ranked: Vec<String> = Vec::new();
    let mut add = |k: &String| {
        if !ranked.contains(k) {
            ranked.push(k.clone());
        }
    };
    for x in &first {
        manual.iter().filter(|k| k.as_str() == *x || k.starts_with(&format!("{x}-"))).for_each(|k| add(k));
    }
    manual.iter().for_each(|k| add(k));
    auto.iter().filter(|k| k.ends_with("-orig~auto") && lang.is_none_or(|l| k.starts_with(l))).for_each(|k| add(k));
    for x in &first {
        auto.iter().filter(|k| k.as_str() == format!("{x}~auto")).for_each(|k| add(k));
    }
    auto.iter().filter(|k| k.ends_with("-orig~auto")).for_each(|k| add(k));
    ranked
}

fn stream_of(info: &Value) -> Value {
    let formats = info["formats"].as_array().cloned().unwrap_or_default();
    let best = formats
        .iter()
        .filter(|f| {
            let h = f["height"].as_i64().unwrap_or(0);
            !matches!(f["vcodec"].as_str(), None | Some("none"))
                && matches!(f["protocol"].as_str(), Some("https" | "http"))
                && h > 0
                && h <= MAX_HEIGHT
        })
        .max_by_key(|f| f["height"].as_i64().unwrap_or(0));
    let Some(best) = best else { return Value::Null };
    let heads: String = best["http_headers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| format!("{k}: {}\r\n", v.as_str().unwrap_or("")))
        .collect();
    json!([best["url"], heads])
}

fn probe_url(home: &Path, url: &str) -> Result<Value, Fail> {
    let args = ["-J", "--no-playlist", "--no-warnings", url].map(String::from);
    let ran = tools::run(&tools::ytdlp(home)?, &args, PROBE_WAIT)?;
    let info: Value = serde_json::from_str(&ran.out)
        .map_err(|_| Fail::hint(format!("could not read the video: {}", ran.why()), TRY_PAGE))?;
    let mut meta: Map<String, Value> = KEEP.iter().map(|k| (k.to_string(), info[*k].clone())).collect();
    meta.insert("board".into(), board_of(&info));
    meta.insert("subs".into(), subs_of(&info));
    meta.insert("stream".into(), stream_of(&info));
    meta.insert("at".into(), json!(lt_memory::now()));
    Ok(Value::Object(meta))
}

fn probe_file(home: &Path, path: &Path) -> Result<Value, Fail> {
    let args = ["-hide_banner", "-i", path.display().to_string().as_str()].map(String::from);
    let ran = tools::run(&tools::need_ffmpeg(home)?, &args, FILE_PROBE_WAIT)?;
    let (dur, size) = (DURATION.captures(&ran.err), SIZE.captures(&ran.err));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if dur.is_none() && size.is_none() {
        return Err(Fail::new(format!("not a video ffmpeg can read: {name}")));
    }
    let n = |m: &regex::Captures, i: usize| m[i].parse::<f64>().unwrap_or(0.0);
    let duration = dur.map_or(0.0, |m| n(&m, 1) * 3600.0 + n(&m, 2) * 60.0 + n(&m, 3));
    Ok(json!({"title": name, "duration": duration, "width": size.as_ref().map(|m| n(m, 1) as i64),
        "height": size.as_ref().map(|m| n(m, 2) as i64), "audio": ran.err.contains("Audio:"), "at": lt_memory::now()}))
}

pub fn meta_of(home: &Path, job: &Job, dir: &Path) -> Result<Value, Fail> {
    let path = dir.join("meta.json");
    let old = load(&path).filter(|m| {
        matches!(job.kind, Kind::File(_)) || lt_memory::now() - m["at"].as_f64().unwrap_or(0.0) <= STREAM_S
    });
    if let Some(meta) = old {
        return Ok(meta);
    }
    let meta = match &job.kind {
        Kind::Url(url) => probe_url(home, url)?,
        Kind::File(file) => probe_file(home, file)?,
    };
    save(&path, &meta);
    Ok(meta)
}

fn page_subs(meta: &Value, lang: Option<&str>) -> Option<Heard> {
    let tracks = meta["subs"].as_object()?;
    let keys: Vec<String> = tracks.keys().cloned().collect();
    for key in sub_order(&keys, lang, meta["language"].as_str()) {
        let Some(url) = tracks[&key].as_str() else { continue };
        let Ok(raw) = tools::fetch(url) else { continue };
        let segs = tools::vtt_segments(&String::from_utf8_lossy(&raw));
        if !segs.is_empty() {
            let lang = key.split('~').next().unwrap_or("").replace("-orig", "");
            let engine = if key.ends_with("~auto") { "captions (auto)" } else { "captions" };
            return Some(Heard { segs, lang, engine: engine.into() });
        }
    }
    None
}

fn heard_json(h: &Heard) -> Value {
    json!([h.segs, h.lang, h.engine])
}

fn heard_from(v: &Value) -> Option<Heard> {
    let segs: Vec<Seg> = serde_json::from_value(v[0].clone()).ok()?;
    Some(Heard { segs, lang: v[1].as_str().unwrap_or("?").into(), engine: v[2].as_str().unwrap_or("").into() })
}

pub fn speech(home: &Path, job: &Job, meta: &Value, dir: &Path, lo: f64, hi: Option<f64>) -> Result<Heard, Fail> {
    let zoomed = job.lo.is_some() || job.hi.is_some();
    let whole = dir.join("speech.json");
    if let Some(h) = load(&whole).as_ref().and_then(heard_from) {
        return Ok(h);
    }
    let lang = job.lang.as_deref();
    match &job.kind {
        Kind::Url(url) => {
            let found = page_subs(meta, lang).or_else(|| listen::captions(home, url, lang, dir).ok().flatten());
            if let Some(h) = found {
                save(&whole, &heard_json(&h));
                return Ok(h);
            }
        }
        Kind::File(_) if meta["audio"] == false => {
            return Ok(Heard { segs: Vec::new(), lang: "?".into(), engine: "no sound track".into() });
        }
        Kind::File(_) => {}
    }
    let tag = format!("{}-{}", lo as i64, hi.unwrap_or(0.0) as i64);
    let part = dir.join(format!("speech-{tag}.json"));
    if zoomed && let Some(h) = load(&part).as_ref().and_then(heard_from) {
        return Ok(h);
    }
    let tmp = dir.join(if zoomed { format!("a-{tag}") } else { "a-all".into() });
    let _ = fs::create_dir_all(&tmp);
    let (path, offset) = match &job.kind {
        Kind::File(file) => (listen::shrink(home, file, &tmp), 0.0),
        Kind::Url(url) => {
            let (a, b) = if zoomed { (Some(lo), hi) } else { (None, None) };
            let got = listen::fetch_audio(home, url, &tmp, a, b)?;
            (listen::shrink(home, &got, &tmp), if zoomed { lo } else { 0.0 })
        }
    };
    let mut h = listen::transcribe(home, &path, lang, job.local)?;
    h.segs.iter_mut().for_each(|s| s.0 += offset);
    save(if zoomed && matches!(job.kind, Kind::Url(_)) { &part } else { &whole }, &heard_json(&h));
    Ok(h)
}

pub fn spread(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let span = (hi - lo).max(0.1);
    (0..n).map(|i| lo + span * (i as f64 + 0.5) / n as f64).collect()
}

fn board_tiles(board: &Value, times: &[f64], dir: &Path) -> Result<Vec<Tile>, Fail> {
    let frags: Vec<(String, f64)> = serde_json::from_value(board["frags"].clone()).unwrap_or_default();
    let (rows, cols) =
        (board["rows"].as_u64().unwrap_or(1).max(1) as u32, board["cols"].as_u64().unwrap_or(1).max(1) as u32);
    let mut starts = Vec::with_capacity(frags.len());
    let mut t = 0.0;
    for (_, d) in &frags {
        starts.push(t);
        t += d;
    }
    let mut tiles = Vec::new();
    for at in times {
        let i = starts.iter().rposition(|s| s <= at).unwrap_or(0);
        let Some((url, len)) = frags.get(i) else { continue };
        let path = dir.join(format!("sb-{i}.jpg"));
        if !path.is_file() {
            fs::write(&path, tools::fetch(url)?).map_err(|e| Fail::new(e.to_string()))?;
        }
        let Ok(img) = image::open(&path) else { continue };
        let img = img.to_rgb8();
        let (w, h) = (img.width() / cols, img.height() / rows);
        let per = board["fps"].as_f64().filter(|f| *f > 0.0).map_or(len / f64::from(rows * cols), |f| 1.0 / f);
        let have = if per > 0.0 { ((len / per).round() as u32).clamp(1, rows * cols) } else { rows * cols };
        let j = if per > 0.0 { (((at - starts[i]) / per) as u32).min(have - 1) } else { 0 };
        let (r, c) = (j / cols, j % cols);
        tiles.push((*at, image::imageops::crop_imm(&img, c * w, r * h, w, h).to_image()));
    }
    Ok(tiles)
}

fn media_of(home: &Path, job: &Job, meta: &Value, dir: &Path, stream: bool) -> Result<(String, Option<String>), Fail> {
    let url = match &job.kind {
        Kind::File(file) => return Ok((file.display().to_string(), None)),
        Kind::Url(url) => url,
    };
    let dur = meta["duration"].as_f64().unwrap_or(0.0);
    let live = meta["stream"]
        .as_array()
        .filter(|s| s.len() == 2)
        .map(|s| (s[0].as_str().unwrap_or("").to_string(), s[1].as_str().map(String::from)));
    if stream
        && dur > SHORT_S
        && let Some(live) = live.clone()
    {
        return Ok(live);
    }
    if dur <= DOWNLOAD_MAX_S {
        if let Some(have) = listen::first_file(dir, "video.") {
            return Ok((have.display().to_string(), None));
        }
        let out = dir.join("video.%(ext)s").display().to_string();
        let mut args: Vec<String> =
            ["-f", VIDEO_FORMAT, "--no-playlist", "--quiet", "--no-warnings", "-o", out.as_str()]
                .map(String::from)
                .to_vec();
        if let Some(ff) = tools::ffmpeg(home) {
            args.extend(["--ffmpeg-location".into(), ff.display().to_string()]);
        }
        args.push(url.clone());
        let ran = tools::run(&tools::ytdlp(home)?, &args, VIDEO_WAIT)?;
        return listen::first_file(dir, "video.")
            .map(|p| (p.display().to_string(), None))
            .ok_or_else(|| Fail::new(format!("could not download the video: {}", ran.why())));
    }
    live.ok_or_else(|| Fail::hint("no video stream to take frames from", TRY_PAGE))
}

pub fn download(home: &Path, job: &Job, meta: &Value, dir: &Path) -> Result<PathBuf, Fail> {
    let (path, _) = media_of(home, job, meta, dir, false)?;
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err(Fail::hint(
            "the video is too long to download",
            "wisply watch <url> --from 1:00 --to 5:00 --frames",
        ));
    }
    Ok(path)
}

fn grab(home: &Path, src: &str, heads: Option<&str>, times: &[f64], dir: &Path) -> Result<Vec<Tile>, Fail> {
    let exe = tools::need_ffmpeg(home)?;
    let one = |at: f64| -> Option<Tile> {
        let path = dir.join(format!("f-{}-{READ_WIDTH}.jpg", (at * 1000.0) as i64));
        if !path.is_file() {
            let mut args: Vec<String> =
                ["-y", "-loglevel", "error", "-ss", format!("{at:.3}").as_str()].map(String::from).to_vec();
            if let Some(h) = heads.filter(|h| !h.is_empty()) {
                args.extend(["-headers".into(), h.to_string()]);
            }
            let scale = format!("scale={READ_WIDTH}:-2");
            args.extend(
                ["-i", src, "-frames:v", "1", "-vf", scale.as_str(), "-q:v", "4", path.display().to_string().as_str()]
                    .map(String::from),
            );
            let _ = tools::run(&exe, &args, FRAME_WAIT);
        }
        image::open(&path).ok().map(|img| (at, img.to_rgb8()))
    };
    let one = &one;
    let mut got: Vec<Tile> = Vec::new();
    for chunk in times.chunks(GRABBERS) {
        thread::scope(|s| {
            let jobs: Vec<_> = chunk.iter().map(|at| s.spawn(move || one(*at))).collect();
            got.extend(jobs.into_iter().filter_map(|j| j.join().ok().flatten()));
        });
    }
    Ok(got)
}

fn pick(tiles: Vec<Tile>, n: usize) -> Vec<Tile> {
    if tiles.len() <= n {
        return tiles;
    }
    let thumbs: Vec<Vec<u8>> = tiles.iter().map(|(_, img)| thumb(&DynamicImage::ImageRgb8(img.clone()))).collect();
    let keep = distinct(&thumbs, n);
    tiles.into_iter().enumerate().filter(|(i, _)| keep.contains(i)).map(|(_, t)| t).collect()
}

fn frames_at(home: &Path, job: &Job, meta: &Value, dir: &Path, times: &[f64]) -> Result<Vec<Tile>, Fail> {
    let (src, heads) = media_of(home, job, meta, dir, true)?;
    let mut got = grab(home, &src, heads.as_deref(), times, dir)?;
    if got.is_empty() && heads.is_some() {
        let (src, heads) = media_of(home, job, meta, dir, false)?;
        got = grab(home, &src, heads.as_deref(), times, dir)?;
    }
    if got.is_empty() {
        return Err(Fail::hint("ffmpeg got no frames from the video", TRY_PAGE));
    }
    Ok(got)
}

pub fn pictures(
    home: &Path,
    job: &Job,
    meta: &Value,
    dir: &Path,
    lo: f64,
    hi: Option<f64>,
) -> Result<(Vec<Tile>, bool, &'static str), Fail> {
    if !job.at.is_empty() {
        return Ok((frames_at(home, job, meta, dir, &job.at)?, true, "at"));
    }
    let end = hi.or(meta["duration"].as_f64()).filter(|e| *e > 0.0);
    let Some(end) = end else {
        return Err(Fail::hint("the video has no known length (a live stream?)", "wisply watch <url> --at 0:10,0:20"));
    };
    let zoomed = job.lo.is_some() || job.hi.is_some();
    if meta["board"].is_object() && !zoomed && !job.text {
        let tiles = board_tiles(&meta["board"], &spread(lo, end, job.count * 3), dir)?;
        if !tiles.is_empty() {
            return Ok((pick(tiles, job.count), false, "storyboard"));
        }
    }
    let times = spread(lo, end, job.count * if zoomed { 1 } else { 2 });
    Ok((pick(frames_at(home, job, meta, dir, &times)?, job.count), true, "video"))
}

#[cfg(windows)]
pub fn read_text(tiles: &[Tile]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for (at, img) in tiles {
        let Ok(lines) = super::super::desk::ocr_lines(img) else { break };
        let new: Vec<String> = lines
            .into_iter()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|line| {
                let key: String = line.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
                key.chars().count() >= 3 && seen.insert(key)
            })
            .collect();
        if !new.is_empty() {
            rows.push(format!("{} {}", tools::clock(*at), new.join(" | ")));
        }
    }
    rows
}

#[cfg(not(windows))]
pub fn read_text(_: &[Tile]) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_rank_like_2x_and_frames_spread_evenly() {
        let tracks: Vec<String> = ["en~auto", "de", "fr", "en-orig~auto", "en-US"].map(String::from).to_vec();
        assert_eq!(sub_order(&tracks, None, Some("en")), vec!["en-US", "de", "fr", "en-orig~auto", "en~auto"]);
        assert_eq!(sub_order(&tracks, Some("fr"), None)[0], "fr");
        assert_eq!(spread(0.0, 10.0, 2), vec![2.5, 7.5]);
        let info = json!({"formats": [
            {"format_note": "storyboard", "fragments": [{"path": "a.jpg", "duration": 10.0}], "columns": 5, "rows": 5, "width": 80, "fragment_base_url": "https://x/"},
            {"vcodec": "avc1", "protocol": "https", "height": 480, "url": "u480", "http_headers": {"A": "b"}},
            {"vcodec": "avc1", "protocol": "https", "height": 1080, "url": "u1080"}
        ], "subtitles": {"de": [{"ext": "vtt", "url": "s"}]}, "automatic_captions": {"ja": [{"ext": "vtt", "url": "j"}]}});
        assert_eq!(board_of(&info)["frags"], json!([["https://x/a.jpg", 10.0]]));
        assert_eq!(stream_of(&info), json!(["u480", "A: b\r\n"]));
        assert_eq!(subs_of(&info), json!({"de": "s"}));
    }
}
