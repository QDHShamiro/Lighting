use std::fs::File;
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use lt_core::{Call, Fail, Flag};
use regex::{Captures, Regex};
use ureq::ResponseExt;
use ureq::tls::{RootCerts, TlsConfig};

use super::super::Daemon;
use super::super::channels::{self, github, ledger};
use super::super::ctx::Ctx;
use super::super::words::{hit, terms};
use super::shot::image_note;
use super::url::{leak_check, local_file};
use super::{TEXT_CHARS, any_browser, capped, cmds, head, html, number, re};

const IMAGE_EXT: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"];
const HTML_EXT: &[&str] = &[".html", ".htm", ".xhtml"];
const PDF_BYTES: u64 = 30_000_000;
const PAGE_BYTES: u64 = 4_000_000;
const TIMEOUT: Duration = Duration::from_secs(20);
const EACH_MIN: usize = 1500;
const PARALLEL: usize = 8;
const ACCEPT: &str = "text/markdown, text/plain;q=0.9, text/html;q=0.8, */*;q=0.1";

static URL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(https?|file)://"));
static BLOCKED: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)bot check|http status: (401|403|429|451|503)\b"));
static CODE: LazyLock<Regex> = LazyLock::new(|| re(r"```[\s\S]*?```|`[^`\n]*`"));
static LINK: LazyLock<Regex> = LazyLock::new(|| re(r"!?\[([^\]]*)\]\((?:[^()]|\([^()]*\))*\)"));
static FOOT: LazyLock<Regex> = LazyLock::new(|| re(r"\[\^[^\]]+\](:?)"));
static PARA: LazyLock<Regex> = LazyLock::new(|| re(r"\n\s*\n"));
static GATE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)<title>\s*(just a moment|attention required|access denied|verify you are human|are you a robot|checking your browser)",
    )
});
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
});

enum Source {
    Web(String),
    File(PathBuf),
}

struct Got {
    raw: Vec<u8>,
    ctype: String,
    place: String,
    local: bool,
}

struct Page {
    title: String,
    text: String,
    place: String,
    how: String,
}

pub fn wants(first: &str) -> bool {
    let lower = first.to_lowercase();
    URL.is_match(first) || lower.ends_with(".pdf") || IMAGE_EXT.iter().any(|e| lower.ends_with(e))
}

pub fn read(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let chars = number(c, "max")?.filter(|n| *n > 0).map_or(TEXT_CHARS, |n| usize::try_from(n).unwrap_or(TEXT_CHARS));
    for url in c.pos.iter().filter(|u| URL.is_match(u)) {
        leak_check(d, url, c)?;
    }
    let mut lead = String::new();
    if let [one] = c.pos.as_slice()
        && !c.on("page")
        && let Some(page) = github::parse(one)
    {
        match github::read(&d.home, &page) {
            Ok(text) => return Ok(capped(d, ctx, &text, "read", None, chars)),
            Err(e) => lead = format!("! github: {} (read the page instead)\n", e.text()),
        }
    }
    let sources: Vec<Source> = c.pos.iter().map(|u| source(ctx, u)).collect();
    if let [one] = sources.as_slice() {
        let got = match one {
            Source::Web(url) => direct_or_browser(d, ctx, url, c, chars)?,
            Source::File(_) => Ok(grab(one)),
        };
        return match got {
            Ok(got) => read_one(d, ctx, got, &c.pos[0], c, chars).map(|t| format!("{lead}{t}")),
            Err(text) => Ok(format!("{lead}{text}")),
        };
    }
    let each = (chars / sources.len().max(1)).max(EACH_MIN);
    let parts: Vec<String> = grab_all(&sources)
        .into_iter()
        .zip(&c.pos)
        .map(|(got, url)| read_one(d, ctx, got, url, c, each).unwrap_or_else(|e| format!("[read] {url}\n{}", e.text())))
        .collect();
    Ok(parts.join("\n\n"))
}

fn read_one(
    d: &Daemon,
    ctx: &mut Ctx,
    got: Result<Got, Fail>,
    url: &str,
    c: &Call,
    chars: usize,
) -> Result<String, Fail> {
    let page = render(d, ctx, got?, url)?;
    let mut text = if c.on("links") { page.text } else { plain_links(&page.text) };
    if let Some(f) = c.val("f").filter(|f| !f.is_empty()) {
        text = paragraphs(&text, f);
    }
    let top = format!("[read] {} - {} ({})", head(&page.title, 70), head(&page.place, 90), page.how);
    Ok(capped(d, ctx, &format!("{top}\n{text}"), "read", None, chars))
}

fn gated(got: &Got) -> bool {
    got.ctype.contains("html") && GATE.is_match(&String::from_utf8_lossy(&got.raw[..got.raw.len().min(30_000)]))
}

type Tried = Result<Result<Got, Fail>, String>;

