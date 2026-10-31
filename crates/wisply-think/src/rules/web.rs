use std::sync::LazyLock;

use regex::Regex;

use super::{Finding, line_of};
use crate::files::Pack;

const FONT_MIN_PX: f64 = 12.0;
const CONTRAST_MIN: f64 = 4.5;
const IMPORTANT_MAX: usize = 10;

fn re(p: &str) -> Regex {
    Regex::new(p).expect("web rule pattern")
}

static IMG: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<img\b[^>]*>"));
static HTML: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<html\b[^>]*>"));
static BLANK: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)<a\b[^>]*target\s*=\s*["']?_blank[^>]*>"#));
static FONT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)font-size\s*:\s*(\d+(?:\.\d+)?)px"));
static RULE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)([^{}]+)\{([^{}]*)\}"));
static FG: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:^|[;\s])color\s*:\s*(#[0-9a-f]{3,8})\b"));
static BG: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)background(?:-color)?\s*:\s*(#[0-9a-f]{3,8})\b"));

fn hex(c: &str) -> Option<[f64; 3]> {
    let h = c.trim_start_matches('#');
    let full: String = match h.len() {
        3 | 4 => h.chars().take(3).flat_map(|c| [c, c]).collect(),
        6 | 8 => h[..6].to_string(),
        _ => return None,
    };
    let v = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok().map(|b| f64::from(b) / 255.0);
    Some([v(0)?, v(2)?, v(4)?])
}

fn luminance(c: [f64; 3]) -> f64 {
    let lin = |v: f64| if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

pub fn contrast(a: &str, b: &str) -> Option<f64> {
    let (x, y) = (luminance(hex(a)?), luminance(hex(b)?));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    Some((hi + 0.05) / (lo + 0.05))
}

fn html(text: &str, out: &mut Vec<Finding>) {
    let missing: Vec<usize> = IMG
        .find_iter(text)
        .filter(|m| !m.as_str().to_lowercase().contains("alt="))
        .map(|m| line_of(text, m.start()))
        .collect();
    if let Some(first) = missing.first() {
        out.push(
            Finding::new(
                "img-alt",
                Pack::Web,
                *first,
                format!("{} images without alt text", missing.len()),
                "describe each image in alt=\"...\"",
            )
            .rate(2, 3, 1),
        );
    }
    if let Some(m) = HTML.find(text)
        && !m.as_str().to_lowercase().contains("lang=")
    {
        out.push(
            Finding::new(
                "html-lang",
                Pack::Web,
                line_of(text, m.start()),
                "<html> has no lang",
                "add lang=\"en\" (or the page language)",
            )
            .rate(1, 3, 1),
        );
    }
    let lower = text.to_lowercase();
    if lower.contains("<head") && !lower.contains("name=\"viewport\"") && !lower.contains("name=viewport") {
        out.push(
            Finding::new(
                "viewport",
                Pack::Web,
                0,
                "no viewport meta, phones show it zoomed out",
                "add <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
            )
            .rate(2, 3, 1),
        );
    }
    for m in BLANK.find_iter(text) {
        if !m.as_str().to_lowercase().contains("noopener") && !m.as_str().to_lowercase().contains("noreferrer") {
            out.push(
                Finding::new(
                    "blank-rel",
                    Pack::Web,
                    line_of(text, m.start()),
                    "target=_blank without rel",
                    "add rel=\"noopener\"",
                )
                .rate(1, 3, 1),
            );
            break;
        }
    }
}

fn css(text: &str, out: &mut Vec<Finding>) {
    for c in FONT.captures_iter(text) {
        let px: f64 = c[1].parse().unwrap_or(FONT_MIN_PX);
        if px < FONT_MIN_PX {
            let at = line_of(text, c.get(0).map_or(0, |m| m.start()));
            out.push(
                Finding::new(
                    "tiny-font",
                    Pack::Web,
                    at,
                    format!("font-size {px}px is hard to read"),
                    "use 12px or more",
                )
                .rate(2, 2, 1),
            );
            break;
        }
    }
    for c in RULE.captures_iter(text) {
        let body = &c[2];
        let (Some(fg), Some(bg)) = (FG.captures(body), BG.captures(body)) else { continue };
        let Some(ratio) = contrast(&fg[1], &bg[1]) else { continue };
        if ratio < CONTRAST_MIN {
            let at = line_of(text, c.get(0).map_or(0, |m| m.start()));
            out.push(
                Finding::new(
                    "contrast",
                    Pack::Web,
                    at,
                    format!("{} on {} is {ratio:.1}:1 in {}", &fg[1], &bg[1], c[1].trim()),
                    format!("make it at least {CONTRAST_MIN}:1"),
                )
                .rate(2, 3, 1),
            );
        }
    }
    let important = text.matches("!important").count();
    if important > IMPORTANT_MAX {
        out.push(
            Finding::new(
                "important",
                Pack::Web,
                0,
                format!("{important} !important"),
                "fix the selector order instead",
            )
            .rate(1, 2, 2),
        );
    }
}

pub fn check(ext: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    match ext {
        "html" | "htm" | "vue" | "svelte" | "jsx" | "tsx" => {
            html(text, &mut out);
            if matches!(ext, "vue" | "svelte" | "html" | "htm") {
                css(text, &mut out);
            }
        }
        "css" | "scss" => css(text, &mut out),
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_and_styles_are_checked() {
        let page =
            "<html><head><title>x</title></head><body><img src=a.png><a href=x target=\"_blank\">x</a></body></html>";
        let got: Vec<String> = check("html", page).iter().map(|f| f.rule.clone()).collect();
        assert_eq!(got, ["img-alt", "html-lang", "viewport", "blank-rel"]);
        let style = ".a { color: #777; background: #888; font-size: 10px }\n.b { color: #000; background-color: #fff }";
        let got: Vec<String> = check("css", style).iter().map(|f| f.rule.clone()).collect();
        assert_eq!(got, ["tiny-font", "contrast"]);
        assert!((contrast("#000", "#fff").unwrap() - 21.0).abs() < 0.01);
        assert!(
            check("html", "<html lang=en><head><meta name=\"viewport\"></head><img alt=\"\" src=a></html>").is_empty()
        );
    }
}
