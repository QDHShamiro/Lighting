mod json;
mod log;
mod pat;
mod text;

pub use text::commas;

const MIN_LINES: usize = 60;
const MIN_CHARS: usize = 4000;

pub struct Squeezed {
    pub plain: String,
    pub body: String,
    pub lines_in: usize,
    pub lines_out: usize,
    pub chars_in: usize,
    pub chars_out: usize,
    pub json: bool,
}

pub fn clean(text: &str) -> String {
    log::clean(text).join("\n")
}

pub fn decode(raw: &[u8]) -> String {
    if raw.starts_with(b"\xff\xfe") || raw.starts_with(b"\xfe\xff") {
        let le = raw.starts_with(b"\xff\xfe");
        let units: Vec<u16> = raw[2..]
            .chunks(2)
            .map(|c| match c {
                [a, b] if le => u16::from_le_bytes([*a, *b]),
                [a, b] => u16::from_be_bytes([*a, *b]),
                _ => 0xFFFD,
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(raw)).into_owned()
}

pub fn read_limited(mut from: impl std::io::Read, max: usize) -> Vec<u8> {
    let keep = max / 2;
    let (mut head, mut data, mut dropped) = (None::<Vec<u8>>, Vec::new(), 0usize);
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let n = match from.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        data.extend_from_slice(&chunk[..n]);
        if head.is_none() && data.len() > keep {
            head = Some(data[..keep].to_vec());
            data.drain(..keep);
        } else if head.is_some() && data.len() > keep {
            let extra = data.len() - keep;
            dropped += extra;
            data.drain(..extra);
        }
    }
    let Some(mut out) = head else { return data };
    if dropped > 0 {
        out.extend(format!("\n... {} MB in the middle not read\n", dropped / 1_000_000).bytes());
    }
    out.extend(data);
    out
}

pub fn squeeze(text: &str) -> Option<Squeezed> {
    let lines = log::clean(text);
    let plain = lines.join("\n");
    let chars_in = text::len(&plain);
    if lines.len() <= MIN_LINES && chars_in <= MIN_CHARS {
        return None;
    }
    let (out, json) = match json::parse(&plain) {
        Some((obj, rows)) => (json::squeeze(&obj, rows), true),
        None => (log::squeeze(&lines), false),
    };
    let body = out.join("\n");
    let chars_out = text::len(&body);
    (chars_out < chars_in).then_some(Squeezed {
        lines_in: lines.len(),
        lines_out: out.len(),
        chars_in,
        chars_out,
        json,
        body,
        plain,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_input_keeps_head_and_tail() {
        let raw: Vec<u8> = (0..30u8).flat_map(|i| [b'a' + i % 26; 10]).collect();
        let got = read_limited(&raw[..], 100);
        assert_eq!(&got[..50], &raw[..50]);
        assert!(got.ends_with(&raw[250..]));
        assert_eq!(read_limited(&raw[..40], 100), &raw[..40]);
        assert_eq!(decode(b"\xef\xbb\xbfhi"), "hi");
        assert_eq!(decode(b"\xff\xfeh\0i\0"), "hi");
    }

    #[test]
    fn small_outputs_stay_as_they_are() {
        assert!(squeeze("one\ntwo").is_none());
        assert!(squeeze(&"x".repeat(4000)).is_none());
        assert!(squeeze(&"ok\n".repeat(61)).is_some());
    }

    #[test]
    fn reports_sizes_of_both_sides() {
        let text = (0..500).map(|i| format!("row {i} fine")).collect::<Vec<_>>().join("\n") + "\n";
        let s = squeeze(&text).expect("squeezed");
        assert_eq!((s.lines_in, s.chars_in, s.json), (500, text.chars().count() - 1, false));
        assert_eq!((s.lines_out, s.chars_out), (s.body.lines().count(), s.body.chars().count()));
        assert!(s.chars_out < s.chars_in);
    }
}