fn direct_or_browser(d: &Daemon, ctx: &mut Ctx, url: &str, c: &Call, chars: usize) -> Result<Tried, Fail> {
    let key = format!("web:{}", channels::site(url));
    if ledger::skip(&d.home, &key, "direct", "browser") && any_browser(d) {
        return browser(d, ctx, url, c, chars, "the direct read failed on this site before").map(Err);
    }
    let got = grab(&Source::Web(url.to_string()));
    let why = match &got {
        Ok(g) if gated(g) => "the site showed a bot check".to_string(),
        Ok(_) => {
            ledger::note(&d.home, &key, "direct", true, "");
            ledger::note(&d.home, "web", "direct", true, "");
            return Ok(Ok(got));
        }
        Err(e) => e.text().lines().next().unwrap_or("").trim_start_matches("err: ").to_string(),
    };
    ledger::note(&d.home, &key, "direct", false, &why);
    ledger::note(&d.home, "web", "direct", false, &why);
    if !BLOCKED.is_match(&why) || !any_browser(d) {
        return Ok(Ok(got));
    }
    browser(d, ctx, url, c, chars, &why).map(Err)
}

fn browser(d: &Daemon, ctx: &mut Ctx, url: &str, c: &Call, chars: usize, why: &str) -> Result<String, Fail> {
    let key = format!("web:{}", channels::site(url));
    let mut open = Call { name: "open".into(), pos: vec![url.to_string()], ..Call::default() };
    open.flags.insert("new".into(), Flag::On);
    let mut text = Call { name: "text".into(), ..Call::default() };
    if let Some(f) = c.flags.get("f") {
        text.flags.insert("f".into(), f.clone());
    }
    let (target, tabs) = (ctx.target.clone(), ctx.tabs.clone());
    let got = cmds::open(d, ctx, &open).and_then(|_| cmds::text(d, ctx, &text));
    if let Some(tab) = ctx.tabs.iter().find(|t| !tabs.contains(t)).cloned() {
        let _ = cmds::close(d, ctx, &Call { name: "close".into(), pos: vec![tab], ..Call::default() });
    }
    ctx.target = target;
    let failed = got.as_ref().err().map(Fail::text).unwrap_or_default();
    ledger::note(&d.home, &key, "browser", got.is_ok(), &failed);
    ledger::note(&d.home, "web", "browser", got.is_ok(), &failed);
    let body = got?;
    let top = format!("[read] {} (in the browser: {})", head(url, 90), head(why, 120));
    Ok(capped(d, ctx, &format!("{top}\n{body}"), "read", None, chars))
}

fn source(ctx: &Ctx, url: &str) -> Source {
    if !URL.is_match(url) {
        return Source::File(local_file(ctx, url));
    }
    match url.get(..7).filter(|s| s.eq_ignore_ascii_case("file://")) {
        Some(_) => Source::File(local_file(ctx, &file_path(&url[7..]))),
        None => Source::Web(url.to_string()),
    }
}

fn file_path(rest: &str) -> String {
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let path = unquote(rest.split(['?', '#']).next().unwrap_or(""));
    let b = path.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        path[1..].to_string()
    } else {
        path
    }
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
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
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn grab_all(sources: &[Source]) -> Vec<Result<Got, Fail>> {
    let mut out = Vec::with_capacity(sources.len());
    for chunk in sources.chunks(PARALLEL) {
        thread::scope(|s| {
            let jobs: Vec<_> = chunk.iter().map(|src| s.spawn(move || grab(src))).collect();
            out.extend(jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| Err(Fail::new("read crashed")))));
        });
    }
    out
}

fn grab(src: &Source) -> Result<Got, Fail> {
    let url = match src {
        Source::File(path) => {
            let mut raw = Vec::new();
            File::open(path)
                .and_then(|f| f.take(PDF_BYTES).read_to_end(&mut raw))
                .map_err(|e| Fail::new(format!("cannot read {}: {e}", path.display())))?;
            return Ok(Got { raw, ctype: String::new(), place: path.display().to_string(), local: true });
        }
        Source::Web(url) => url,
    };
    let failed = |e: String| {
        Fail::hint(format!("read failed: {}", head(&e, 200)), format!("wisply open {url} (uses the real browser)"))
    };
    let agent = format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) wisply/{}", env!("CARGO_PKG_VERSION"));
    let mut res = AGENT
        .get(url.as_str())
        .header("Accept", ACCEPT)
        .header("User-Agent", &agent)
        .call()
        .map_err(|e| failed(e.to_string()))?;
    let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let place = res.get_uri().to_string();
    let limit = if ctype.contains("pdf") || bare(url).ends_with(".pdf") { PDF_BYTES } else { PAGE_BYTES };
    let mut raw = Vec::new();
    res.body_mut().as_reader().take(limit).read_to_end(&mut raw).map_err(|e| failed(e.to_string()))?;
    Ok(Got { raw, ctype, place, local: false })
}

fn bare(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or("").to_lowercase()
}

