use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::re;

const SEED: &[(&str, &str)] = &[
    ("youtube.com", "https://www.youtube.com/results?search_query={q}"),
    ("github.com", "https://github.com/search?q={q}"),
    ("google.com", "https://www.google.com/search?q={q}"),
    ("duckduckgo.com", "https://duckduckgo.com/?q={q}"),
    ("de.wikipedia.org", "https://de.wikipedia.org/w/index.php?search={q}"),
    ("en.wikipedia.org", "https://en.wikipedia.org/w/index.php?search={q}"),
    ("amazon.de", "https://www.amazon.de/s?k={q}"),
    ("modrinth.com", "https://modrinth.com/plugins?q={q}"),
    ("spigotmc.org", "https://www.spigotmc.org/search/?q={q}"),
    ("tiktok.com", "https://www.tiktok.com/search?q={q}"),
    ("reddit.com", "https://www.reddit.com/search/?q={q}"),
];
const NAMES: &[(&str, &str)] = &[
    ("yt", "youtube.com"),
    ("gh", "github.com"),
    ("ddg", "duckduckgo.com"),
    ("wiki", "de.wikipedia.org"),
    ("wikipedia", "de.wikipedia.org"),
    ("enwiki", "en.wikipedia.org"),
    ("amazon", "amazon.de"),
    ("spigot", "spigotmc.org"),
];

static HOST: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:[a-z][a-z0-9+.-]*://)?(?:www\.|m\.)?([^/:?#]+)"));
static NOISE: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:utm_|ref|fbclid|gclid|sp$|si$)"));

pub fn host_of(url: &str) -> String {
    HOST.captures(url).map(|m| m[1].to_lowercase()).unwrap_or_default()
}

fn load(home: &Path) -> Map<String, Value> {
    match fs::read_to_string(home.join("sites.json")).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn seed(host: &str) -> Option<&'static str> {
    SEED.iter().find(|(h, _)| *h == host).map(|(_, tpl)| *tpl)
}

pub fn named(home: &Path, word: &str) -> Option<String> {
    let w = word.trim().to_lowercase();
    if let Some((_, host)) = NAMES.iter().find(|(name, _)| *name == w) {
        return Some((*host).to_string());
    }
    let mut known: Vec<String> = SEED.iter().map(|(h, _)| (*h).to_string()).collect();
    known.extend(load(home).keys().cloned());
    known.sort();
    known.dedup();
    if known.contains(&w) {
        return Some(w);
    }
    known.into_iter().find(|h| h.split('.').next() == Some(w.as_str()) || *h == format!("{w}.com"))
}

pub fn template(home: &Path, host: &str) -> Option<String> {
    let host = host_of(host);
    let labels: Vec<&str> = host.split('.').collect();
    let short = labels[labels.len().saturating_sub(2)..].join(".");
    let data = load(home);
    for h in [host.as_str(), short.as_str()] {
        if let Some(tpl) = data.get(h).and_then(|s| s.get("search")).and_then(Value::as_str).filter(|t| !t.is_empty()) {
            return Some(tpl.to_string());
        }
        if let Some(tpl) = seed(h) {
            return Some(tpl.to_string());
        }
    }
    None
}

fn quote(text: &str, safe: &str, plus: bool) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(b as char),
            b' ' if plus => out.push('+'),
            _ if safe.as_bytes().contains(&b) => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn unquote(text: &str, plus: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
                continue;
            }
            (b'+', _) if plus => out.push(b' '),
            _ => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn build(tpl: &str, words: &str) -> String {
    match tpl.split_once('?') {
        Some((base, query)) if query.contains("{q}") => {
            format!("{base}?{}", query.replace("{q}", &quote(words, "", true)))
        }
        _ => tpl.replace("{q}", &quote(words, "/", false)),
    }
}

