use std::collections::HashSet;
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, Rgb, RgbImage};
use lt_core::{Call, Fail, Flag, out_file};
use serde_json::{Map, Value, json};

use super::super::ctx::Ctx;
use super::super::media::tools::{clock, seconds};
use super::super::{Daemon, config};
use super::url::local_file;
use super::{REF_RE, call, head, num, number, obj, text_of};

const SHOT_KINDS: &[&str] = &["png", "jpg", "jpeg", "webp"];
const SMALL_WIDTH: u32 = 768;
const JPEG_QUALITY: u8 = 70;
const PRINT: (u32, u32) = (48, 27);
const THUMB: (u32, u32) = (32, 18);
const STEP: u8 = 24;
const TOKEN_PX: u32 = 28;
const TILE_W: u32 = 400;
const CAPTION_CHARS: usize = 140;
const GLYPH_SCALE: u32 = 2;
const GLYPH_ADVANCE: u32 = 6 * GLYPH_SCALE;
const GLYPHS: &[(char, [u8; 7])] = &[
    ('0', [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E]),
    ('1', [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E]),
    ('2', [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F]),
    ('3', [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E]),
    ('4', [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02]),
    ('5', [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E]),
    ('6', [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E]),
    ('7', [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
    ('8', [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E]),
    ('9', [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C]),
    (':', [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00]),
    ('.', [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C]),
    ('-', [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00]),
    ('?', [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04]),
];

pub fn shot(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let out = out_path(ctx, c, SHOT_KINDS, "wisply shot --out C:/shots/page.png")?;
    let reference = c.pos.first().filter(|p| REF_RE.is_match(p)).cloned();
    let msg = call(d, ctx, "shot", obj(json!({"ref": reference, "marks": c.on("marks")})), 30.0)?;
    let img = crop(decode(text_of(&msg, "image"))?, &msg);
    let width = width(d, c, None)?;
    let key = format!("web-{}", reference.as_deref().unwrap_or("view"));
    let mut res = save_image(d, ctx, &img, &key, c.on("if-changed"), width, threshold(c)?, out)?;
    if c.on("marks") {
        res.push_str(&format!("\n{} orange labels = e-refs (label 12 -> wisply click e12)", num(msg.get("marks"))));
    }
    Ok(res)
}

pub fn save_picture(
    d: &Daemon,
    ctx: &mut Ctx,
    c: &Call,
    img: &DynamicImage,
    key: &str,
    hint: &str,
) -> Result<String, Fail> {
    let out = out_path(ctx, c, SHOT_KINDS, hint)?;
    save_image(d, ctx, img, key, c.on("if-changed"), width(d, c, None)?, threshold(c)?, out)
}

pub fn thumb(img: &DynamicImage) -> Vec<u8> {
    luma(img, THUMB, None)
}

pub fn save_sheet(d: &Daemon, ctx: &mut Ctx, img: &DynamicImage, key: &str) -> Result<String, Fail> {
    save_image(d, ctx, img, key, false, width(d, &Call::default(), None)?, 0.0, None)
}

pub fn image_note(d: &Daemon, ctx: &mut Ctx, raw: &[u8]) -> Result<String, Fail> {
    let img = image::load_from_memory(raw)
        .map_err(|e| Fail::new(format!("not a readable image ({})", head(&e.to_string(), 80))))?;
    save_image(d, ctx, &img, "image", false, width(d, &Call::default(), None)?, 0.0, None)
}

pub fn out_path(ctx: &Ctx, c: &Call, kinds: &[&str], hint: &str) -> Result<Option<PathBuf>, Fail> {
    let raw = match c.flags.get("out") {
        None => return Ok(None),
        Some(Flag::On) => "",
        Some(Flag::Val(v)) => v.as_str(),
    };
    let path = local_file(ctx, raw);
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if raw.is_empty() || !kinds.contains(&ext.as_str()) {
        let list: Vec<String> = kinds.iter().map(|k| format!(".{k}")).collect();
        return Err(Fail::hint(format!("--out needs a file name ending in {}", list.join(", ")), hint));
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    Ok(Some(path))
}

fn decode(data: &str) -> Result<DynamicImage, Fail> {
    let data = if data.starts_with("data:") { data.split_once(',').map_or(data, |(_, b)| b) } else { data };
    let raw = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|_| Fail::new("the browser sent no readable picture"))?;
    image::load_from_memory(&raw).map_err(|e| Fail::new(format!("the browser sent no readable picture ({e})")))
}

fn crop(img: DynamicImage, msg: &Map<String, Value>) -> DynamicImage {
    let vw = msg.get("vw").and_then(Value::as_f64).unwrap_or(0.0);
    let Some(Value::Object(area)) = msg.get("crop") else { return img };
    if vw <= 0.0 {
        return img;
    }
    let s = f64::from(img.width()) / vw;
    let at = |k: &str| area.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let (w, h) = (f64::from(img.width()), f64::from(img.height()));
    let x0 = (at("x") * s).max(0.0) as u32;
    let y0 = (at("y") * s).max(0.0) as u32;
    let x1 = ((at("x") + at("w")) * s).min(w) as u32;
    let y1 = ((at("y") + at("h")) * s).min(h) as u32;
    if x1 > x0 && y1 > y0 { img.crop_imm(x0, y0, x1 - x0, y1 - y0) } else { img }
}

fn width(d: &Daemon, c: &Call, small: Option<bool>) -> Result<u32, Fail> {
    if let Some(n) = number(c, "width")?.filter(|n| *n > 0) {
        return Ok(u32::try_from(n).unwrap_or(u32::MAX));
    }
    if small == Some(true) {
        return Ok(SMALL_WIDTH);
    }
    let cfg = config::load(&d.home);
    let n = cfg.get("shot_width").and_then(Value::as_i64).filter(|n| *n > 0).unwrap_or(config::SHOT_WIDTH);
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

fn threshold(c: &Call) -> Result<f64, Fail> {
    let raw = match c.flags.get("threshold") {
        None => return Ok(0.0),
        Some(Flag::On) => return Ok(2.0),
        Some(Flag::Val(v)) => v,
    };
    raw.trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .map(|v| v.clamp(0.0, 100.0))
        .map_err(|_| Fail::hint("--threshold needs a percent of the picture", "wisply shot --threshold 2"))
}

fn luma(img: &DynamicImage, size: (u32, u32), step: Option<u8>) -> Vec<u8> {
    let small = imageops::resize(&img.to_luma8(), size.0, size.1, FilterType::Triangle);
    let raw = small.into_raw();
    match step {
        Some(step) => raw.into_iter().map(|v| v / step * step).collect(),
        None => raw,
    }
}

#[allow(clippy::too_many_arguments)]
fn save_image(
    d: &Daemon,
    ctx: &mut Ctx,
    img: &DynamicImage,
    key: &str,
    if_changed: bool,
    width: u32,
    threshold: f64,
    out: Option<PathBuf>,
) -> Result<String, Fail> {
    let img = if img.width() > width {
        let h = (f64::from(img.height()) * f64::from(width) / f64::from(img.width())).round().max(1.0) as u32;
        img.resize_exact(width, h, FilterType::Lanczos3)
    } else {
        img.clone()
    };
    let print = luma(&img, PRINT, Some(STEP));
    if (if_changed || threshold > 0.0)
        && let Some(old) = ctx.shots.get(key).filter(|old| old.len() == print.len())
    {
        let changed = old.iter().zip(&print).filter(|(a, b)| a != b).count();
        let moved = changed as f64 * 100.0 / print.len() as f64;
        if moved <= threshold {
            if changed == 0 {
                return Ok(format!("unchanged since the last shot of {key} (no new image)"));
            }
            return Ok(format!(
                "unchanged since the last shot of {key} ({moved:.1}% moved, under --threshold {threshold}; no new image)"
            ));
        }
    }
    ctx.shots.insert(key.to_string(), print);
    let path = out.unwrap_or_else(|| out_file(&d.home.join("out"), &format!("shot-{key}"), "jpg"));
    write_image(&img, &path).map_err(|e| Fail::new(format!("cannot save {} ({e})", path.display())))?;
    ctx.image = path.display().to_string();
    let (w, h) = (img.width(), img.height());
    let cost = w.div_ceil(TOKEN_PX) * h.div_ceil(TOKEN_PX);
    let shown = path.to_string_lossy().replace('\\', "/");
    Ok(format!("shot {shown} ({w}x{h}, ~{cost} image tokens; open it with the Read tool)"))
}

fn write_image(img: &DynamicImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let rgb = img.to_rgb8();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "jpg" || ext == "jpeg" {
        let file = fs::File::create(path).map_err(|e| e.to_string())?;
        let mut writer = BufWriter::new(file);
        return JpegEncoder::new_with_quality(&mut writer, JPEG_QUALITY).encode_image(&rgb).map_err(|e| e.to_string());
    }
    rgb.save(path).map_err(|e| e.to_string())
}

pub fn distinct(thumbs: &[Vec<u8>], n: usize) -> Vec<usize> {
    let gap =
        |a: usize, b: usize| thumbs[a].iter().zip(&thumbs[b]).map(|(x, y)| u32::from(x.abs_diff(*y))).sum::<u32>();
    let mut chosen = vec![0];
    while chosen.len() < n.min(thumbs.len()) {
        let mut best: Option<(u32, usize)> = None;
        for i in (0..thumbs.len()).filter(|i| !chosen.contains(i)) {
            let near = chosen.iter().map(|&j| gap(i, j)).min().unwrap_or(0);
            if best.is_none_or(|(b, _)| near > b) {
                best = Some((near, i));
            }
        }
        match best {
            Some((_, i)) => chosen.push(i),
            None => break,
        }
    }
    chosen.sort_unstable();
    chosen
}

fn only<T>(all: Vec<T>, keep: &[usize]) -> Vec<T> {
    all.into_iter().enumerate().filter(|(i, _)| keep.contains(i)).map(|(_, v)| v).collect()
}

fn fill(sheet: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, color: Rgb<u8>) {
    for y in y0..y1.min(sheet.height()) {
        for x in x0..x1.min(sheet.width()) {
            sheet.put_pixel(x, y, color);
        }
    }
}

fn stamp(sheet: &mut RgbImage, x: u32, y: u32, text: &str) {
    for (k, ch) in text.chars().enumerate() {
        let Some((_, rows)) = GLYPHS.iter().find(|(g, _)| *g == ch) else { continue };
        let left = x + k as u32 * GLYPH_ADVANCE;
        for (r, bits) in rows.iter().enumerate() {
            for b in 0..5u32 {
                if bits & (0x10 >> b) != 0 {
                    let (px, py) = (left + b * GLYPH_SCALE, y + r as u32 * GLYPH_SCALE);
                    fill(sheet, px, py, px + GLYPH_SCALE, py + GLYPH_SCALE, Rgb([255, 255, 255]));
                }
            }
        }
    }
}

pub fn sheet(tiles: &[DynamicImage], stamps: &[String]) -> RgbImage {
    let cols = if tiles.len() > 4 { 3 } else { 2 };
    let first = &tiles[0];
    let th = (f64::from(first.height()) * f64::from(TILE_W) / f64::from(first.width().max(1))).round().max(1.0) as u32;
    let rows = (tiles.len() as u32).div_ceil(cols);
    let mut sheet = RgbImage::new(cols * TILE_W, rows * th);
    for (i, (tile, label)) in tiles.iter().zip(stamps).enumerate() {
        let (x, y) = (i as u32 % cols * TILE_W, i as u32 / cols * th);
        let small = imageops::resize(&tile.to_rgb8(), TILE_W, th, FilterType::CatmullRom);
        imageops::replace(&mut sheet, &small, i64::from(x), i64::from(y));
        let w = (8 + GLYPH_ADVANCE * label.chars().count() as u32).min(TILE_W);
        fill(&mut sheet, x, y, x + w, y + (7 * GLYPH_SCALE + 4).min(th), Rgb([0, 0, 0]));
        stamp(&mut sheet, x + 4, y + 2, label);
    }
    sheet
}

pub fn frames(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let Some(reference) = c.pos.first().filter(|p| REF_RE.is_match(p)).cloned() else {
        return Err(Fail::hint("frames needs the ref of a video", "wisply snap -f video, then wisply frames e40"));
    };
    let count = number(c, "count")?.unwrap_or(6).clamp(2, 12);
    let every = number(c, "every")?.unwrap_or(1500).clamp(200, 10_000);
    let (scenes, live) = (c.on("scenes"), c.on("live"));
    let range = seconds(c.val("from")).and_then(|lo| seconds(c.val("to")).map(|hi| (lo, hi)));
    let (lo, hi) = range
        .map_err(|()| Fail::hint("--from and --to take seconds or m:ss", "wisply frames e40 --from 1:30 --to 2:00"))?;
    let samples = if scenes { count * 4 } else { count };
    let timeout = 30 + if live { count * every / 1000 } else { samples * 8 };
    let args = obj(json!({"ref": reference, "count": count, "every": every, "live": live, "from": lo, "to": hi,
        "samples": samples}));
    let msg = call(d, ctx, "frames", args, timeout as f64)?;
    let mut frames: Vec<Map<String, Value>> = match msg.get("frames") {
        Some(Value::Array(list)) => list.iter().filter_map(|f| f.as_object().cloned()).collect(),
        _ => Vec::new(),
    };
    if frames.is_empty() {
        return Err(Fail::hint("no frames captured", format!("wisply shot {reference}")));
    }
    let mut tiles = frames.iter().map(|f| decode(text_of(f, "image"))).collect::<Result<Vec<_>, _>>()?;
    let moving = msg.get("live").and_then(Value::as_bool).unwrap_or(false);
    let mut thumbs: Vec<Vec<u8>> = tiles.iter().map(|t| luma(t, THUMB, None)).collect();
    if scenes && !moving {
        let keep = distinct(&thumbs, usize::try_from(count).unwrap_or(6));
        (tiles, frames, thumbs) = (only(tiles, &keep), only(frames, &keep), only(thumbs, &keep));
    }
    let stamps: Vec<String> = frames
        .iter()
        .map(|f| text_of(f, "media").split('/').next().unwrap_or("").split(' ').next().unwrap_or("").to_string())
        .map(|s| if s.is_empty() { "?".into() } else { s })
        .collect();
    let width = width(d, c, Some(c.on("small")))?;
    let res = save_image(
        d,
        ctx,
        &DynamicImage::ImageRgb8(sheet(&tiles, &stamps)),
        &format!("frames-{reference}"),
        false,
        width,
        0.0,
        None,
    )?;
    let looks: HashSet<Vec<u8>> = thumbs.iter().map(|t| t.iter().map(|v| v / STEP * STEP).collect()).collect();
    let same_stamp = stamps.iter().collect::<HashSet<_>>().len() == 1 && stamps[0].contains(':');
    let note = match (looks.len() == 1 || same_stamp, moving) {
        (false, _) => String::new(),
        (true, true) => format!(
            " | the video did not move (paused or not playing) -> try: click {reference} or press k, then frames again"
        ),
        (true, false) => {
            format!(" | all frames look the same (black or not decoding) -> try: wisply frames {reference} --live")
        }
    };
    let mode = if moving {
        format!("live, every {:.1} s{}", every as f64 / 1000.0, if live { "" } else { ", video not seekable" })
    } else {
        let duration = msg.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
        let span = format!("{}-{} of {}", clock(lo.unwrap_or(0.0)), clock(hi.unwrap_or(duration)), clock(duration));
        let lead = if scenes {
            format!("{} distinct scenes from {samples} samples, ", tiles.len())
        } else {
            "spread over ".into()
        };
        format!("{lead}{span}")
    };
    let mut lines = vec![
        res.replacen("shot ", "frames ", 1),
        format!("{} frames ({mode}) at {}{note}", tiles.len(), stamps.join(" ")),
    ];
    let caps: Vec<String> = stamps
        .iter()
        .zip(&frames)
        .filter(|(_, f)| !text_of(f, "cap").is_empty())
        .map(|(s, f)| format!("{s} {}", head(text_of(f, "cap"), CAPTION_CHARS)))
        .collect();
    if !caps.is_empty() {
        lines.push("captions:".into());
        lines.extend(caps);
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_scenes_read_like_2x() {
        assert_eq!(seconds(Some("1:02:03")), Ok(Some(3723.0)));
        assert_eq!(seconds(None), Ok(None));
        assert_eq!(clock(83.9), "1:23");
        let thumbs = vec![vec![0u8; 4], vec![1; 4], vec![200; 4], vec![90; 4], vec![201; 4]];
        assert_eq!(distinct(&thumbs, 3), vec![0, 3, 4]);
        assert_eq!(distinct(&thumbs, 9), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn contact_sheets_have_stamped_tiles() {
        let tile = DynamicImage::ImageRgb8(RgbImage::from_pixel(800, 450, Rgb([90, 90, 90])));
        let tiles = vec![tile.clone(), tile.clone(), tile];
        let stamps = vec!["0:01".to_string(), "?".into(), "1:23".into()];
        let sheet = sheet(&tiles, &stamps);
        assert_eq!(sheet.dimensions(), (800, 450));
        assert_eq!(sheet.get_pixel(1, 1), &Rgb([0, 0, 0]));
        assert_eq!(sheet.get_pixel(200, 200), &Rgb([90, 90, 90]));
        let white =
            (0..30u32).flat_map(|x| (0..18u32).map(move |y| (x, y))).any(|(x, y)| sheet.get_pixel(x, y)[0] == 255);
        assert!(white);
    }
}
