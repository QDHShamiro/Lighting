pub fn close_match<'a>(word: &str, options: &[&'a str]) -> Option<&'a str> {
    let b: Vec<char> = word.chars().collect();
    let mut best: Option<(usize, usize, &str)> = None;
    for &x in options {
        let a: Vec<char> = x.chars().collect();
        let (m, t) = (matches(&a, &b), a.len() + b.len());
        if 8 * m < 3 * t {
            continue;
        }
        let better = best.is_none_or(|(bm, bt, bx)| m * bt > bm * t || (m * bt == bm * t && x > bx));
        if better {
            best = Some((m, t, x));
        }
    }
    best.map(|(_, _, x)| x)
}

fn matches(a: &[char], b: &[char]) -> usize {
    let (i, j, k) = longest(a, b);
    if k == 0 {
        return 0;
    }
    k + matches(&a[..i], &b[..j]) + matches(&a[i + k..], &b[j + k..])
}

fn longest(a: &[char], b: &[char]) -> (usize, usize, usize) {
    let mut best = (0, 0, 0);
    for i in 0..a.len() {
        for j in 0..b.len() {
            let k = a[i..].iter().zip(&b[j..]).take_while(|(x, y)| x == y).count();
            if k > best.2 {
                best = (i, j, k);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_like_difflib_get_close_matches() {
        let names = ["snap", "shot", "status", "stop", "tab", "tabs", "table"];
        assert_eq!(close_match("snapp", &names), Some("snap"));
        assert_eq!(close_match("stauts", &names), Some("status"));
        assert_eq!(close_match("tabe", &names), Some("table"));
        assert_eq!(close_match("zz", &names), None);
        assert_eq!(close_match("", &names), None);
        assert_eq!(close_match("x", &[]), None);
    }
}
