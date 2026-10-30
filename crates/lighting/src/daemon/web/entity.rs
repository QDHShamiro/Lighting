const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{a0}"),
    ("shy", "\u{ad}"),
    ("ensp", "\u{2002}"),
    ("emsp", "\u{2003}"),
    ("thinsp", "\u{2009}"),
    ("zwnj", "\u{200c}"),
    ("zwj", "\u{200d}"),
    ("lrm", "\u{200e}"),
    ("rlm", "\u{200f}"),
    ("ndash", "\u{2013}"),
    ("mdash", "\u{2014}"),
    ("lsquo", "\u{2018}"),
    ("rsquo", "\u{2019}"),
    ("sbquo", "\u{201a}"),
    ("ldquo", "\u{201c}"),
    ("rdquo", "\u{201d}"),
    ("bdquo", "\u{201e}"),
    ("laquo", "\u{ab}"),
    ("raquo", "\u{bb}"),
    ("lsaquo", "\u{2039}"),
    ("rsaquo", "\u{203a}"),
    ("hellip", "\u{2026}"),
    ("bull", "\u{2022}"),
    ("middot", "\u{b7}"),
    ("copy", "\u{a9}"),
    ("reg", "\u{ae}"),
    ("trade", "\u{2122}"),
    ("euro", "\u{20ac}"),
    ("pound", "\u{a3}"),
    ("yen", "\u{a5}"),
    ("cent", "\u{a2}"),
    ("sect", "\u{a7}"),
    ("para", "\u{b6}"),
    ("deg", "\u{b0}"),
    ("plusmn", "\u{b1}"),
    ("times", "\u{d7}"),
    ("divide", "\u{f7}"),
    ("frac12", "\u{bd}"),
    ("frac14", "\u{bc}"),
    ("frac34", "\u{be}"),
    ("sup2", "\u{b2}"),
    ("sup3", "\u{b3}"),
    ("micro", "\u{b5}"),
    ("larr", "\u{2190}"),
    ("uarr", "\u{2191}"),
    ("rarr", "\u{2192}"),
    ("darr", "\u{2193}"),
    ("harr", "\u{2194}"),
    ("auml", "\u{e4}"),
    ("ouml", "\u{f6}"),
    ("uuml", "\u{fc}"),
    ("Auml", "\u{c4}"),
    ("Ouml", "\u{d6}"),
    ("Uuml", "\u{dc}"),
    ("szlig", "\u{df}"),
    ("aacute", "\u{e1}"),
    ("agrave", "\u{e0}"),
    ("acirc", "\u{e2}"),
    ("eacute", "\u{e9}"),
    ("egrave", "\u{e8}"),
    ("ecirc", "\u{ea}"),
    ("iacute", "\u{ed}"),
    ("oacute", "\u{f3}"),
    ("ocirc", "\u{f4}"),
    ("uacute", "\u{fa}"),
    ("ccedil", "\u{e7}"),
    ("ntilde", "\u{f1}"),
    ("Eacute", "\u{c9}"),
];
const LEGACY: &[&str] = &["amp", "lt", "gt", "quot", "nbsp", "copy", "reg"];

pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let (piece, used) = entity(rest);
        out.push_str(piece.as_deref().unwrap_or("&"));
        rest = &rest[if piece.is_some() { used } else { 1 }..];
    }
    out.push_str(rest);
    out
}

fn entity(at: &str) -> (Option<String>, usize) {
    let body = &at[1..];
    if let Some(num) = body.strip_prefix('#') {
        let (digits, radix) = match num.strip_prefix(['x', 'X']) {
            Some(hex) => (hex.bytes().take_while(u8::is_ascii_hexdigit).count(), 16),
            None => (num.bytes().take_while(u8::is_ascii_digit).count(), 10),
        };
        let skip = if radix == 16 { 1 } else { 0 };
        if digits == 0 {
            return (None, 0);
        }
        let raw = &num[skip..skip + digits];
        let ch = u32::from_str_radix(raw, radix).ok().and_then(char::from_u32).filter(|c| *c != '\0');
        let semi = usize::from(num[skip + digits..].starts_with(';'));
        return (Some(ch.unwrap_or('\u{fffd}').to_string()), 2 + skip + digits + semi);
    }
    let name_len = body.bytes().take_while(u8::is_ascii_alphanumeric).count();
    let name = &body[..name_len];
    if body[name_len..].starts_with(';')
        && let Some((_, value)) = ENTITIES.iter().find(|(n, _)| *n == name)
    {
        return (Some((*value).to_string()), name_len + 2);
    }
    match LEGACY.iter().find(|n| name.starts_with(**n)) {
        Some(n) => {
            let value = ENTITIES.iter().find(|(e, _)| e == n).map_or("", |(_, v)| *v);
            (Some(value.to_string()), n.len() + 1)
        }
        None => (None, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_decode_like_python_html_unescape() {
        assert_eq!(unescape("a &lt;b&gt; &amp &#x41;&#66; &bogus; &"), "a <b> & AB &bogus; &");
        assert_eq!(unescape("&copy2026 &#0; &#xD800; &auml;"), "\u{a9}2026 \u{fffd} \u{fffd} \u{e4}");
    }
}
