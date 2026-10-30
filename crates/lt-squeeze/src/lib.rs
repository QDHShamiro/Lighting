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
    (chars_out < chars_in).then(|| Squeezed {
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
    fn small_outputs_stay_as_they_are() {
        assert!(squeeze("one\ntwo").is_none());
        assert!(squeeze(&"x".repeat(4000)).is_none());
        assert!(squeeze(&"ok\n".repeat(61)).is_some());
    }

    #[test]
    fn reports_sizes_of_both_sides() {
        let text: String = (0..500).map(|i| format!("row {i} fine\n")).collect();
        let s = squeeze(&text).expect("squeezed");
        assert_eq!((s.lines_in, s.chars_in, s.json), (500, text.chars().count() - 1, false));
        assert_eq!((s.lines_out, s.chars_out), (s.body.lines().count(), s.body.chars().count()));
        assert!(s.chars_out < s.chars_in);
    }
}
