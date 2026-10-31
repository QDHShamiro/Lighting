use std::sync::LazyLock;

use regex::Regex;

use super::entity::unescape;
use super::re;

const SKIP: &[&str] = &[
    "script", "style", "noscript", "svg", "template", "iframe", "canvas", "form", "button", "select", "nav", "footer",
    "aside", "head", "object", "dialog",
];
const BLOCK: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "li",
    "ul",
    "ol",
    "table",
    "tr",
    "blockquote",
    "figure",
    "figcaption",
    "dl",
    "dt",
    "dd",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "pre",
    "hr",
    "br",
];
const VOID: &[&str] =
    &["br", "hr", "img", "input", "meta", "link", "source", "area", "base", "col", "embed", "param", "track", "wbr"];
const RAW: &[&str] = &["script", "style"];
const HIDE_CLASSES: &[&str] =
    &["hidden", "sr-only", "visually-hidden", "d-none", "hide", "invisible", "screen-reader-text", "offscreen"];
const SCOPE_MIN: usize = 200;
const LINK_CHARS: usize = 80;
static HIDE_STYLE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)display\s*:\s*none|visibility\s*:\s*hidden|(?:width|height)\s*:\s*[01]px|clip\s*:\s*rect\(\s*0|left\s*:\s*-\d{3,}px|text-indent\s*:\s*-\d{3,}px",
    )
});
static HIDE_ZERO: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:font-size|opacity)\s*:\s*0([.\d]*)"));
static STYLE_BLOCK: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<style[^>]*>(.*?)</style>"));
static RULE: LazyLock<Regex> = LazyLock::new(|| re(r"([^{}]+)\{([^{}]*)\}"));
static CLASS: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*\.([\w-]+)\s*$"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));
static EMPTY_LINK: LazyLock<Regex> = LazyLock::new(|| re(r"\[\]\([^)]*\)"));
static GAPS: LazyLock<Regex> = LazyLock::new(|| re(r"\n{3,}"));
static SCHEME: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-zA-Z][a-zA-Z0-9+.-]*:"));

fn hides(style: &str) -> bool {
    HIDE_STYLE.is_match(style)
        || HIDE_ZERO.captures_iter(style).any(|m| !m[1].bytes().any(|b| (b'1'..=b'9').contains(&b)))
}

fn hidden_classes(html: &str) -> Vec<String> {
    let mut found: Vec<String> = HIDE_CLASSES.iter().map(|c| (*c).to_string()).collect();
    for block in STYLE_BLOCK.captures_iter(html) {
        for rule in RULE.captures_iter(&block[1]) {
            if hides(&rule[2]) {
                found.extend(rule[1].split(',').filter_map(|sel| CLASS.captures(sel)).map(|m| m[1].to_string()));
            }
        }
    }
    found
}

struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    closes: bool,
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn tag_at(src: &str) -> Option<(Tag, usize)> {
    let bytes = src.as_bytes();
    let end_tag = bytes.get(1) == Some(&b'/');
    let start = if end_tag { 2 } else { 1 };
    if !bytes.get(start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let name_len =
        src[start..].find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/').unwrap_or(src.len() - start);
    let name = src[start..start + name_len].to_ascii_lowercase();
    let mut i = start + name_len;
    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => return None,
            Some(b'>') => return Some((Tag { name, attrs, closes: false }, i + 1)),
            Some(b'/') if !end_tag && bytes.get(i + 1) == Some(&b'>') => {
                return Some((Tag { name, attrs, closes: true }, i + 2));
            }
            Some(b'/') => {
                i += 1;
                continue;
            }
            Some(_) => {}
        }
        let key_len = src[i..]
            .find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '>' || c == '/')
            .unwrap_or(src.len() - i)
            .max(1);
        let key = src[i..i + key_len].to_ascii_lowercase();
        i += key_len;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            match bytes.get(i) {
                Some(q @ (b'"' | b'\'')) => {
                    let close = src[i + 1..].find(*q as char).map_or(src.len(), |n| i + 1 + n);
                    value = unescape(&src[i + 1..close]);
                    i = (close + 1).min(src.len());
                }
                _ => {
                    let len = src[i..].find(|c: char| c.is_ascii_whitespace() || c == '>').unwrap_or(src.len() - i);
                    value = unescape(&src[i..i + len]);
                    i += len;
                }
            }
        }
        if !end_tag {
            attrs.push((key, value));
        }
    }
}

