use std::time::{Duration, Instant};

use image::{DynamicImage, RgbImage};
use lt_core::{Call, Fail, OUT_CHARS};

use super::super::config;
use super::super::web::{SNAP_LINES, parse_ms, save_picture};
use super::super::words::{best_only, terms, tiers};
use super::super::{Out, Pending};
use super::grab;
use super::ocr;
use super::state;
use super::uia;
use super::win::{self, Rect};
use super::{Desk, Halt, Res, windowish};

const WAIT_MS: u64 = 30_000;
const OCR_EVERY_S: f64 = 1.5;
const POLL: Duration = Duration::from_millis(500);

fn lang(x: &Desk, c: &Call) -> String {
    c.val("lang").map(String::from).unwrap_or_else(|| config::text(&config::load(&x.d.home), "ocr_lang"))
}

fn grab_area(area: Rect, hwnd: isize) -> Res<RgbImage> {
    if hwnd != 0
        && hwnd != win::foreground()
        && let Some(img) = grab::print_window(hwnd)
    {
        return Ok(img);
    }
    grab::screen(area).ok_or_else(|| Fail::new("could not capture the screen").into())
}

fn app_text(hwnd: isize) -> String {
    let mut parts = Vec::new();
    for el in uia::collect(hwnd, true, None) {
        for v in el.texts() {
            if !v.trim().is_empty() {
                parts.push(v.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
    }
    parts.join(" ")
}

impl Desk<'_> {
    pub(super) fn region(&mut self, first: Option<&str>) -> Res<(isize, Rect)> {
        if first == Some("screen") {
            return Ok((0, win::virtual_screen()));
        }
        let hwnd = self.target(first)?;
        if win::minimized(hwnd) {
            let wref = self.wref(hwnd);
            return Err(Fail::hint(
                format!("{wref} is minimized, nothing to capture"),
                format!("lighting focus {wref}"),
            )
            .into());
        }
        Ok((hwnd, win::rect(hwnd)))
    }
}

pub fn read(x: &mut Desk, c: &Call) -> Res<String> {
    let (hwnd, area) = x.region(c.pos.first().map(String::as_str))?;
    let img = grab_area(area, hwnd)?;
    let lines = ocr::recognize(&img, &lang(x, c))?;
    let filt = c.val("f").filter(|f| !f.is_empty()).map(terms);
    x.st.ocr.clear();
    let mut rows: Vec<(Vec<u8>, String)> = Vec::new();
    for (i, (text, lx, ly, lw, lh)) in lines.iter().enumerate() {
        let cx = area.0 + (lx + lw / 2.0) as i32;
        let cy = area.1 + (ly + lh / 2.0) as i32;
        let r = format!("o{}", i + 1);
        x.st.ocr.push((r.clone(), text.clone(), cx, cy));
        let t = match &filt {
            Some(filt) => {
                let t = tiers(filt, &[text.as_str()]);
                if !t.iter().any(|v| *v > 0) {
                    continue;
                }
                t
            }
            None => Vec::new(),
        };
        rows.push((t, format!("{r} \"{}\" @{cx},{cy}", text.replace('"', "'"))));
    }
    let view: Vec<(Vec<u8>, &str)> = rows.iter().map(|(t, l)| (t.clone(), l.as_str())).collect();
    let out = best_only(&view);
    let head = if hwnd == 0 {
        format!("[screen] (screen text, {} lines)", lines.len())
    } else {
        x.header(hwnd, &format!(" (screen text, {} lines)", lines.len()))
    };
    let body = if out.is_empty() { "(no text found)".to_string() } else { out.join("\n") };
    Ok(x.cap(&format!("{head}\n{body}"), "read", Some(SNAP_LINES), OUT_CHARS))
}

pub fn shot(x: &mut Desk, c: &Call) -> Res<String> {
    let (hwnd, area) = x.region(c.pos.first().map(String::as_str))?;
    let img = DynamicImage::ImageRgb8(grab_area(area, hwnd)?);
    let key = if hwnd == 0 { "app-screen".to_string() } else { format!("app-{hwnd}") };
    Ok(save_picture(x.d, x.ctx, c, &img, &key, "lighting shot w3 --out C:/shots/app.png")?)
}

pub fn wait(x: &mut Desk, c: &Call, aim: Option<&str>) -> Result<Out, Fail> {
    let words: Vec<&String> = c.pos.iter().filter(|p| !windowish(p)).collect();
    if words.is_empty() {
        return Err(Fail::hint("waiting in an app needs \"text\"", "lighting wait \"Done\" w3 --timeout 60s"));
    }
    let shown = words.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" ");
    let want = shown.to_lowercase();
    let gone = c.on("gone");
    let spec = aim.map(String::from).or_else(|| c.pos.iter().find(|p| windowish(p)).cloned());
    let hwnd = x.target(spec.as_deref()).map_err(|h| match h {
        Halt::Fail(f) => f,
        Halt::Busy => Fail::new("busy"),
    })?;
    let limit = parse_ms(c.flags.get("timeout"), WAIT_MS)? as f64 / 1000.0;
    let lang = lang(x, c);
    let t0 = Instant::now();
    let mut last_ocr: Option<Instant> = None;
    let mut seen = move |d: &super::super::Daemon, ctx: &mut super::super::ctx::Ctx| -> Result<Option<bool>, Fail> {
        if !win::alive(hwnd) {
            let wref = state::with(ctx, |st, ctx| Desk { d, st, ctx }.wref(hwnd));
            return Err(Fail::hint(format!("window {wref} is gone"), "lighting windows"));
        }
        if app_text(hwnd).to_lowercase().contains(&want) {
            return Ok(Some(true));
        }
        let due = last_ocr.is_none_or(|t| t.elapsed().as_secs_f64() >= OCR_EVERY_S);
        if !(gone || due) || win::minimized(hwnd) {
            return Ok(None);
        }
        last_ocr = Some(Instant::now());
        let Ok(img) = grab_area(win::rect(hwnd), hwnd) else { return Ok(None) };
        let text: Vec<String> = ocr::recognize(&img, &lang)?.into_iter().map(|l| l.0).collect();
        Ok(Some(text.join(" ").to_lowercase().contains(&want)))
    };
    Ok(Out::Wait(Pending::new(Duration::from_secs_f64(limit), POLL, move |d, ctx, end| {
        let hit = seen(d, ctx)?;
        let done = if gone { hit == Some(false) } else { hit == Some(true) };
        if done {
            return Ok(Some(format!("ok ({:.1} s)", t0.elapsed().as_secs_f64())));
        }
        if end {
            let wref = state::with(ctx, |st, ctx| Desk { d, st, ctx }.wref(hwnd));
            let how = if gone { "still there" } else { "not there" };
            return Err(Fail::hint(
                format!("\"{shown}\" {how} after {} s in {wref}", limit as i64),
                format!("lighting read {wref}"),
            ));
        }
        Ok(None)
    })))
}
