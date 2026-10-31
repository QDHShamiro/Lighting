use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use wisply_memory::stop_word;

use super::engines::host_of;

const RRF_K: f64 = 60.0;
const PER_DOMAIN: usize = 2;
const PASSAGE_MIN: usize = 400;
const PASSAGE_MAX: usize = 900;
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const TRUSTED: &[&str] = &[
    "wikipedia.org",
    "github.com",
    "stackoverflow.com",
    "stackexchange.com",
    "readthedocs.io",
    "mozilla.org",
    "rust-lang.org",
    "python.org",
    "papermc.io",
    "docs.rs",
    "arxiv.org",
    "news.ycombinator.com",
];
const WEAK: &[&str] = &["pinterest.com", "quora.com", "facebook.com", "instagram.com", "tiktok.com", "linkedin.com"];
const TRACKING: &[&str] = &["utm_", "fbclid", "gclid", "ref", "ref_src", "si", "mc_cid", "mc_eid"];

static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}][\p{L}\p{N}._+#-]*[\p{L}\p{N}+#]|[\p{L}\p{N}]").unwrap());
static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d+(?:\.\d+){1,3}\b").unwrap());
static AMOUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d+(?:[.,]\d+)?\s?(?:ms|s|MB|GB|KB|TB|%|fps|tps|GHz|MHz|k|M)\b").unwrap());
static CAMEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Z][a-z0-9]+(?:[A-Z][a-z0-9]+)+\b").unwrap());
static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"`([^`\n]{3,40})`|"([^"\n]{3,40})""#).unwrap());
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(19|20)\d{2}\b").unwrap());
static DATES: LazyLock<[Regex; 4]> = LazyLock::new(|| {
    [
        Regex::new(r#"(?i)(?:article:published_time|datePublished|dateModified|article:modified_time)["']?\s*(?:content=|:)\s*["'](\d{4}-\d{2}-\d{2})"#).unwrap(),
        Regex::new(r#"(?i)<time[^>]*datetime=["'](\d{4}-\d{2}-\d{2})"#).unwrap(),
        Regex::new(r#"(?i)"(?:created_at|updated_at|pushed_at)"\s*:\s*"(\d{4}-\d{2}-\d{2})"#).unwrap(),
        Regex::new(r"/((?:19|20)\d{2})/(\d{2})/").unwrap(),
    ]
});

pub fn words(text: &str) -> Vec<String> {
    WORD.find_iter(&text.to_lowercase()).map(|m| m.as_str().to_string()).filter(|w| !stop_word(w)).collect()
}

pub fn normalize(url: &str) -> String {
    let (no_frag, _) = url.split_once('#').unwrap_or((url, ""));
    let (base, query) = no_frag.split_once('?').unwrap_or((no_frag, ""));
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .filter(|kv| {
            let key = kv.split('=').next().unwrap_or("").to_lowercase();
            !TRACKING.iter().any(|t| if t.ends_with('_') { key.starts_with(t) } else { key == *t })
        })
        .collect();
    let path = path.trim_end_matches('/');
    let mut out = format!("https://{host}");
    if !path.is_empty() {
        out.push('/');
        out.push_str(path);
    }
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    out
}

pub fn prior(host: &str, extra: &[(String, f64)]) -> f64 {
    let on = |list: &[&str]| list.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")));
    if let Some((_, v)) = extra.iter().find(|(d, _)| host == d || host.ends_with(&format!(".{d}"))) {
        return *v;
    }
    if on(TRUSTED) || host.starts_with("docs.") || host.starts_with("developer.") {
        0.3
    } else if on(WEAK) {
        -0.5
    } else {
        0.0
    }
}

#[derive(Default, Clone)]
pub struct Candidate {
    pub url: String,
    pub title: String,
    pub score: f64,
    pub engines: HashSet<String>,
}

#[derive(Default)]
pub struct Pool {
    pub by_url: HashMap<String, Candidate>,
}

impl Pool {
    pub fn add(&mut self, engine: &str, hits: &[(String, String)]) {
        for (rank, (url, title)) in hits.iter().enumerate() {
            let key = normalize(url);
            let c = self.by_url.entry(key).or_insert_with(|| Candidate {
                url: url.clone(),
                title: title.clone(),
                ..Candidate::default()
            });
            c.score += 1.0 / (RRF_K + rank as f64 + 1.0);
            c.engines.insert(engine.to_string());
        }
    }

    pub fn best(&self, extra: &[(String, f64)], skip: &HashSet<String>, n: usize) -> Vec<Candidate> {
        let mut all: Vec<Candidate> = self
            .by_url
            .iter()
            .filter(|(k, _)| !skip.contains(*k))
            .map(|(_, c)| {
                let mut c = c.clone();
                c.score *= 1.0 + prior(&host_of(&c.url), extra);
                c
            })
            .collect();
        all.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.url.cmp(&b.url)));
        let mut per: HashMap<String, usize> = HashMap::new();
        all.into_iter()
            .filter(|c| {
                let n = per.entry(host_of(&c.url)).or_default();
                *n += 1;
                *n <= PER_DOMAIN
            })
            .take(n)
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct Passage {
    pub page: usize,
    pub heading: String,
    pub text: String,
}

pub fn passages(page: usize, text: &str) -> Vec<Passage> {
    let mut out = Vec::new();
    let mut heading = String::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, heading: &str, out: &mut Vec<Passage>| {
        let t = buf.trim();
        if t.chars().count() >= 40 {
            out.push(Passage { page, heading: heading.to_string(), text: t.to_string() });
        }
        buf.clear();
    };
    for block in text.split("\n\n").map(str::trim).filter(|b| !b.is_empty()) {
        if let Some(h) = block.strip_prefix('#') {
            flush(&mut buf, &heading, &mut out);
            heading = h.trim_start_matches('#').trim().to_string();
            continue;
        }
        if !buf.is_empty() && buf.chars().count() + block.chars().count() > PASSAGE_MAX {
            flush(&mut buf, &heading, &mut out);
        }
        if block.chars().count() > PASSAGE_MAX {
            for piece in split_long(block) {
                buf.push_str(&piece);
                flush(&mut buf, &heading, &mut out);
            }
            continue;
        }
        if !buf.is_empty() {
            buf.push_str("\n\n");
        }
        buf.push_str(block);
        if buf.chars().count() >= PASSAGE_MIN {
            flush(&mut buf, &heading, &mut out);
        }
    }
    flush(&mut buf, &heading, &mut out);
    out
}

fn split_long(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for sentence in block.split_inclusive(['.', '!', '?', '\n']) {
        if cur.chars().count() + sentence.chars().count() > PASSAGE_MAX && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push_str(sentence);
        while cur.chars().count() > PASSAGE_MAX {
            let cut = cur.char_indices().nth(PASSAGE_MAX).map_or(cur.len(), |(i, _)| i);
            out.push(cur[..cut].to_string());
            cur = cur[cut..].to_string();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

pub fn bm25(passages: &[Passage], query: &[String]) -> Vec<f64> {
    let docs: Vec<Vec<String>> = passages.iter().map(|p| words(&format!("{} {}", p.heading, p.text))).collect();
    let n = docs.len().max(1) as f64;
    let avg = docs.iter().map(Vec::len).sum::<usize>() as f64 / n;
    let terms: HashSet<&String> = query.iter().collect();
    let df: HashMap<&String, f64> =
        terms.iter().map(|t| (*t, docs.iter().filter(|d| d.contains(t)).count() as f64)).collect();
    docs.iter()
        .map(|d| {
            let len = d.len() as f64;
            terms
                .iter()
                .map(|t| {
                    let tf = d.iter().filter(|w| w == t).count() as f64;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let idf = ((n - df[t] + 0.5) / (df[t] + 0.5) + 1.0).ln();
                    idf * tf * (BM25_K1 + 1.0) / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * len / avg.max(1.0)))
                })
                .sum()
        })
        .collect()
}

pub fn boost(p: &Passage, phrases: &[String], query: &[String], technical: bool) -> f64 {
    let lower = p.text.to_lowercase();
    let mut f = 1.0;
    if phrases.iter().any(|ph| ph.contains(' ') && lower.contains(ph.as_str())) {
        f += 0.5;
    }
    let head = p.heading.to_lowercase();
    if query.iter().any(|q| head.contains(q.as_str())) {
        f += 0.3;
    }
    if technical && (p.text.contains("```") || p.text.contains('`')) {
        f += 0.2;
    }
    f
}

pub fn technical(queries: &[String]) -> bool {
    queries.iter().any(|q| {
        VERSION.is_match(q)
            || q.split_whitespace()
                .any(|w| w.contains(['.', '_', ':', '(']) || (w.chars().any(char::is_uppercase) && w.len() > 2))
    })
}

pub fn expand(top: &[&Passage], all: &[Passage], query: &[String], n: usize) -> Vec<String> {
    let total = all.len().max(1) as f64;
    let mut df: HashMap<String, f64> = HashMap::new();
    for p in all {
        for w in words(&p.text).into_iter().collect::<HashSet<_>>() {
            *df.entry(w).or_default() += 1.0;
        }
    }
    let mut tf: HashMap<String, f64> = HashMap::new();
    for p in top {
        for w in words(&p.text) {
            *tf.entry(w).or_default() += 1.0;
        }
    }
    let mut scored: Vec<(String, f64)> = tf
        .into_iter()
        .filter(|(w, c)| {
            *c >= 2.0 && w.chars().count() >= 3 && !query.contains(w) && !w.chars().all(|c| c.is_ascii_digit())
        })
        .map(|(w, c)| {
            let idf = (total / df.get(&w).copied().unwrap_or(1.0)).ln() + 1.0;
            (w, c * idf)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored.into_iter().take(n).map(|(w, _)| w).collect()
}

pub fn cited(pages: &[(String, Vec<String>)], read: &HashSet<String>) -> Vec<String> {
    let mut count: BTreeMap<String, (String, HashSet<String>)> = BTreeMap::new();
    for (from, links) in pages {
        let own = host_of(from);
        for link in links {
            let key = normalize(link);
            if host_of(link) == own || read.contains(&key) {
                continue;
            }
            count.entry(key).or_insert_with(|| (link.clone(), HashSet::new())).1.insert(own.clone());
        }
    }
    let mut out: Vec<(usize, String)> =
        count.into_values().filter(|(_, by)| by.len() >= 2).map(|(url, by)| (by.len(), url)).collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, url)| url).collect()
}

pub fn agreed(items: &[(String, &str)], query: &[String], n: usize) -> Vec<(String, usize)> {
    let mut seen: HashMap<String, (String, HashSet<String>)> = HashMap::new();
    for (host, text) in items {
        let mut found: Vec<String> = Vec::new();
        found.extend(VERSION.find_iter(text).map(|m| m.as_str().to_string()));
        found.extend(AMOUNT.find_iter(text).map(|m| m.as_str().to_string()));
        found.extend(CAMEL.find_iter(text).map(|m| m.as_str().to_string()));
        found.extend(QUOTED.captures_iter(text).filter_map(|c| c.get(1).or(c.get(2))).map(|m| m.as_str().to_string()));
        for f in found {
            let key = f.to_lowercase();
            if query.contains(&key) {
                continue;
            }
            seen.entry(key).or_insert_with(|| (f.clone(), HashSet::new())).1.insert(host.clone());
        }
    }
    let mut out: Vec<(String, usize)> =
        seen.into_values().filter(|(_, by)| by.len() >= 2).map(|(name, by)| (name, by.len())).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.truncate(n);
    out
}

pub fn date(url: &str, head: &str) -> Option<String> {
    for re in DATES.iter().take(3) {
        if let Some(c) = re.captures(head) {
            return Some(c[1][..7].to_string());
        }
    }
    DATES[3].captures(url).map(|c| format!("{}-{}", &c[1], &c[2]))
}

pub fn wants_fresh(queries: &[String]) -> bool {
    queries.iter().any(|q| YEAR.is_match(q) || VERSION.is_match(q) || q.to_lowercase().contains("latest"))
}

pub fn freshness(date: Option<&str>, this_year: i32) -> f64 {
    let year = date.and_then(|d| d.get(..4)).and_then(|y| y.parse::<i32>().ok());
    match year {
        Some(y) if y >= this_year - 1 => 1.2,
        Some(y) if y >= this_year - 3 => 1.05,
        Some(_) => 0.9,
        None => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalized() {
        assert_eq!(normalize("http://www.GitHub.com/a/b/?utm_source=x&tab=1#top"), "https://github.com/a/b?tab=1");
        assert_eq!(normalize("https://example.com/"), "https://example.com");
    }

    #[test]
    fn pool_merges_engines_and_caps_domains() {
        let mut pool = Pool::default();
        let a = |u: &str| (u.to_string(), "t".to_string());
        pool.add("bing", &[a("https://x.com/1"), a("https://x.com/2"), a("https://x.com/3"), a("https://y.org")]);
        pool.add("ddg", &[a("https://y.org/"), a("https://x.com/1")]);
        let best = pool.best(&[], &HashSet::new(), 10);
        let urls: Vec<&str> = best.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(urls.len(), 3);
        assert_eq!(best[0].engines.len(), 2);
        assert!(urls.contains(&"https://y.org"));
        assert_eq!(prior("en.wikipedia.org", &[]), 0.3);
        assert_eq!(prior("www.pinterest.com".trim_start_matches("www."), &[]), -0.5);
        assert_eq!(prior("a.dev", &[("a.dev".into(), -1.0)]), -1.0);
    }

    #[test]
    fn passages_are_split_and_ranked() {
        let text = format!(
            "# Intro\n\nShort.\n\n## Scheduler\n\nFolia has a region scheduler for every region. {}\n\nOther text about cats and dogs that is long enough to count as a passage on its own here.",
            "Use it for blocks. ".repeat(30)
        );
        let ps = passages(0, &text);
        assert!(ps.iter().all(|p| p.text.chars().count() <= PASSAGE_MAX));
        assert_eq!(ps[0].heading, "Scheduler");
        let q = words("folia region scheduler");
        let scores = bm25(&ps, &q);
        let best = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert!(ps[best].text.contains("region scheduler"));
        assert!(boost(&ps[best], &["region scheduler".into()], &q, false) > 1.5);
    }

    #[test]
    fn feedback_terms_links_and_agreement() {
        let all = vec![
            Passage { page: 0, heading: String::new(), text: "FoliaLib wraps schedulers. FoliaLib is small.".into() },
            Passage { page: 1, heading: String::new(), text: "Try FoliaLib or MorePaperLib.".into() },
            Passage { page: 2, heading: String::new(), text: "Unrelated cooking text with butter.".into() },
        ];
        let top: Vec<&Passage> = all.iter().take(2).collect();
        assert_eq!(expand(&top, &all, &words("folia scheduler"), 1), ["folialib"]);
        let pages = vec![
            ("https://a.com/x".to_string(), vec!["https://lib.dev/".to_string(), "https://a.com/self".to_string()]),
            ("https://b.com/y".to_string(), vec!["https://lib.dev".to_string()]),
        ];
        assert_eq!(cited(&pages, &HashSet::new()), ["https://lib.dev/"]);
        let items = vec![
            ("a.com".to_string(), "Use FoliaLib 0.4.3 on 1.21.11"),
            ("b.com".to_string(), "FoliaLib works on 1.21.11, 50 ms"),
            ("b.com".to_string(), "MorePaperLib"),
        ];
        let got = agreed(&items, &[], 5);
        assert_eq!(got, [("1.21.11".to_string(), 2), ("FoliaLib".to_string(), 2)]);
    }

    #[test]
    fn dates_and_freshness() {
        assert_eq!(
            date("https://a.com/x", r#"<meta property="article:published_time" content="2025-03-04T10:00">"#)
                .as_deref(),
            Some("2025-03")
        );
        assert_eq!(date("https://a.com/2024/07/post", "").as_deref(), Some("2024-07"));
        assert!(wants_fresh(&["paper 1.21.11".into()]));
        assert!(freshness(Some("2026-01"), 2026) > freshness(Some("2019-01"), 2026));
        assert!(technical(&["Bukkit.getScheduler()".into()]));
    }
}
