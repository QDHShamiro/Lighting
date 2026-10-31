pub fn words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                started = true;
            }
            None if matches!(ch, ' ' | '\t' | '\r' | '\n') => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => {
                cur.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("No closing quotation".into());
    }
    if started {
        out.push(cur);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(line: &str) -> Vec<String> {
        words(line).unwrap()
    }

    #[test]
    fn splits_like_python_shlex_posix_without_escapes() {
        assert_eq!(w("snap -f price"), ["snap", "-f", "price"]);
        assert_eq!(w(r#"fill "Email=a@b.c" --submit"#), ["fill", "Email=a@b.c", "--submit"]);
        assert_eq!(w(r#"click "Sign in""#), ["click", "Sign in"]);
        assert_eq!(w(r#"a "" b"#), ["a", "", "b"]);
        assert_eq!(w(r#"x"y z"w 'it''s'"#), ["xy zw", "its"]);
        assert_eq!(w(r"type e3 C:\Users\x"), ["type", "e3", r"C:\Users\x"]);
        assert_eq!(w(r#"say 'he said "hi"'"#), ["say", r#"he said "hi""#]);
        assert_eq!(w(" \t\r\nsnap\n"), ["snap"]);
        assert_eq!(w("type e3 Grüße 😀"), ["type", "e3", "Grüße", "😀"]);
        assert!(w("").is_empty());
        assert_eq!(words(r#"click "Sign in"#), Err("No closing quotation".to_string()));
    }
}