fn find_close(html: &str, from: usize, name: &str) -> usize {
    let bytes = html.as_bytes();
    let mut i = from;
    while let Some(n) = html[i..].find("</") {
        let at = i + n;
        if bytes.get(at + 2..at + 2 + name.len()).is_some_and(|b| b.eq_ignore_ascii_case(name.as_bytes())) {
            return at;
        }
        i = at + 2;
    }
    html.len()
}

struct Converter<'a> {
    base: &'a str,
    scope: Option<&'a str>,
    hide: &'a [String],
    inside: usize,
    skip: usize,
    stack: Vec<(String, bool, bool)>,
    out: Vec<String>,
    cur: String,
    pre: usize,
    href: Option<String>,
    title: String,
    in_title: bool,
    cells: Option<Vec<String>>,
}

impl Converter<'_> {
    fn is_hidden(&self, tag: &str, attrs: &[(String, String)]) -> bool {
        if SKIP.contains(&tag) || attr(attrs, "hidden").is_some() || attr(attrs, "aria-hidden") == Some("true") {
            return true;
        }
        if attr(attrs, "style").is_some_and(hides) {
            return true;
        }
        attr(attrs, "class").unwrap_or("").split_whitespace().any(|c| self.hide.iter().any(|h| h == c))
    }

    fn flush(&mut self, prefix: &str) {
        let text = std::mem::take(&mut self.cur);
        let text = if self.pre > 0 { text } else { SPACES.replace_all(&text, " ").trim().to_string() };
        if !text.is_empty() {
            self.out.push(format!("{prefix}{text}"));
        }
    }

    fn start(&mut self, tag: &str, attrs: &[(String, String)]) {
        if tag == "title" {
            self.in_title = true;
        }
        if VOID.contains(&tag) {
            if self.skip == 0 && self.inside > 0 {
                if tag == "br" || tag == "hr" {
                    self.flush("");
                } else if tag == "img"
                    && let Some(alt) = attr(attrs, "alt").filter(|a| !a.is_empty())
                    && !self.is_hidden(tag, attrs)
                {
                    self.cur.push_str(&format!("[img: {alt}]"));
                }
            }
            return;
        }
        let hidden = self.is_hidden(tag, attrs);
        let scoped = self.scope == Some(tag);
        self.stack.push((tag.to_string(), hidden, scoped));
        if scoped {
            self.inside += 1;
        }
        if hidden {
            self.skip += 1;
            return;
        }
        if self.skip > 0 || self.inside == 0 {
            return;
        }
        if BLOCK.contains(&tag) {
            self.flush("");
        }
        match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = usize::from(tag.as_bytes()[1] - b'0');
                self.cur.push_str(&format!("{} ", "#".repeat(level)));
            }
            "li" => self.cur.push_str("- "),
            "pre" => {
                self.pre += 1;
                self.out.push("```".into());
            }
            "code" if self.pre == 0 => self.cur.push('`'),
            "a" => {
                self.href = attr(attrs, "href").map(String::from);
                self.cur.push('[');
            }
            "tr" => self.cells = Some(Vec::new()),
            "td" | "th" if self.cells.is_some() => self.flush(""),
            _ => {}
        }
    }

    fn close(&mut self, tag: &str, hidden: bool, scoped: bool) {
        if scoped {
            self.flush("");
            self.inside = self.inside.saturating_sub(1);
        }
        if hidden {
            self.skip = self.skip.saturating_sub(1);
            return;
        }
        if self.skip > 0 || self.inside == 0 {
            return;
        }
        match tag {
            "a" => {
                let link = self.href.take().unwrap_or_default();
                if link.is_empty() || link.starts_with('#') || link.starts_with("javascript:") {
                    self.cur.push(']');
                } else {
                    let short: String = short_link(self.base, &link).chars().take(LINK_CHARS).collect();
                    self.cur.push_str(&format!("]({short})"));
                }
            }
            "code" if self.pre == 0 => self.cur.push('`'),
            "pre" => {
                self.flush("");
                self.pre = self.pre.saturating_sub(1);
                self.out.push("```".into());
            }
            "td" | "th" if self.cells.is_some() => {
                let cell = SPACES.replace_all(&std::mem::take(&mut self.cur), " ").trim().to_string();
                if let Some(cells) = self.cells.as_mut() {
                    cells.push(cell);
                }
            }
            "tr" if self.cells.is_some() => {
                if let Some(cells) = self.cells.take().filter(|c| c.iter().any(|x| !x.is_empty())) {
                    self.out.push(cells.join(" | "));
                }
            }
            _ if BLOCK.contains(&tag) => self.flush(""),
            _ => {}
        }
    }

    fn end(&mut self, tag: &str) {
        if tag == "title" {
            self.in_title = false;
        }
        if let Some(at) = self.stack.iter().rposition(|(t, ..)| t == tag) {
            let open: Vec<(String, bool, bool)> = self.stack.drain(at..).collect();
            for (t, hidden, scoped) in open.into_iter().rev() {
                self.close(&t, hidden, scoped);
            }
        }
    }

    fn data(&mut self, text: &str) {
        if self.in_title {
            self.title.push_str(text);
            return;
        }
        if self.skip == 0 && self.inside > 0 {
            self.cur.push_str(text);
        }
    }

    fn feed(&mut self, html: &str) {
        let mut i = 0;
        while i < html.len() {
            let Some(lt) = html[i..].find('<').map(|n| i + n) else {
                self.data(&unescape(&html[i..]));
                return;
            };
            if lt > i {
                self.data(&unescape(&html[i..lt]));
            }
            let rest = &html[lt..];
            if rest.starts_with("<!--") {
                i = rest.find("-->").map_or(html.len(), |n| lt + n + 3);
            } else if rest.starts_with("<!") || rest.starts_with("<?") {
                i = rest.find('>').map_or(html.len(), |n| lt + n + 1);
            } else if let Some((tag, used)) = tag_at(rest) {
                i = lt + used;
                if tag.closes && !rest.starts_with("</") {
                    self.start(&tag.name, &tag.attrs);
                    self.end(&tag.name);
                } else if rest.starts_with("</") {
                    self.end(&tag.name);
                } else {
                    self.start(&tag.name, &tag.attrs);
                    if RAW.contains(&tag.name.as_str()) {
                        let stop = find_close(html, i, &tag.name);
                        self.data(&html[i..stop]);
                        i = stop;
                    }
                }
            } else {
                self.data("<");
                i = lt + 1;
            }
        }
    }
}

