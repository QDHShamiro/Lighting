use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

use lt_core::Fail;
use regex::Regex;

const FFMPEG_SPOTS: &[&str] = &[r"C:\ffmpeg\bin\ffmpeg.exe", "/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"];
const TICK: Duration = Duration::from_millis(50);
const ERR_CHARS: usize = 160;
const FETCH_WAIT: Duration = Duration::from_secs(30);

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("tag"));
static CUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d+:\d\d:\d\d[.,]\d+|\d+:\d\d[.,]\d+)\s+-->").expect("cue"));
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\W*(?:(?:intro |outro |background |upbeat |sad |soft )?(?:music|musik|applause|applaus|laughter|lachen|silence|stille)|♪+)\W*$",
    )
    .expect("noise")
});
static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(false).build().into());

pub type Seg = (f64, String);

pub struct Ran {
    pub ok: bool,
    pub bytes: Vec<u8>,
    pub out: String,
    pub err: String,
}

impl Ran {
    pub fn why(&self) -> String {
        last_line(&self.err)
    }
}

pub fn last_line(text: &str) -> String {
    let line = text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("unknown error");
    line.chars().take(ERR_CHARS).collect()
}

pub fn which(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let path = env::var_os("PATH")?;
    env::split_paths(&path).map(|dir| dir.join(&file)).find(|p| p.is_file())
}

pub fn ffmpeg(home: &Path) -> Option<PathBuf> {
    let own = home.join("bin").join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    which("ffmpeg")
        .or_else(|| own.is_file().then_some(own))
        .or_else(|| FFMPEG_SPOTS.iter().map(PathBuf::from).find(|p| p.is_file()))
}

pub fn need_ffmpeg(home: &Path) -> Result<PathBuf, Fail> {
    ffmpeg(home).ok_or_else(|| {
        Fail::hint(
            "ffmpeg is not installed",
            if cfg!(windows) { "winget install Gyan.FFmpeg" } else { "install ffmpeg" },
        )
    })
}

pub fn ytdlp(home: &Path) -> Result<PathBuf, Fail> {
    let own = home.join("bin").join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" });
    which("yt-dlp").or_else(|| own.is_file().then_some(own)).ok_or_else(|| {
        Fail::hint(
            "yt-dlp is not installed",
            if cfg!(windows) { "winget install yt-dlp.yt-dlp" } else { "install yt-dlp" },
        )
    })
}

fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut raw = Vec::new();
        let _ = pipe.read_to_end(&mut raw);
        raw
    })
}

fn wait(child: &mut Child, limit: Duration) -> Option<bool> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if start.elapsed() < limit => thread::sleep(TICK),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

pub fn run(exe: &Path, args: &[String], limit: Duration) -> Result<Ran, Fail> {
    let name = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::v2::hide(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| Fail::new(format!("could not start {name}: {e}")))?;
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let done = wait(&mut child, limit);
    let bytes = |h: Option<thread::JoinHandle<Vec<u8>>>| h.and_then(|h| h.join().ok()).unwrap_or_default();
    let (bytes, err) = (bytes(out), String::from_utf8_lossy(&bytes(err)).into_owned());
    let out = String::from_utf8_lossy(&bytes).into_owned();
    match done {
        Some(ok) => Ok(Ran { ok, bytes, out, err }),
        None => Err(Fail::new(format!("{name} took longer than {} s", limit.as_secs()))),
    }
}

pub fn fetch(url: &str) -> Result<Vec<u8>, Fail> {
    let mut resp = AGENT
        .get(url)
        .config()
        .timeout_global(Some(FETCH_WAIT))
        .build()
        .header("User-Agent", "Mozilla/5.0 wisply")
        .call()
        .map_err(|e| Fail::new(format!("download failed: {e}")))?;
    if resp.status().as_u16() != 200 {
        return Err(Fail::new(format!("download failed: HTTP {}", resp.status().as_u16())));
    }
    resp.body_mut().with_config().limit(u64::MAX).read_to_vec().map_err(|e| Fail::new(format!("download failed: {e}")))
}

pub fn clock(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn seconds(text: Option<&str>) -> Result<Option<f64>, ()> {
    let Some(text) = text.filter(|t| !t.is_empty()) else { return Ok(None) };
    text.split(':')
        .try_fold(0.0, |total, part| part.trim().parse::<f64>().map(|v| total * 60.0 + v))
        .map(Some)
        .map_err(|_| ())
}

fn stamp(t: &str) -> f64 {
    let parts: Vec<f64> = t.replace(',', ".").split(':').filter_map(|p| p.parse().ok()).collect();
    parts.iter().fold(0.0, |total, v| total * 60.0 + v)
}

pub fn noise(text: &str) -> bool {
    NOISE.is_match(text)
}

pub fn vtt_segments(text: &str) -> Vec<Seg> {
    let mut segs: Vec<Seg> = Vec::new();
    let text = text.replace('\r', "");
    for block in text.split("\n\n") {
        let rows: Vec<&str> = block.lines().collect();
        let Some(at) = rows.iter().position(|r| r.contains("-->")) else { continue };
        let Some(m) = CUE.captures(rows[at]) else { continue };
        let start = stamp(&m[1]);
        for line in &rows[at + 1..] {
            let line = TAG.replace_all(line, "").split_whitespace().collect::<Vec<_>>().join(" ");
            if !line.is_empty() && segs.last().is_none_or(|(_, last)| *last != line) {
                segs.push((start, line));
            }
        }
    }
    segs
}

pub fn lines_of(segs: &[Seg], offset: f64, lo: Option<f64>, hi: Option<f64>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut last = String::new();
    for (start, text) in segs {
        let t = start + offset;
        if lo.is_some_and(|lo| t < lo - 1.0) || hi.is_some_and(|hi| t > hi) {
            continue;
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() || noise(&text) || text == last {
            continue;
        }
        out.push(format!("{} {text}", clock(t)));
        last = text;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captions_read_like_2x() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\n<c>Hello</c> there\n\n00:01.500 --> 00:04.000\nHello there\n\n1:02:03.000 --> 1:02:05.000\n[Music]\nbye\n";
        let segs = vtt_segments(vtt);
        assert_eq!(segs, vec![(1.0, "Hello there".into()), (3723.0, "[Music]".into()), (3723.0, "bye".into())]);
        assert_eq!(lines_of(&segs, 0.0, None, None), vec!["0:01 Hello there", "1:02:03 bye"]);
        assert_eq!(lines_of(&segs, 0.0, Some(100.0), None), vec!["1:02:03 bye"]);
        assert_eq!(seconds(Some("1:30")), Ok(Some(90.0)));
        assert!(seconds(Some("x")).is_err());
        assert_eq!(clock(3723.0), "1:02:03");
        assert!(noise("♪♪") && noise("[Applause]") && !noise("music is great"));
        assert_eq!(last_line("a\nlast one\n\n"), "last one");
    }
}
