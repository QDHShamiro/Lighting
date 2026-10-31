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
pub struct Incoming {
    pub token: String,
    pub cwd: String,
    pub sid: String,
    pub secret: Option<String>,
    pub argv: Vec<String>,
}

impl Incoming {
    pub fn decode(raw: &[u8]) -> Option<Incoming> {
        let second = raw.starts_with(b"L2\0");
        if !second && !raw.starts_with(b"L1\0") {
            return None;
        }
        let mut parts: Vec<&str> = std::str::from_utf8(&raw[3..]).ok()?.split('\0').collect();
        if parts.len() < if second { 4 } else { 3 } {
            return None;
        }
        let sid = if second { parts.remove(2) } else { "" };
        Some(Incoming {
            token: parts[0].into(),
            cwd: parts[1].into(),
            sid: sid.into(),
            secret: parts[2].strip_prefix('+').map(String::from),
            argv: parts[3..].iter().map(|s| (*s).to_string()).collect(),
        })
    }
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

    pub fn encode(&self) -> Vec<u8> {
        format!("{}\0{}\0{}", self.code, self.image, self.out).into_bytes()
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

    #[test]
    fn incoming_reads_both_fast_formats_like_the_2x_daemon() {
        let argv: Vec<String> = ["recall", "a b", ""].map(String::from).to_vec();
        let req = Request { token: "ab", cwd: "C:/x", sid: "s1", secret: Some("k"), argv: &argv };
        let got = Incoming::decode(&req.encode()).unwrap();
        assert_eq!((got.token.as_str(), got.cwd.as_str(), got.sid.as_str()), ("ab", "C:/x", "s1"));
        assert_eq!((got.secret.as_deref(), got.argv), (Some("k"), argv));
        let old = Incoming::decode(b"L1\0ab\0C:/y\0-\0ping").unwrap();
        assert_eq!((old.sid.as_str(), old.secret, old.argv), ("", None, vec!["ping".to_string()]));
        assert!(Incoming::decode(b"L2\0ab\0C:/x\0s1").is_none());
        assert!(Incoming::decode(b"L1\0ab\0x").is_none());
        assert!(Incoming::decode(b"{\"argv\":[]}").is_none());
        assert!(Incoming::decode(b"L2\0\xff\0\0\0-").is_none());
    }

    #[test]
    fn reply_round_trips() {
        let reply = Reply { code: 3, image: "C:/a.jpg".into(), out: "x\0y".into() };
        assert_eq!(Reply::decode(&reply.encode()), reply);
    }
}