fn split_url(url: &str) -> (&str, &str) {
    let rest = SCHEME.find(url).map_or(url, |m| &url[m.end()..]);
    let (netloc, path) = match rest.strip_prefix("//") {
        Some(after) => after.split_at(after.find(['/', '?', '#']).unwrap_or(after.len())),
        None => ("", rest),
    };
    (netloc, path.split(['?', '#']).next().unwrap_or(""))
}

fn short_link(base: &str, link: &str) -> String {
    let (base_net, base_path) = split_url(base);
    let (net, path) = if SCHEME.is_match(link) || link.starts_with("//") {
        let (net, path) = split_url(link);
        (net.to_string(), path.to_string())
    } else {
        let path = link.split(['?', '#']).next().unwrap_or("");
        let joined = if path.starts_with('/') {
            path.to_string()
        } else if path.is_empty() {
            base_path.to_string()
        } else {
            format!("{}{path}", &base_path[..base_path.rfind('/').map_or(0, |n| n + 1)])
        };
        (base_net.to_string(), dot_segments(&joined))
    };
    if net == base_net { path } else { format!("{net}{path}") }
}

fn dot_segments(path: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let parts: Vec<&str> = path.split('/').collect();
    for (n, part) in parts.iter().enumerate() {
        let last = n + 1 == parts.len();
        match *part {
            "." => {
                if last {
                    kept.push("");
                }
            }
            ".." => {
                if kept.len() > 1 {
                    kept.pop();
                }
                if last {
                    kept.push("");
                }
            }
            other => kept.push(other),
        }
    }
    kept.join("/")
}

