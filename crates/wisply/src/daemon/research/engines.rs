use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use regex::Regex;
use serde_json::{Map, Value};
use ureq::tls::{RootCerts, TlsConfig};

pub const BASE_VAR: &str = "WISPLY_RESEARCH_BASE";
const TIMEOUT: Duration = Duration::from_secs(5);
const BODY_MAX: u64 = 3_000_000;
const BLOCK_SECS: u64 = 1800;
const STATE: &str = "research/engines.json";
const PER_ENGINE: usize = 10;
const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";

static ANCHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<a\s[^>]*?href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap());
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static CAPTCHA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)captcha|unusual traffic|are you a robot|verify you are human|anomaly-modal|challenge-form")
        .unwrap()
});
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
});

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Page,
    Wikipedia,
    GitHub,
    StackOverflow,
    Reddit,
    HackerNews,
}

pub struct Engine {
    pub name: &'static str,
    pub url: &'static str,
    pub kind: Kind,
    pub own: &'static [&'static str],
    pub host: &'static str,
}

pub const ENGINES: &[Engine] = &[
    Engine {
        name: "duckduckgo",
        url: "https://html.duckduckgo.com/html/?q={q}",
        kind: Kind::Page,
        own: &["duckduckgo.com"],
        host: "",
    },
    Engine {
        name: "bing",
        url: "https://www.bing.com/search?q={q}&count=20",
        kind: Kind::Page,
        own: &["bing.com", "microsoft.com", "msn.com", "go.microsoft.com"],
        host: "",
    },
    Engine {
        name: "brave",
        url: "https://search.brave.com/search?q={q}&source=web",
        kind: Kind::Page,
        own: &["brave.com", "search.brave.com"],
        host: "",
    },
    Engine {
        name: "mojeek",
        url: "https://www.mojeek.com/search?q={q}",
        kind: Kind::Page,
        own: &["mojeek.com"],
        host: "",
    },
    Engine {
        name: "wikipedia-en",
        url: "https://en.wikipedia.org/w/api.php?action=query&list=search&format=json&srlimit=5&srsearch={q}",
        kind: Kind::Wikipedia,
        own: &[],
        host: "en.wikipedia.org",
    },
    Engine {
        name: "wikipedia-de",
        url: "https://de.wikipedia.org/w/api.php?action=query&list=search&format=json&srlimit=5&srsearch={q}",
        kind: Kind::Wikipedia,
        own: &[],
        host: "de.wikipedia.org",
    },
    Engine {
        name: "github",
        url: "https://api.github.com/search/repositories?per_page=6&q={q}",
        kind: Kind::GitHub,
        own: &[],
        host: "github.com",
    },
    Engine {
        name: "stackoverflow",
        url: "https://api.stackexchange.com/2.3/search/advanced?order=desc&sort=relevance&site=stackoverflow&pagesize=6&q={q}",
        kind: Kind::StackOverflow,
        own: &[],
        host: "stackoverflow.com",
    },
    Engine {
        name: "reddit",
        url: "https://www.reddit.com/search.json?limit=6&q={q}",
        kind: Kind::Reddit,
        own: &[],
        host: "reddit.com",
    },
    Engine {
        name: "hackernews",
        url: "https://hn.algolia.com/api/v1/search?hitsPerPage=6&query={q}",
        kind: Kind::HackerNews,
        own: &[],
        host: "news.ycombinator.com",
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub url: String,
    pub title: String,
}

pub enum Outcome {
    Hits(Vec<Hit>),
    Blocked(String),
    Failed,
}

pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(char::from(b)),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn unquote(text: &str) -> String {
    let b = text.as_bytes();
    let hex = |c: u8| char::from(c).to_digit(16);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(hi), Some(lo)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn address(e: &Engine, query: &str, base: Option<&str>) -> String {
    match base {
        Some(base) => format!("{}/{}?q={}", base.trim_end_matches('/'), e.name, quote(query)),
        None => e.url.replace("{q}", &quote(query)),
    }
}

pub fn site_query(e: &Engine, query: &str, site: Option<&str>) -> Option<String> {
    let Some(site) = site else { return Some(query.to_string()) };
    match e.kind {
        Kind::Page => Some(format!("{query} site:{site}")),
        _ if host_of(e.host).ends_with(&host_of(site)) || host_of(site).ends_with(&host_of(e.host)) => {
            Some(query.to_string())
        }
        _ => None,
    }
}

pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").to_lowercase();
    let host = host.rsplit_once('@').map_or(host.as_str(), |(_, h)| h).split(':').next().unwrap_or("").to_string();
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

pub fn ask(e: &Engine, url: &str) -> Outcome {
    let res = AGENT
        .get(url)
        .header("User-Agent", if e.kind == Kind::Page { UA } else { "wisply-research" })
        .header("Accept-Language", "en-US,en;q=0.8,de;q=0.6")
        .call();
    let mut res = match res {
        Ok(r) => r,
        Err(err) => {
            let text = err.to_string();
            return if ["403", "429", "503", "451"].iter().any(|c| text.contains(c)) {
                Outcome::Blocked(text)
            } else {
                Outcome::Failed
            };
        }
    };
    let mut raw = Vec::new();
    if let Err(err) = res.body_mut().as_reader().take(BODY_MAX).read_to_end(&mut raw) {
        return Outcome::Failed;
    }
    let body = String::from_utf8_lossy(&raw);
    parse(e, &body)
}

pub fn parse(e: &Engine, body: &str) -> Outcome {
    let hits = match e.kind {
        Kind::Page => {
            let hits = page_hits(body, e.own);
            if hits.is_empty() && CAPTCHA.is_match(body) {
                return Outcome::Blocked("captcha".into());
            }
            hits
        }
        _ => match serde_json::from_str::<Value>(body) {
            Ok(v) => api_hits(e, &v),
            Err(_) => return Outcome::Failed,
        },
    };
    Outcome::Hits(hits.into_iter().take(PER_ENGINE).collect())
}

fn clean(html: &str) -> String {
    let text = TAGS.replace_all(html, " ");
    let text = text
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    SPACE.replace_all(&text, " ").trim().to_string()
}

fn real_target(href: &str) -> String {
    let href = href.replace("&amp;", "&");
    let href = if href.starts_with("//") { format!("https:{href}") } else { href };
    let param = |name: &str| {
        href.split(['?', '&']).find_map(|kv| kv.strip_prefix(name).and_then(|v| v.strip_prefix('='))).map(unquote)
    };
    if host_of(&href).ends_with("duckduckgo.com")
        && let Some(target) = param("uddg")
    {
        return target;
    }
    if host_of(&href).ends_with("bing.com")
        && let Some(u) = param("u")
        && let Some(code) = u.strip_prefix("a1")
        && let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(code.trim_end_matches('='))
    {
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    href
}

pub fn page_hits(body: &str, own: &[&str]) -> Vec<Hit> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for cap in ANCHOR.captures_iter(body) {
        let url = real_target(&cap[1]);
        if !url.starts_with("http") {
            continue;
        }
        let host = host_of(&url);
        if host.is_empty() || own.iter().any(|o| host == *o || host.ends_with(&format!(".{o}"))) {
            continue;
        }
        let title = clean(&cap[2]);
        if title.chars().count() < 8 || (title.starts_with("http") && title.chars().count() < 25) {
            continue;
        }
        if seen.insert(url.trim_end_matches('/').to_string()) {
            out.push(Hit { url, title });
        }
    }
    out
}

fn list<'a>(v: &'a Value, path: &[&str]) -> &'a [Value] {
    path.iter().try_fold(v, |acc, key| acc.get(*key)).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn api_hits(e: &Engine, v: &Value) -> Vec<Hit> {
    let hit =
        |url: String, title: String| (!url.is_empty() && !title.is_empty()).then(|| Hit { url, title: clean(&title) });
    match e.kind {
        Kind::Wikipedia => list(v, &["query", "search"])
            .iter()
            .filter_map(|r| {
                let title = text(r, "title");
                hit(format!("https://{}/wiki/{}", e.host, title.replace(' ', "_")), title)
            })
            .collect(),
        Kind::GitHub => list(v, &["items"])
            .iter()
            .filter_map(|r| {
                let about = text(r, "description");
                let name = text(r, "full_name");
                hit(text(r, "html_url"), if about.is_empty() { name } else { format!("{name}: {about}") })
            })
            .collect(),
        Kind::StackOverflow => {
            list(v, &["items"]).iter().filter_map(|r| hit(text(r, "link"), text(r, "title"))).collect()
        }
        Kind::Reddit => list(v, &["data", "children"])
            .iter()
            .filter_map(|r| {
                let d = r.get("data")?;
                hit(format!("https://www.reddit.com{}", text(d, "permalink")), text(d, "title"))
            })
            .collect(),
        Kind::HackerNews => list(v, &["hits"])
            .iter()
            .filter_map(|r| {
                let url = text(r, "url");
                let url = if url.is_empty() {
                    format!("https://news.ycombinator.com/item?id={}", text(r, "objectID"))
                } else {
                    url
                };
                hit(url, text(r, "title"))
            })
            .collect(),
        Kind::Page => Vec::new(),
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn load(home: &Path) -> Map<String, Value> {
    fs::read_to_string(home.join(STATE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub fn blocked(home: &Path, name: &str) -> bool {
    load(home).get(name).and_then(Value::as_u64).is_some_and(|until| until > now())
}

pub fn mark(home: &Path, name: &str, block: bool) {
    let mut state = load(home);
    let had = state.contains_key(name);
    if block {
        state.insert(name.into(), Value::from(now() + BLOCK_SECS));
    } else if had {
        state.remove(name);
    } else {
        return;
    }
    let path = home.join(STATE);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, Value::Object(state).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(name: &str) -> &'static Engine {
        ENGINES.iter().find(|e| e.name == name).unwrap()
    }

    #[test]
    fn result_pages_give_their_real_links() {
        let ddg = r#"<a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fpapermc.io%2Fsoftware%2Ffolia&amp;rut=x">Folia | PaperMC</a>
            <a href="https://duckduckgo.com/y.js?ad=1">Sponsored result here</a>"#;
        let Outcome::Hits(hits) = parse(engine("duckduckgo"), ddg) else { panic!() };
        assert_eq!(hits, [Hit { url: "https://papermc.io/software/folia".into(), title: "Folia | PaperMC".into() }]);
        let code = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("https://github.com/a/b");
        let bing = format!(
            r#"<h2><a href="https://www.bing.com/ck/a?!&amp;p=1&amp;u=a1{code}&amp;ntb=1">A <b>library</b> for Folia</a></h2>"#
        );
        let Outcome::Hits(hits) = parse(engine("bing"), &bing) else { panic!() };
        assert_eq!(hits[0].url, "https://github.com/a/b");
        assert_eq!(hits[0].title, "A library for Folia");
        assert!(matches!(parse(engine("mojeek"), "<form id=captcha>"), Outcome::Blocked(_)));
    }

    #[test]
    fn apis_become_hits() {
        let wiki = r#"{"query":{"search":[{"title":"Rust (programming language)"}]}}"#;
        let Outcome::Hits(h) = parse(engine("wikipedia-en"), wiki) else { panic!() };
        assert_eq!(h[0].url, "https://en.wikipedia.org/wiki/Rust_(programming_language)");
        let gh = r#"{"items":[{"html_url":"https://github.com/x/y","full_name":"x/y","description":"Folia lib"}]}"#;
        let Outcome::Hits(h) = parse(engine("github"), gh) else { panic!() };
        assert_eq!(h[0].title, "x/y: Folia lib");
        let hn = r#"{"hits":[{"url":"","objectID":"42","title":"Ask HN"}]}"#;
        let Outcome::Hits(h) = parse(engine("hackernews"), hn) else { panic!() };
        assert_eq!(h[0].url, "https://news.ycombinator.com/item?id=42");
    }

    #[test]
    fn queries_are_quoted_and_limited_to_a_site() {
        assert_eq!(quote("a b&c"), "a+b%26c");
        assert_eq!(unquote("a+b%26c"), "a b&c");
        assert_eq!(site_query(engine("bing"), "x", Some("github.com")).as_deref(), Some("x site:github.com"));
        assert_eq!(site_query(engine("github"), "x", Some("github.com")).as_deref(), Some("x"));
        assert_eq!(site_query(engine("reddit"), "x", Some("github.com")), None);
        assert_eq!(address(engine("bing"), "x y", Some("http://127.0.0.1:9/")), "http://127.0.0.1:9/bing?q=x+y");
        assert_eq!(host_of("https://www.Example.com:8080/a"), "example.com");
    }
}
