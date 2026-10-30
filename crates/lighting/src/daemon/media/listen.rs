use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use base64::Engine;
use lt_core::Fail;
use serde_json::{Value, json};

use super::super::STOPPED;
use super::super::config;
use super::super::hosts::{Called, Host};
use super::super::memory::chat::api_key;
use super::gemini::{self, Answer};
use super::tools::{self, Seg, lines_of};

const CLOUDS: [(&str, &str, &str, &str); 2] = [
    ("GROQ_API_KEY", "https://api.groq.com/openai/v1/audio/transcriptions", "whisper-large-v3-turbo", "groq"),
    ("OPENAI_API_KEY", "https://api.openai.com/v1/audio/transcriptions", "whisper-1", "openai"),
];
const CAPTION_WAIT: Duration = Duration::from_secs(120);
const AUDIO_WAIT: Duration = Duration::from_secs(240);
const SHRINK_WAIT: Duration = Duration::from_secs(300);
const ASR_WAIT: Duration = Duration::from_secs(600);
pub const MAX_LIVE_S: f64 = 600.0;
const LIVE_SPARE_S: f64 = 45.0;
const LIVE_DEFAULT_S: f64 = 60.0;
const LIVE_MIN_S: f64 = 5.0;
const EMPTY_BYTES: usize = 1000;
const NO_SPEECH: &str = "(no speech: only music or silence; on-screen text: lighting frames)";
const NO_ASR: &str = "set GROQ_API_KEY (free key: console.groq.com/keys) or lighting config whisper_api <LocalAI url>";
const ASK_AUDIO: &str = "Answer the question about this audio. Quote what is said with m:ss times where it helps. Plain text, short.\nQuestion: ";
const ASK_TEXT: &str =
    "Answer the question about this transcript (lines start with m:ss). Plain text, short, cite times.\nQuestion: ";

static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(false).build().into());

pub struct Live {
    pub host: Arc<Host>,
    pub tab: Option<String>,
    pub group: String,
    pub reference: Option<String>,
    pub duration: f64,
}

pub enum Src {
    File(PathBuf),
    Url(String),
    Page(Live),
}

pub struct Job {
    pub src: Src,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub lang: Option<String>,
    pub local: bool,
    pub live: bool,
    pub question: Option<String>,
}

pub struct Heard {
    pub segs: Vec<Seg>,
    pub lang: String,
    pub engine: String,
}

pub fn captions(home: &Path, url: &str, lang: Option<&str>, dir: &Path) -> Result<Option<Heard>, Fail> {
    let mut langs: Vec<String> = lang.map(|l| format!("{l}.*")).into_iter().collect();
    langs.extend(["de.*".to_string(), "en.*".to_string()]);
    let out = dir.join("sub").display().to_string();
    let args = [
        "--skip-download",
        "--write-subs",
        "--write-auto-subs",
        "--sub-langs",
        langs.join(",").as_str(),
        "--sub-format",
        "vtt",
        "--no-playlist",
        "--quiet",
        "--no-warnings",
        "-o",
        out.as_str(),
        url,
    ];
    tools::run(&tools::ytdlp(home)?, &args.map(String::from), CAPTION_WAIT)?;
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("sub"))
                && p.extension().is_some_and(|e| e == "vtt")
        })
        .collect();
    let order: Vec<String> = lang
        .map(|l| format!(".{l}."))
        .into_iter()
        .chain(["-orig.", ".en.", ".de.", ".en-", ".de-"].map(String::from))
        .collect();
    let rank = |p: &PathBuf| {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        order.iter().position(|part| name.contains(part.as_str())).unwrap_or(99)
    };
    files.sort_by_key(|p| (rank(p), p.clone()));
    for file in files {
        let segs = tools::vtt_segments(&String::from_utf8_lossy(&fs::read(&file).unwrap_or_default()));
        if !segs.is_empty() {
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let got = name.split('.').nth(1).filter(|_| name.matches('.').count() >= 2).map(String::from);
            return Ok(Some(Heard {
                segs,
                lang: got.or(lang.map(String::from)).unwrap_or("?".into()),
                engine: "captions".into(),
            }));
        }
    }
    Ok(None)
}