fn run(html: &str, base: &str, scope: Option<&str>, hide: &[String]) -> (String, Vec<String>) {
    let mut c = Converter {
        base,
        scope,
        hide,
        inside: usize::from(scope.is_none()),
        skip: 0,
        stack: Vec::new(),
        out: Vec::new(),
        cur: String::new(),
        pre: 0,
        href: None,
        title: String::new(),
        in_title: false,
        cells: None,
    };
    c.feed(html);
    c.flush("");
    (c.title, c.out)
}

pub fn convert(html: &str, base: &str) -> (String, String) {
    let hide = hidden_classes(html);
    let lower = html.to_ascii_lowercase();
    let scope = if lower.contains("<main") {
        Some("main")
    } else if lower.contains("<article") {
        Some("article")
    } else {
        None
    };
    let (mut title, mut out) = run(html, base, scope, &hide);
    if scope.is_some() && out.iter().map(|l| l.chars().count()).sum::<usize>() < SCOPE_MIN {
        (title, out) = run(html, base, None, &hide);
    }
    let lines: Vec<&str> =
        out.iter().map(String::as_str).filter(|l| !["[", "]", "[]", "-"].contains(&l.trim())).collect();
    let text = EMPTY_LINK.replace_all(&lines.join("\n"), "").to_string();
    let text = GAPS.replace_all(&text, "\n\n").trim().to_string();
    (title.trim().to_string(), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_become_markdown_like_2x() {
        let page = concat!(
            "<html><head><title>T &amp; Co</title><style>.x{display:none}</style></head><body><main><h1>Hi</h1>",
            "<p class=\"x\">SECRET1</p><p style=\"font-size:0\">SECRET2</p><p hidden>SECRET3</p>",
            "<p aria-hidden=\"true\">SECRET4</p><p>Visible <a href=\"/a\">link</a> &mdash; caf&eacute; &#8364;5</p>",
            "<ul><li>one</li><li>two</li></ul><table><tr><td>a</td><td>b</td></tr></table>",
            "<p style=\"font-size:0.9em\">small but shown</p><script>if (a < b) { x = '<p>'; }</script>",
            "<pre>line 1\n  line 2</pre><img alt=\"logo\" src=\"l.png\"/><br/>",
            "<a href=\"https://other.org/x/y?q=1\">out</a> <a href=\"../up/./z#k\">rel</a></main></body></html>"
        );
        let (title, text) = convert(page, "https://example.com/docs/page");
        assert_eq!(title, "T & Co");
        assert!(!text.contains("SECRET") && !text.contains("x = "), "{text}");
        for want in [
            "# Hi",
            "Visible [link](/a) \u{2014} caf\u{e9} \u{20ac}5",
            "- one",
            "- two",
            "a | b",
            "small but shown",
            "```\nline 1\n  line 2\n```",
            "[img: logo]",
            "[out](other.org/x/y) [rel](/up/z)",
        ] {
            assert!(text.contains(want), "missing {want:?} in\n{text}");
        }
        assert_eq!(short_link("https://a.de/x/", "mailto:me@a.de"), "me@a.de");
        assert_eq!(dot_segments("/a/b/../../../c/."), "/c/");
    }
}