fn split(url: &str) -> (&str, &str, &str, &str) {
    let (rest, _) = url.split_once('#').unwrap_or((url, ""));
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (scheme, rest) = match rest.split_once("://") {
        Some((scheme, rest)) => (scheme, rest),
        None => ("", rest),
    };
    let (netloc, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    (scheme, netloc, path, query)
}

fn derive(url: &str, value: &str) -> Option<String> {
    let value = value.trim().to_lowercase();
    if value.chars().count() < 2 || url.is_empty() {
        return None;
    }
    let (scheme, netloc, path, query) = split(url);
    let join = |path: &str, query: &str| {
        let q = if query.is_empty() { String::new() } else { format!("?{query}") };
        format!("{scheme}://{netloc}{path}{q}")
    };
    let pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (unquote(k, true), unquote(v, true))
        })
        .collect();
    let hits: Vec<usize> = (0..pairs.len()).filter(|i| pairs[*i].1.trim().to_lowercase() == value).collect();
    if let [found] = *hits.as_slice() {
        let kept: Vec<String> = pairs
            .iter()
            .enumerate()
            .filter(|(i, (k, _))| *i == found || !NOISE.is_match(k))
            .map(|(i, (k, v))| {
                let v = if i == found { "{q}".to_string() } else { quote(v, "", true) };
                format!("{}={v}", quote(k, "", true))
            })
            .collect();
        return Some(join(path, &kept.join("&")));
    }
    let mut segs: Vec<String> = path.split('/').map(String::from).collect();
    let hits: Vec<usize> = (0..segs.len()).filter(|i| unquote(&segs[*i], false).to_lowercase() == value).collect();
    if let [found] = *hits.as_slice() {
        segs[found] = "{q}".into();
        return Some(join(&segs.join("/"), ""));
    }
    None
}

pub fn learn(home: &Path, url: &str, value: &str) -> Option<String> {
    let tpl = derive(url, value)?;
    let host = host_of(url);
    if host.is_empty() {
        return None;
    }
    let mut data = load(home);
    if data.get(&host).and_then(|s| s.get("search")).and_then(Value::as_str) == Some(tpl.as_str()) {
        return None;
    }
    match data.get_mut(&host) {
        Some(Value::Object(site)) => {
            site.insert("search".into(), json!(tpl));
        }
        _ => {
            data.insert(host, json!({"search": tpl}));
        }
    }
    if let Ok(text) = serde_json::to_string_pretty(&Value::Object(data)) {
        let _ = fs::create_dir_all(home).and_then(|()| fs::write(home.join("sites.json"), text));
    }
    Some(tpl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_urls_are_built_and_learned_like_2x() {
        let home = std::env::temp_dir().join(format!("lt-sites-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        assert_eq!(host_of("https://www.YouTube.com/watch?v=1"), "youtube.com");
        assert_eq!(host_of("m.reddit.com/r/x"), "reddit.com");
        assert_eq!(named(&home, "yt").as_deref(), Some("youtube.com"));
        assert_eq!(named(&home, "github").as_deref(), Some("github.com"));
        assert_eq!(named(&home, "zzz"), None);
        assert_eq!(template(&home, "https://music.youtube.com/x").as_deref(), Some(SEED[0].1));
        assert_eq!(
            build(SEED[0].1, "lofi beats & c/d"),
            "https://www.youtube.com/results?search_query=lofi+beats+%26+c%2Fd"
        );
        assert_eq!(build("https://x.test/s/{q}", "a b/c"), "https://x.test/s/a%20b/c");
        assert_eq!(
            derive("https://shop.test/find?utm_source=x&query=Red+Shoes&page=2#top", "red shoes").as_deref(),
            Some("https://shop.test/find?query={q}&page=2")
        );
        assert_eq!(
            derive("https://wiki.test/search/Red%20Shoes", "red shoes").as_deref(),
            Some("https://wiki.test/search/{q}")
        );
        assert_eq!(derive("https://x.test/?a=b", "zz"), None);
        assert_eq!(
            learn(&home, "https://shop.test/find?q=boots", "boots").as_deref(),
            Some("https://shop.test/find?q={q}")
        );
        assert_eq!(learn(&home, "https://shop.test/find?q=boots", "boots"), None);
        assert_eq!(template(&home, "shop.test").as_deref(), Some("https://shop.test/find?q={q}"));
        assert_eq!(named(&home, "shop").as_deref(), Some("shop.test"));
        fs::remove_dir_all(&home).unwrap();
    }
}
