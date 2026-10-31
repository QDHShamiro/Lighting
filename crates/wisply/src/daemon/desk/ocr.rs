use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use image::{RgbImage, imageops};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;
use wisply_core::Fail;

const UPSCALE_BELOW: u32 = 1200;
const KEPT: usize = 8;
const NO_LANGUAGE: &str = "no Windows OCR language installed (Settings > Time & language > Language > add English)";

pub type Line = (String, f32, f32, f32, f32);

thread_local! {
    static ENGINES: RefCell<HashMap<String, OcrEngine>> = RefCell::new(HashMap::new());
    static CACHE: RefCell<Vec<(u64, Vec<Line>)>> = const { RefCell::new(Vec::new()) };
}

fn tag(lang: &str) -> &str {
    match lang {
        "en" => "en-US",
        "de" => "de-DE",
        other => other,
    }
}

fn create(lang: &str) -> windows::core::Result<Option<OcrEngine>> {
    if !lang.is_empty() && lang != "auto" {
        let language = Language::CreateLanguage(&HSTRING::from(tag(lang)))?;
        if OcrEngine::IsLanguageSupported(&language)? {
            return OcrEngine::TryCreateFromLanguage(&language).map(Some);
        }
    }
    let mut english = None;
    for l in OcrEngine::AvailableRecognizerLanguages()? {
        if l.LanguageTag()?.to_string().to_lowercase().starts_with("en") {
            english = Some(l);
            break;
        }
    }
    match english {
        Some(l) => OcrEngine::TryCreateFromLanguage(&l).map(Some),
        None => OcrEngine::TryCreateFromUserProfileLanguages().map(Some),
    }
}

fn engine(lang: &str) -> Result<OcrEngine, Fail> {
    ENGINES.with(|cell| {
        let mut map = cell.borrow_mut();
        if let Some(e) = map.get(lang) {
            return Ok(e.clone());
        }
        let e = create(lang).ok().flatten().ok_or_else(|| Fail::new(NO_LANGUAGE))?;
        map.insert(lang.to_string(), e.clone());
        Ok(e)
    })
}

fn digest(img: &RgbImage, lang: &str) -> u64 {
    let mut h = DefaultHasher::new();
    img.as_raw().iter().step_by(97).collect::<Vec<_>>().hash(&mut h);
    (img.width(), img.height(), lang).hash(&mut h);
    h.finish()
}

fn run(img: &RgbImage, lang: &str) -> windows::core::Result<Vec<(String, [f32; 4])>> {
    let rgba: Vec<u8> = img.pixels().flat_map(|p| [p.0[0], p.0[1], p.0[2], 255]).collect();
    let writer = DataWriter::new()?;
    writer.WriteBytes(&rgba)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &writer.DetachBuffer()?,
        BitmapPixelFormat::Rgba8,
        img.width() as i32,
        img.height() as i32,
    )?;
    let Ok(engine) = engine(lang) else { return Ok(Vec::new()) };
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let mut lines = Vec::new();
    for line in result.Lines()? {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for word in line.Words()? {
            let r = word.BoundingRect()?;
            b = [b[0].min(r.X), b[1].min(r.Y), b[2].max(r.X + r.Width), b[3].max(r.Y + r.Height)];
        }
        if b[0] <= b[2] {
            lines.push((line.Text()?.to_string(), b));
        }
    }
    Ok(lines)
}

pub fn recognize(img: &RgbImage, lang: &str) -> Result<Vec<Line>, Fail> {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    engine(lang)?;
    let scale = if img.width().max(img.height()) <= UPSCALE_BELOW { 2 } else { 1 };
    let big;
    let img = if scale == 1 {
        img
    } else {
        big = imageops::resize(img, img.width() * 2, img.height() * 2, imageops::FilterType::Lanczos3);
        &big
    };
    let key = digest(img, lang);
    if let Some(hit) = CACHE.with(|c| c.borrow().iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())) {
        return Ok(hit);
    }
    let found = run(img, lang).map_err(|e| Fail::new(format!("Windows OCR failed ({})", e.message())))?;
    let s = scale as f32;
    let lines: Vec<Line> = found
        .into_iter()
        .map(|(text, [x0, y0, x1, y1])| (text, x0 / s, y0 / s, (x1 - x0) / s, (y1 - y0) / s))
        .collect();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.push((key, lines.clone()));
        if c.len() > KEPT {
            c.remove(0);
        }
    });
    Ok(lines)
}