pub fn fetch_audio(home: &Path, url: &str, dir: &Path, lo: Option<f64>, hi: Option<f64>) -> Result<PathBuf, Fail> {
    let out = dir.join("audio.%(ext)s").display().to_string();
    let mut args: Vec<String> =
        ["-f", "bestaudio/best", "--no-playlist", "--quiet", "--no-warnings", "-o", out.as_str()]
            .map(String::from)
            .to_vec();
    if (lo.is_some() || hi.is_some()) && tools::ffmpeg(home).is_some() {
        let end = hi.map_or("inf".to_string(), |h| h.to_string());
        args.extend(["--download-sections".into(), format!("*{}-{end}", lo.unwrap_or(0.0))]);
    }
    args.push(url.to_string());
    let ran = tools::run(&tools::ytdlp(home)?, &args, AUDIO_WAIT)?;
    first_file(dir, "audio.").ok_or_else(|| Fail::new(format!("could not download the audio: {}", ran.why())))
}

pub fn first_file(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            name.starts_with(prefix) && !name.ends_with(".part") && fs::metadata(p).is_ok_and(|m| m.len() > 0)
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

pub fn shrink(home: &Path, path: &Path, dir: &Path) -> PathBuf {
    let Some(exe) = tools::ffmpeg(home) else { return path.to_path_buf() };
    let out = dir.join("speech.ogg");
    let args = [
        "-y",
        "-loglevel",
        "error",
        "-i",
        path.display().to_string().as_str(),
        "-vn",
        "-ac",
        "1",
        "-ar",
        "16000",
        "-c:a",
        "libopus",
        "-b:a",
        "24k",
        out.display().to_string().as_str(),
    ]
    .map(String::from);
    let ok = tools::run(&exe, &args, SHRINK_WAIT).is_ok_and(|r| r.ok);
    if ok && fs::metadata(&out).is_ok_and(|m| m.len() > 0) { out } else { path.to_path_buf() }
}

fn multipart(fields: &[(&str, &str)], filename: &str, data: &[u8]) -> (Vec<u8>, String) {
    let boundary = format!("lighting{:x}{:x}", std::process::id(), lt_memory::now().to_bits());
    let mut body = Vec::with_capacity(data.len() + 1024);
    for (k, v) in fields {
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    body.extend(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
            .as_bytes(),
    );
    body.extend(data);
    body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    (body, format!("multipart/form-data; boundary={boundary}"))
}

fn num(seg: &Value, key: &str) -> f64 {
    seg.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

pub fn spoken(seg: &Value) -> bool {
    let (none, logprob) = (num(seg, "no_speech_prob"), num(seg, "avg_logprob"));
    none < 0.6 && !(logprob < -1.0 && none > 0.3) && num(seg, "compression_ratio") < 2.4
}

pub fn read_segments(data: &Value, engine: &str, lang: Option<&str>) -> Heard {
    let raw: Vec<&Value> =
        data.get("segments").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let mut segs: Vec<Seg> = raw
        .iter()
        .filter(|s| spoken(s))
        .map(|s| (num(s, "start"), s.get("text").and_then(Value::as_str).unwrap_or("").to_string()))
        .collect();
    if raw.is_empty() {
        segs = vec![(0.0, data.get("text").and_then(Value::as_str).unwrap_or("").to_string())];
    }
    let unsure = raw.iter().filter(|s| !spoken(s) || num(s, "no_speech_prob") > 0.3).count();
    let note =
        if !raw.is_empty() && unsure * 2 >= raw.len() { ", mostly music: sung words may be misheard" } else { "" };
    let lang = data.get("language").and_then(Value::as_str).or(lang).unwrap_or("?").to_string();
    Heard { segs, lang, engine: format!("{engine}{note}") }
}

fn post_audio(
    url: &str,
    auth: Option<String>,
    model: &str,
    lang: Option<&str>,
    path: &Path,
) -> Result<Value, (u16, String)> {
    let mut fields = vec![("model", model), ("response_format", "verbose_json"), ("temperature", "0")];
    if let Some(lang) = lang {
        fields.push(("language", lang));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
    let raw = fs::read(path).map_err(|e| (0, e.to_string()))?;
    let (body, ctype) = multipart(&fields, &name, &raw);
    let mut req = AGENT.post(url).config().timeout_global(Some(ASR_WAIT)).build().header("Content-Type", ctype);
    if let Some(auth) = auth {
        req = req.header("Authorization", auth);
    }
    let mut resp = req.send(&body[..]).map_err(|e| (0, e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(u64::MAX).read_to_string().unwrap_or_default();
    if status != 200 {
        return Err((status, tools::last_line(&text)));
    }
    serde_json::from_str(&text).map_err(|_| (status, "no JSON".into()))
}

pub fn transcribe(home: &Path, path: &Path, lang: Option<&str>, local: bool) -> Result<Heard, Fail> {
    if !local {
        for (env, url, model, name) in CLOUDS {
            let Some(key) = api_key(env) else { continue };
            match post_audio(url, Some(format!("Bearer {key}")), model, lang, path) {
                Ok(data) => return Ok(read_segments(&data, &format!("{name} {model}"), lang)),
                Err((401, _)) => return Err(Fail::hint(format!("the {name} key was refused"), NO_ASR)),
                Err((413, _)) => {
                    return Err(Fail::hint(
                        format!("the audio is too big for {name}"),
                        "listen to a part with --from/--to",
                    ));
                }
                Err(_) => break,
            }
        }
    }
    let cfg = config::load(home);
    let base = config::text(&cfg, "whisper_api");
    let base = base.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    if base.is_empty() {
        return Err(Fail::hint("no speech-to-text service", NO_ASR));
    }
    let model = config::text(&cfg, "whisper_model");
    let url = format!("{base}/v1/audio/transcriptions");
    match post_audio(&url, None, &model, lang, path) {
        Ok(data) => Ok(read_segments(&data, &format!("{model} at {base}"), lang)),
        Err((status, why)) => Err(Fail::new(format!("speech-to-text at {base} failed ({status}: {why})"))),
    }
}

fn capture(live: &Live, ms: f64, lo: Option<f64>, dir: &Path) -> Result<(PathBuf, f64, bool), Fail> {
    let args = json!({"ref": live.reference, "op": "record", "ms": ms as i64, "from": lo});
    let wait = Duration::from_secs_f64(ms / 1000.0 + LIVE_SPARE_S);
    let msg = match live.host.call("media-audio", args, live.tab.as_deref(), wait, Some(live.group.as_str())) {
        Ok(msg) => msg,
        Err(Called::Timeout) => return Err(Fail::new("recording timed out")),
        Err(Called::Aborted) => return Err(Fail::uncounted(STOPPED)),
    };
    if msg.get("ok").and_then(Value::as_bool) == Some(false) || msg.get("error").is_some_and(|e| !e.is_null()) {
        let why = msg.get("error").and_then(Value::as_str).unwrap_or("recording failed");
        return Err(Fail::new(why.to_string()));
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(msg.get("audio").and_then(Value::as_str).unwrap_or(""))
        .unwrap_or_default();
    if raw.len() < EMPTY_BYTES {
        return Err(Fail::hint(
            "the recording is empty (the video did not play)",
            "click play once in the browser, then listen again",
        ));
    }
    let path = dir.join("live.webm");
    fs::write(&path, raw).map_err(|e| Fail::new(format!("cannot save the recording ({e})")))?;
    let start = msg.get("start").and_then(Value::as_f64).unwrap_or(0.0);
    Ok((path, start, msg.get("blocked").and_then(Value::as_bool).unwrap_or(false)))
}

fn answered(a: &Answer, model: &str, took: f64) -> String {
    format!("answer (Gemini {model}, {} tokens, call {} today, {took:.1} s):\n{}", a.tokens, a.calls, a.text)
}

fn ask_text(home: &Path, question: &str, lines: &[String], took: Instant) -> Result<String, Fail> {
    let g = gemini::gemini(home)?;
    let prompt = format!("{ASK_TEXT}{}\n\n{}", lt_memory::scrub(question), lines.join("\n"));
    let a = g.ask(home, vec![json!({"text": prompt})], false)?;
    Ok(answered(&a, &g.model, took.elapsed().as_secs_f64()))
}

pub fn run(home: &Path, job: &Job, dir: &Path) -> Result<String, Fail> {
    let t0 = Instant::now();
    let (lo, hi, lang) = (job.lo, job.hi, job.lang.as_deref());
    let (path, offset, note) = match &job.src {
        Src::File(file) => (shrink(home, file, dir), 0.0, String::new()),
        Src::Url(url) => {
            if let Ok(Some(heard)) = captions(home, url, lang, dir) {
                let lines = lines_of(&heard.segs, 0.0, lo, hi);
                if let Some(q) = &job.question {
                    return ask_text(home, q, &lines, t0);
                }
                let took = t0.elapsed().as_secs_f64();
                return Ok(format!("transcript ({}, captions, {took:.1} s):\n{}", heard.lang, lines.join("\n")));
            }
            let path = fetch_audio(home, url, dir, lo, hi)?;
            (shrink(home, &path, dir), lo.unwrap_or(0.0), String::new())
        }
        Src::Page(live) => {
            let span = match (hi, live.duration) {
                (Some(hi), _) => hi - lo.unwrap_or(0.0),
                (None, d) if d > 0.0 => (d - lo.unwrap_or(0.0)).max(LIVE_MIN_S),
                _ => LIVE_DEFAULT_S,
            };
            let (path, start, blocked) = capture(live, span.min(MAX_LIVE_S) * 1000.0, lo, dir)?;
            let note = if blocked { "recorded live, the page kept it muted" } else { "recorded live" };
            (shrink(home, &path, dir), start, note.to_string())
        }
    };
    let note = if note.is_empty() { String::new() } else { format!(", {note}") };
    if let Some(q) = &job.question {
        let g = gemini::gemini(home)?;
        let part = g.file_part(&path, "audio/ogg", &dir.join("gemini-file.json"))?;
        let a = g.ask(home, vec![part, json!({"text": format!("{ASK_AUDIO}{}", lt_memory::scrub(q))})], false)?;
        return Ok(answered(&a, &g.model, t0.elapsed().as_secs_f64()));
    }
    let heard = transcribe(home, &path, lang, job.local)?;
    let lines = lines_of(&heard.segs, offset, None, None);
    let text = if lines.is_empty() { NO_SPEECH.to_string() } else { lines.join("\n") };
    let took = t0.elapsed().as_secs_f64();
    Ok(format!("transcript ({}, {}{note}, {took:.1} s):\n{text}", heard.lang, heard.engine))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_segments_drop_music_and_note_it() {
        let data = json!({"language": "en", "segments": [
            {"start": 1.0, "text": " hi", "no_speech_prob": 0.1, "avg_logprob": -0.2, "compression_ratio": 1.2},
            {"start": 5.0, "text": " la la", "no_speech_prob": 0.7, "avg_logprob": -1.5, "compression_ratio": 1.0},
            {"start": 9.0, "text": " oh", "no_speech_prob": 0.35, "avg_logprob": -0.5, "compression_ratio": 1.0}
        ]});
        let heard = read_segments(&data, "groq w", None);
        assert_eq!(heard.segs, vec![(1.0, " hi".into()), (9.0, " oh".into())]);
        assert_eq!(heard.engine, "groq w, mostly music: sung words may be misheard");
        assert_eq!(heard.lang, "en");
        let plain = read_segments(&json!({"text": "all"}), "x", Some("de"));
        assert_eq!((plain.segs, plain.lang), (vec![(0.0, "all".into())], "de".into()));
        let (body, ctype) = multipart(&[("model", "m")], "a.ogg", b"DATA");
        let text = String::from_utf8_lossy(&body);
        let boundary = ctype.split("boundary=").nth(1).unwrap();
        assert!(text.starts_with(&format!("--{boundary}\r\n")) && text.ends_with(&format!("--{boundary}--\r\n")));
        assert!(
            text.contains("name=\"model\"\r\n\r\nm\r\n")
                && text.contains("filename=\"a.ogg\"")
                && text.contains("DATA")
        );
    }
}
