pub fn terms(f: &str) -> Vec<String> {
    f.to_lowercase().split('|').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect()
}

pub fn hit(words: &[String], texts: &[&str]) -> bool {
    texts.iter().map(|x| x.to_lowercase()).any(|x| words.iter().any(|w| x.contains(w.as_str())))
}

pub fn tiers(words: &[String], texts: &[&str]) -> Vec<u8> {
    let lowered: Vec<String> = texts.iter().map(|x| x.to_lowercase()).collect();
    words
        .iter()
        .map(|w| {
            let mut best = 0;
            for x in &lowered {
                match tier(w, x) {
                    3 => return 3,
                    t => best = best.max(t),
                }
            }
            best
        })
        .collect()
}

fn tier(word: &str, text: &str) -> u8 {
    let mut best = 0;
    for (i, _) in text.char_indices() {
        if word.is_empty() || !text[i..].starts_with(word) {
            continue;
        }
        if text[..i].chars().next_back().is_some_and(char::is_alphanumeric) {
            best = best.max(1);
            continue;
        }
        if text[i + word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric()) {
            return 3;
        }
        best = 2;
    }
    best
}

pub fn best_only<'a>(rows: &[(Vec<u8>, &'a str)]) -> Vec<&'a str> {
    let width = rows.iter().map(|(t, _)| t.len()).max().unwrap_or(0);
    let top: Vec<u8> =
        (0..width).map(|i| rows.iter().filter_map(|(t, _)| t.get(i)).copied().max().unwrap_or(0)).collect();
    rows.iter()
        .filter(|(t, _)| t.is_empty() || t.iter().zip(&top).any(|(x, best)| *x > 0 && x == best))
        .map(|(_, item)| *item)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_rank_like_2x() {
        let w = terms(" Save | |DRAFT");
        assert_eq!(w, ["save", "draft"]);
        assert!(hit(&w, &["x", "Saved"]) && !hit(&w, &["nothing"]));
        assert_eq!(tiers(&w, &["save draft"]), [3, 3]);
        assert_eq!(tiers(&w, &["saved", "x_drafts"]), [2, 2]);
        assert_eq!(tiers(&w, &["unsaved", "redraft"]), [1, 1]);
        assert_eq!(tiers(&w, &["none"]), [0, 0]);
        let rows = [(vec![3, 0], "a"), (vec![2, 0], "b"), (vec![0, 1], "c")];
        assert_eq!(best_only(&rows), ["a", "c"]);
    }
}
