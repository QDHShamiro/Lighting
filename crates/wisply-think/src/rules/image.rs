use std::fs;
use std::io::Cursor;
use std::path::Path;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ColorType, DynamicImage, ImageEncoder, ImageFormat};

use super::Finding;
use crate::files::Pack;

const RASTER_KB: u64 = 500;
const GIF_KB: u64 = 2048;
const SVG_KB: u64 = 200;
const SIDE_MAX: u32 = 4096;
const PNG_DEEP_MAX: u64 = 8 * 1024 * 1024;
const SAVE_SHARE: f64 = 0.1;
const SAVE_MIN: u64 = 4096;

fn opaque(img: &DynamicImage) -> bool {
    match img {
        DynamicImage::ImageRgba8(b) => b.pixels().all(|p| p.0[3] == 255),
        DynamicImage::ImageLumaA8(b) => b.pixels().all(|p| p.0[1] == 255),
        _ => false,
    }
}

pub fn recompress(raw: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(raw, ImageFormat::Png).ok()?;
    if !matches!(img.color(), ColorType::Rgb8 | ColorType::Rgba8 | ColorType::L8 | ColorType::La8) {
        return None;
    }
    let img = match (&img, opaque(&img)) {
        (DynamicImage::ImageRgba8(_), true) => DynamicImage::ImageRgb8(img.to_rgb8()),
        (DynamicImage::ImageLumaA8(_), true) => DynamicImage::ImageLuma8(img.to_luma8()),
        _ => img,
    };
    let mut out = Vec::new();
    PngEncoder::new_with_quality(Cursor::new(&mut out), CompressionType::Best, FilterType::Adaptive)
        .write_image(img.as_bytes(), img.width(), img.height(), img.color().into())
        .ok()?;
    (out.len() < raw.len()).then_some(out)
}

pub fn png_smaller(path: &Path) -> Option<(u64, u64)> {
    let raw = fs::read(path).ok()?;
    let new = recompress(&raw)?;
    let (old, new) = (raw.len() as u64, new.len() as u64);
    let saved = old - new;
    (saved >= SAVE_MIN && saved as f64 >= old as f64 * SAVE_SHARE).then_some((old, new))
}

fn kb(n: u64) -> String {
    format!("{} KB", n.div_ceil(1024))
}

pub fn check(path: &Path, ext: &str, deep: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    let Ok(meta) = fs::metadata(path) else { return out };
    let size = meta.len();
    if ext == "svg" {
        if size > SVG_KB * 1024 {
            out.push(
                Finding::new(
                    "big-svg",
                    Pack::Images,
                    0,
                    format!("SVG is {}", kb(size)),
                    "simplify the paths or export smaller",
                )
                .rate(1, 2, 2),
            );
        }
        if let Ok(text) = fs::read_to_string(path)
            && text.contains("href=\"data:image/")
        {
            out.push(
                Finding::new(
                    "svg-raster",
                    Pack::Images,
                    0,
                    "SVG carries an embedded raster image",
                    "link the image or redraw it as vectors",
                )
                .rate(1, 2, 2),
            );
        }
        return out;
    }
    let limit = if ext == "gif" { GIF_KB } else { RASTER_KB };
    if size > limit * 1024 {
        out.push(
            Finding::new(
                "big-image",
                Pack::Images,
                0,
                format!("{} (budget {})", kb(size), kb(limit * 1024)),
                "export smaller or as webp",
            )
            .rate(2, 2, 1),
        );
    }
    if let Ok((w, h)) = image::image_dimensions(path)
        && w.max(h) > SIDE_MAX
    {
        out.push(
            Finding::new(
                "huge-image",
                Pack::Images,
                0,
                format!("{w}x{h} px"),
                format!("scale to at most {SIDE_MAX} px"),
            )
            .rate(2, 2, 1),
        );
    }
    if deep
        && ext == "png"
        && size <= PNG_DEEP_MAX
        && let Some((old, new)) = png_smaller(path)
    {
        out.push(
            Finding::new(
                "png-smaller",
                Pack::Images,
                0,
                format!("lossless {} -> {}", kb(old), kb(new)),
                "recompress it",
            )
            .rate(1, 3, 1)
            .fixed_by("png"),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    #[test]
    fn opaque_png_gets_smaller_without_losing_pixels() {
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_fn(200, 200, |x, y| Rgba([(x % 7) as u8 * 30, (y % 5) as u8 * 40, 90, 255]));
        let mut raw = Vec::new();
        PngEncoder::new_with_quality(Cursor::new(&mut raw), CompressionType::Fast, FilterType::NoFilter)
            .write_image(img.as_raw(), 200, 200, ColorType::Rgba8.into())
            .unwrap();
        let small = recompress(&raw).unwrap();
        assert!(small.len() < raw.len());
        let back = image::load_from_memory(&small).unwrap().to_rgba8();
        assert_eq!(back.as_raw(), img.as_raw());
    }
}