fn render(d: &Daemon, ctx: &mut Ctx, got: Got, url: &str) -> Result<Page, Fail> {
    let page = |title: String, text: String, how: String| Page { title, text, place: got.place.clone(), how };
    let path = bare(&got.place);
    let ends = |exts: &[&str]| exts.iter().any(|e| path.ends_with(e));
    let pdf = got.raw.starts_with(b"%PDF-") || got.ctype.contains("pdf") || path.ends_with(".pdf");
    if (got.local && ends(IMAGE_EXT)) || got.ctype.starts_with("image/") {
        return Ok(page(String::new(), image_note(d, ctx, &got.raw)?, "image".into()));
    }
    if pdf {
        let (text, n) = pdf_text(&got.raw)?;
        return Ok(page(String::new(), text, format!("pdf, {n} pages")));
    }
    let body = decode(&got.raw, &got.ctype);
    if got.ctype.contains("html") || (got.local && ends(HTML_EXT)) {
        let (title, text) = html::convert(&body, &got.place);
        return Ok(page(title, text, "html converted".into()));
    }
    if got.local {
        return Ok(page(String::new(), body.trim().to_string(), "text file".into()));
    }
    if got.ctype.contains("markdown") || got.ctype.contains("text/plain") {
        return Ok(page(String::new(), body.trim().to_string(), "markdown from server".into()));
    }
    let kind = got.ctype.split(';').next().unwrap_or("");
    Err(Fail::hint(format!("not a text page ({kind})"), format!("wisply open {url}")))
}

fn decode(raw: &[u8], ctype: &str) -> String {
    let label = ctype
        .split(';')
        .filter_map(|p| p.trim().strip_prefix("charset="))
        .map(|c| c.trim_matches(['"', '\'', ' ']))
        .next()
        .unwrap_or("utf-8");
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    encoding.decode(raw).0.into_owned()
}

fn pdf_text(raw: &[u8]) -> Result<(String, usize), Fail> {
    let pages = panic::catch_unwind(AssertUnwindSafe(|| pdf_extract::extract_text_from_mem_by_pages(raw)))
        .map_err(|_| Fail::new("cannot read this pdf (the pdf reader crashed)"))?
        .map_err(|e| Fail::new(format!("cannot read this pdf ({})", head(&e.to_string(), 120))))?;
    let text: Vec<String> = pages.iter().enumerate().map(|(i, p)| format!("[p{}] {}", i + 1, p.trim())).collect();
    Ok((text.join("\n\n"), pages.len()))
}

pub fn plain_links(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut last = 0;
    for code in CODE.find_iter(md) {
        out.push_str(&plain(&md[last..code.start()]));
        out.push_str(code.as_str());
        last = code.end();
    }
    out.push_str(&plain(&md[last..]));
    out
}

fn plain(part: &str) -> String {
    let unlinked = LINK.replace_all(part, "$1");
    FOOT.replace_all(&unlinked, |m: &Captures| if m[1].is_empty() { String::new() } else { m[0].to_string() })
        .into_owned()
}

fn paragraphs(text: &str, f: &str) -> String {
    let words = terms(f);
    let kept: Vec<&str> = PARA.split(text).filter(|p| hit(&words, &[*p])).collect();
    if kept.is_empty() { format!("(no paragraph contains '{f}')") } else { kept.join("\n\n") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_loses_links_but_keeps_code() {
        let md = "see [docs](https://a.de/x_(y)) and ![pic](p.png) note[^1]\n[^1]: foot\n`[k](v)` ```\n[a](b)\n```";
        assert_eq!(plain_links(md), "see docs and pic note\n[^1]: foot\n`[k](v)` ```\n[a](b)\n```");
        let text = "alpha one\n\nbeta two\n  \ngamma alpha";
        assert_eq!(paragraphs(text, "ALPHA"), "alpha one\n\ngamma alpha");
        assert_eq!(paragraphs(text, "zeta"), "(no paragraph contains 'zeta')");
    }

    #[test]
    fn sources_and_bodies_are_understood() {
        assert!(wants("https://x.y") && wants("FILE:///c:/a.txt") && wants("a/b.PDF") && wants("c.webp"));
        assert!(!wants("w3") && !wants("notes.txt"));
        assert_eq!(file_path("/C:/My%20Docs/a.pdf"), "C:/My Docs/a.pdf");
        assert_eq!(file_path("localhost/home/me/x%zz.html#top"), "/home/me/x%zz.html");
        assert_eq!(decode(&[0x63, 0x61, 0x66, 0xe9], "text/html; charset=ISO-8859-1"), "caf\u{e9}");
        assert_eq!(decode("caf\u{e9}".as_bytes(), "text/plain"), "caf\u{e9}");
        assert!(pdf_text(b"%PDF-1.4 broken").is_err());
        assert_eq!(bare("https://a.de/X.PDF?dl=1"), "https://a.de/x.pdf");
    }
}
