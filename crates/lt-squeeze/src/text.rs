pub(crate) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub(crate) fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

pub(crate) fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

pub(crate) fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

pub(crate) fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(is_space).filter(|w| !w.is_empty())
}

pub(crate) fn len(s: &str) -> usize {
    s.chars().count()
}

pub(crate) fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

pub(crate) fn clip(s: &str, max: usize) -> String {
    let n = len(s);
    if n <= max { s.to_string() } else { format!("{}... (+{} chars)", head(s, max), n - max) }
}

pub fn commas(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_is_the_same_as_python_str_split() {
        assert_eq!(words(" a\tb\u{1f}c\u{a0}d\u{200b}e ").collect::<Vec<_>>(), ["a", "b", "c", "d\u{200b}e"]);
        assert_eq!(strip("\u{3000} x \n"), "x");
        assert_eq!(rstrip("x \r"), "x");
    }

    #[test]
    fn counts_and_cuts_by_characters() {
        assert_eq!(len("Grüße 😀"), 7);
        assert_eq!(head("Grüße", 3), "Grü");
        assert_eq!(head("ab", 5), "ab");
        assert_eq!(clip("äöüß", 2), "äö... (+2 chars)");
        assert_eq!(clip("äö", 2), "äö");
    }

    #[test]
    fn thousands_get_commas() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(1000), "1,000");
        assert_eq!(commas(1234567), "1,234,567");
    }
}
