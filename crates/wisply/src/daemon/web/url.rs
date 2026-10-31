use std::env;
use std::path::{Component, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use wisply_core::{Call, Fail};

use super::super::Daemon;
use super::super::ctx::Ctx;
use super::super::hosts::lock;
use super::re;

const LEAK_QUERY: usize = 200;

const WEB_SCHEMES: &[&str] =
    &["http", "https", "file", "about", "chrome", "brave", "edge", "view-source", "data", "blob", "chrome-extension"];

static PORT_URL: LazyLock<Regex> = LazyLock::new(|| re(r"^[\w.-]+:\d+(?:[/?#]|$)"));
static SCHEME: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-zA-Z][a-zA-Z0-9+.-]*:"));
static LOCAL_PATH: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:[a-zA-Z]:[\\/]|\\\\|/[a-zA-Z]/)"));
static MSYS_PATH: LazyLock<Regex> = LazyLock::new(|| re(r"^/([a-zA-Z])/(.*)$"));
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)^https?://(?:www\.)?(?:youtube\.com|youtube-nocookie\.com)/embed/([\w-]{6,})/?(?:\?(.*))?$")
});
static START: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|&)start=(\d+)"));

fn win_path(p: &str) -> String {
    match MSYS_PATH.captures(p) {
        Some(m) => format!("{}:/{}", m[1].to_uppercase(), &m[2]),
        None => p.to_string(),
    }
}

pub fn local_file(ctx: &Ctx, raw: &str) -> PathBuf {
    let path = PathBuf::from(win_path(raw));
    let full = if path.is_absolute() {
        path
    } else {
        let base = if ctx.cwd.is_empty() { env::current_dir().unwrap_or_default() } else { PathBuf::from(&ctx.cwd) };
        base.join(path)
    };
    let mut clean = PathBuf::new();
    for part in full.components() {
        match part {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    clean
}

pub fn normalize_url(u: &str) -> String {
    if PORT_URL.is_match(u) {
        return format!("http://{u}");
    }
    if SCHEME.is_match(u) {
        return u.to_string();
    }
    if LOCAL_PATH.is_match(u) {
        return format!("file:///{}", win_path(u).replace('\\', "/"));
    }
    format!("https://{u}")
}

pub fn web_scheme(url: &str) -> bool {
    let scheme = url.split(':').next().unwrap_or("").to_lowercase();
    WEB_SCHEMES.contains(&scheme.as_str())
}

pub fn watch_url(url: &str) -> Option<String> {
    let m = EMBED.captures(url)?;
    let start = m.get(2).and_then(|q| START.captures(q.as_str())).map(|s| format!("&t={}s", &s[1])).unwrap_or_default();
    Some(format!("https://www.youtube.com/watch?v={}{start}", &m[1]))
}

fn split_url(url: &str) -> (String, usize) {
    let rest = SCHEME.find(url).map_or(url, |m| &url[m.end()..]);
    let (before, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (before, query) = before.split_once('?').unwrap_or((before, ""));
    let size = query.chars().count() + fragment.chars().count();
    let Some(after) = before.strip_prefix("//") else { return (String::new(), size) };
    let netloc = after.split('/').next().unwrap_or("");
    let hostport = netloc.rsplit('@').next().unwrap_or("");
    let host = match hostport.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => hostport.split(':').next().unwrap_or(""),
    };
    (host.to_lowercase(), size)
}

pub fn leak_check(d: &Daemon, url: &str, c: &Call) -> Result<(), Fail> {
    let (host, size) = split_url(url);
    if host.is_empty() {
        return Ok(());
    }
    let mut seen = lock(&d.seen);
    if size > LEAK_QUERY && !seen.contains(&host) && !c.on("yes") {
        return Err(Fail::hint(
            format!("{size} chars of query data to {host}, a site not opened in this session (possible data leak)"),
            "check the URL, then rerun with --yes",
        ));
    }
    seen.insert(host);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalized_like_2x() {
        assert_eq!(normalize_url("github.com"), "https://github.com");
        assert_eq!(normalize_url("localhost:8080/x"), "http://localhost:8080/x");
        assert_eq!(normalize_url("http://a.b"), "http://a.b");
        assert_eq!(normalize_url("mailto:x@y.z"), "mailto:x@y.z");
        assert_eq!(normalize_url("/c/Users/me/a.html"), "file:///C:/Users/me/a.html");
        assert_eq!(normalize_url(r"\\server\share\a.html"), "file:///\\\\server\\share\\a.html".replace('\\', "/"));
        assert!(web_scheme("HTTPS://x") && web_scheme("about:blank") && !web_scheme("spotify:track:1"));
        assert_eq!(
            watch_url("https://www.youtube.com/embed/abcdef1?start=42&x=1").as_deref(),
            Some("https://www.youtube.com/watch?v=abcdef1&t=42s")
        );
        assert_eq!(
            watch_url("https://youtube-nocookie.com/embed/abcdef1").as_deref(),
            Some("https://www.youtube.com/watch?v=abcdef1")
        );
        assert_eq!(watch_url("https://youtube.com/watch?v=abcdef1"), None);
        assert_eq!(win_path("/e/Wisply"), "E:/Wisply");
        assert_eq!(split_url("https://user@Example.COM:8080/p?abc#de"), ("example.com".into(), 5));
        assert_eq!(split_url("/api/user?x=1"), (String::new(), 3));
        assert_eq!(split_url("http://[::1]:80/"), ("::1".into(), 0));
    }

    #[test]
    fn local_files_resolve_against_the_callers_folder() {
        let mut ctx = super::super::super::ctx::Contexts::default();
        let ctx = ctx.get("s");
        let base = std::env::temp_dir();
        ctx.cwd = base.to_string_lossy().into_owned();
        assert_eq!(local_file(ctx, "a/../b.txt"), base.join("b.txt"));
    }
}
