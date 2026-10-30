pub struct Request<'a> {
    pub token: &'a str,
    pub cwd: &'a str,
    pub sid: &'a str,
    pub secret: Option<&'a str>,
    pub argv: &'a [String],
}

impl Request<'_> {
    pub fn encode(&self) -> Vec<u8> {
        let size = 80 + self.cwd.len() + self.argv.iter().map(|a| a.len() + 1).sum::<usize>();
        let mut req = String::with_capacity(size);
        req.push_str("L2\0");
        req.push_str(self.token);
        req.push('\0');
        req.push_str(&clean(self.cwd));
        req.push('\0');
        req.push_str(&clean(self.sid));
        req.push('\0');
        match self.secret {
            Some(s) => {
                req.push('+');
                req.push_str(&clean(s));
            }
            None => req.push('-'),
        }
        for a in self.argv {
            req.push('\0');
            req.push_str(&clean(a));
        }
        req.into_bytes()
    }
}

fn clean(s: &str) -> String {
    s.replace('\0', "")
}

#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub code: i32,
    pub image: String,
    pub out: String,
}

impl Reply {
    pub fn decode(raw: &[u8]) -> Reply {
        let text = String::from_utf8_lossy(raw);
        let mut parts = text.splitn(3, '\0');
        let code = parts.next().and_then(|c| c.parse().ok()).unwrap_or(1);
        let image = parts.next().unwrap_or("").to_string();
        let out = parts.next().unwrap_or("").to_string();
        Reply { code, image, out }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_matches_the_2x_fast_format() {
        let argv: Vec<String> = ["snap", "-f", "a b"].map(String::from).to_vec();
        let req = Request { token: "ab", cwd: "C:/x", sid: "s\0id", secret: None, argv: &argv };
        assert_eq!(req.encode(), b"L2\0ab\0C:/x\0sid\0-\0snap\0-f\0a b".to_vec());
        let req = Request { token: "ab", cwd: "", sid: "", secret: Some("k"), argv: &[] };
        assert_eq!(req.encode(), b"L2\0ab\0\0\0+k".to_vec());
    }

    #[test]
    fn reply_splits_code_image_and_text() {
        assert_eq!(
            Reply::decode(b"0\0C:/out/a.jpg\0line\0with nul"),
            Reply { code: 0, image: "C:/out/a.jpg".into(), out: "line\0with nul".into() }
        );
        assert_eq!(Reply::decode(b"2\0\0err: x"), Reply { code: 2, image: String::new(), out: "err: x".into() });
        assert_eq!(Reply::decode(b"garbage"), Reply { code: 1, image: String::new(), out: String::new() });
    }
}
