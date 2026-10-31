use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use image::RgbImage;
use image::imageops::{self, FilterType};
use wisply_core::{Call, Fail, out_file};

use super::super::Out;
use super::super::media::{self, tools};
use super::super::web::{self, out_path};
use super::grab;
use super::win::{self, Rect};
use super::{Desk, Halt, windowish};

const FPS: u32 = 15;
const DEFAULT_S: f64 = 10.0;
const MAX_S: f64 = 60.0;
const WIDTH: u32 = 1280;
const CRF: &str = "30";
const SPARE: Duration = Duration::from_secs(30);
const KINDS: &[&str] = &["mp4"];
const HINT: &str = "wisply shot w3 --seconds 10 --out C:/shots/clip.mp4";
const SHORTER: &str = "wisply shot w3 --seconds 5 (a shorter video)";

fn even(n: f64) -> u32 {
    ((n.max(2.0) as u32) / 2 * 2).max(2)
}

fn encode(out: &Path) -> Vec<String> {
    let mut args: Vec<String> =
        ["-c:v", "libx264", "-preset", "veryfast", "-crf", CRF, "-pix_fmt", "yuv420p", "-movflags", "+faststart"]
            .map(String::from)
            .to_vec();
    args.push(out.display().to_string());
    args
}

fn fit(img: RgbImage, (w, h): (u32, u32)) -> RgbImage {
    if img.dimensions() == (w, h) { img } else { imageops::resize(&img, w, h, FilterType::Triangle) }
}

fn failed(err: &str, code: Option<i32>) -> Fail {
    let last = err.lines().rev().find(|l| !l.trim().is_empty()).map(str::trim);
    let why = last.map_or_else(|| format!("ffmpeg exit {}", code.unwrap_or(-1)), |l| l.chars().take(200).collect());
    Fail::new(format!("video failed: {why}"))
}

fn screen_video(exe: &Path, area: Rect, secs: u64, width: u32, out: &Path) -> Result<(u32, u32), Fail> {
    let (w, h) = (even(f64::from(area.2 - area.0)), even(f64::from(area.3 - area.1)));
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "gdigrab".into(),
        "-framerate".into(),
        FPS.to_string(),
        "-offset_x".into(),
        area.0.to_string(),
        "-offset_y".into(),
        area.1.to_string(),
        "-video_size".into(),
        format!("{w}x{h}"),
        "-i".into(),
        "desktop".into(),
        "-t".into(),
        secs.to_string(),
        "-vf".into(),
        format!("scale='min({width},iw)':-2"),
    ];
    args.extend(encode(out));
    let ran = tools::run(exe, &args, Duration::from_secs(secs) + SPARE)?;
    if !ran.ok || !out.is_file() {
        return Err(failed(&ran.err, None));
    }
    let k = (f64::from(width) / f64::from(w)).min(1.0);
    Ok((even(f64::from(w) * k), even(f64::from(h) * k)))
}

fn window_video(
    exe: &Path,
    hwnd: isize,
    first: RgbImage,
    secs: u64,
    width: u32,
    out: &Path,
) -> Result<(u32, u32), Fail> {
    let k = (f64::from(width) / f64::from(first.width())).min(1.0);
    let size = (even(f64::from(first.width()) * k), even(f64::from(first.height()) * k));
    let mut args: Vec<String> = [
        "-y",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-s",
        &format!("{}x{}", size.0, size.1),
        "-r",
        &FPS.to_string(),
        "-i",
        "-",
        "-t",
        &secs.to_string(),
    ]
    .map(String::from)
    .to_vec();
    args.extend(encode(out));
    let mut cmd = Command::new(exe);
    cmd.args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    crate::v2::hide(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| Fail::new(format!("could not start ffmpeg: {e}")))?;
    let errors = child.stderr.take().map(|mut s| {
        thread::spawn(move || {
            let mut text = String::new();
            let _ = s.read_to_string(&mut text);
            text
        })
    });
    let step = Duration::from_secs_f64(1.0 / f64::from(FPS));
    let mut last = fit(first, size);
    if let Some(mut pipe) = child.stdin.take() {
        let t0 = Instant::now();
        for i in 0..secs * u64::from(FPS) {
            let due = t0 + step.mul_f64(i as f64);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
            if Instant::now().saturating_duration_since(due) < step
                && win::alive(hwnd)
                && let Some(img) = grab::print_window(hwnd)
            {
                last = fit(img, size);
            }
            if pipe.write_all(last.as_raw()).is_err() {
                break;
            }
        }
    }
    let status = child.wait().map_err(|e| Fail::new(format!("ffmpeg did not finish: {e}")))?;
    let err = errors.and_then(|t| t.join().ok()).unwrap_or_default();
    if !status.success() || !out.is_file() {
        return Err(failed(&err, status.code()));
    }
    Ok(size)
}

pub fn video(x: &mut Desk, c: &Call) -> Result<Out, Fail> {
    let secs = match c.val("seconds").filter(|v| !v.is_empty()) {
        Some(v) => v.trim().parse::<f64>().map_err(|_| Fail::hint("--seconds takes a number", HINT))?,
        None => DEFAULT_S,
    }
    .clamp(1.0, MAX_S)
    .round() as u64;
    let first = c.pos.first().map(String::as_str);
    if let Some(p) = first
        && p != "screen"
        && !windowish(p)
    {
        return Err(Fail::hint(
            "a video films the screen or a window",
            format!(
                "wisply shot screen --seconds {secs}  (a browser: wisply windows -f brave, then shot w<N> --seconds {secs})"
            ),
        ));
    }
    let exe = tools::need_ffmpeg(&x.d.home)?;
    let (hwnd, area) = x.region(first.or(Some("screen"))).map_err(|h| match h {
        Halt::Fail(f) => f,
        Halt::Busy => Fail::new("busy"),
    })?;
    let width = web::number(c, "width")?.map_or(WIDTH, |w| w.clamp(64, 3840) as u32);
    let key = if hwnd == 0 { "video-screen".to_string() } else { format!("video-{hwnd}") };
    let out: PathBuf = out_path(x.ctx, c, KINDS, HINT)?.unwrap_or_else(|| out_file(&x.d.home.join("out"), &key, "mp4"));
    let start = if hwnd != 0 && !c.on("screen-area") { grab::print_window(hwnd) } else { None };
    let how = if start.is_some() { ", window only, also when covered" } else { "" };
    let work = move || {
        let size = match start {
            Some(img) => window_video(&exe, hwnd, img, secs, width, &out)?,
            None => screen_video(&exe, area, secs, width, &out)?,
        };
        let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        let amount = if bytes >= 1_000_000 {
            format!("{:.1} MB", bytes as f64 / 1e6)
        } else {
            format!("{} KB", (bytes as f64 / 1e3).round().max(1.0))
        };
        let shown = out.to_string_lossy().replace('\\', "/");
        Ok(format!("video {shown} ({secs} s, {}x{}, {amount}{how})", size.0, size.1))
    };
    let pending =
        media::background("video", Duration::from_secs(secs) + SPARE, SHORTER, work, |_, _, text: String| Ok(text));
    Ok(Out::Wait(pending))
}
