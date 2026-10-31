mod engines;
mod rank;

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use wisply_core::{Call, Fail, local_now, out_file};

use super::Daemon;
use super::ctx::Ctx;
use super::memory::index::vault;
use super::web;
use engines::{ENGINES, Kind, Outcome};
use rank::{Candidate, Passage, Pool};

const EXPAND_WEIGHT: f64 = 0.4;
const PAGE_WEIGHT: f64 = 0.5;
const DOMAINS: &str = "research/domains.txt";
const VAULT_DIR: &str = "Research";
const REPORT_MARK: &str = "<!-- report -->";
const PASSAGES_PER_PAGE: usize = 3;
const PASSAGE_CHARS: usize = 320;
const REPORT_PAGES: usize = 8;
const AGREED_MAX: usize = 8;
const AGREED_FROM: usize = 40;
const EXPAND_TERMS: usize = 3;
const EXPAND_FROM: usize = 10;
const CITED_MAX: usize = 3;
const SEARCH_WAIT: Duration = Duration::from_secs(6);
const PAGE_WAIT: Duration = Duration::from_secs(8);
const SLUG_MAX: usize = 60;
const GOOGLE: &str = "https://www.google.com/search?hl=en&q=";
const GOOGLE_JS: &str =
    "[...document.querySelectorAll('a h3')].map(h => h.closest('a').href + '\\t' + h.textContent).join('\\n')";

struct Level {
    rounds: usize,
    first: usize,
    later: usize,
    budget: Duration,
}

const QUICK: Level = Level { rounds: 1, first: 6, later: 0, budget: Duration::from_secs(12) };
const NORMAL: Level = Level { rounds: 2, first: 8, later: 5, budget: Duration::from_secs(25) };
const DEEP: Level = Level { rounds: 3, first: 10, later: 6, budget: Duration::from_secs(45) };

struct Page {
    url: String,
    title: String,
    text: String,
    date: Option<String>,
    links: Vec<String>,
    score: f64,
    engines: usize,
}

struct Found {
    asked: usize,
    queries: Vec<String>,
    pages: Vec<Page>,
    used: BTreeSet<String>,
    notes: Vec<String>,
    rounds: usize,
}

pub fn run(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let queries: Vec<String> =
        c.pos.iter().map(|q| q.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|q| !q.is_empty()).collect();
    if queries.is_empty() {
        return Err(Fail::hint(
            "research needs a question",
            "wisply research \"folia scheduler\" \"folia scheduler library\"",
        ));
    }
    let level = if c.on("quick") {
        &QUICK
    } else if c.on("deep") {
        &DEEP
    } else {
        &NORMAL
    };
    let site = c.val("site").filter(|s| !s.is_empty());
    let now = local_now();
    let file = vault(&d.home).join(VAULT_DIR).join(format!("{} {}.md", &now[..10], slug(&queries, site)));
    if !c.on("fresh")
        && let Ok(saved) = fs::read_to_string(&file)
        && let Some((_, report)) = saved.split_once(REPORT_MARK)
    {
        return Ok(format!("{}\n(saved earlier today; --fresh searches again)", report.trim()));
    }
    let started = Instant::now();
    let found = gather(d, ctx, &queries, site, level, started + level.budget);
    if found.pages.is_empty() {
        let why = if found.notes.is_empty() { String::new() } else { format!(" ({})", found.notes.join("; ")) };
        return Err(Fail::hint(
            format!("research found no readable pages{why}"),
            "other words, or wisply search google \"words\" in the browser",
        ));
    }
    let year = now[..4].parse().unwrap_or(2026);
    let body = report(&found, year, started.elapsed());
    let texts = save_texts(&d.home, &found.pages);
    let stored = save(&file, &queries, &now, &body);
    let mut out = body;
    out.push_str(&format!("\nnext: wisply read <url> for a whole page · full texts: {}", texts.display()));
    if stored {
        out.push_str(&format!("\nsaved: vault {VAULT_DIR}/{}", file.file_name().unwrap_or_default().to_string_lossy()));
    }
    Ok(out)
}

fn gather(
    d: &Daemon,
    ctx: &mut Ctx,
    queries: &[String],
    site: Option<&str>,
    level: &Level,
    deadline: Instant,
) -> Found {
    let base = std::env::var(engines::BASE_VAR).ok().filter(|b| !b.is_empty());
    let extra = domains(&d.home);
    let year = local_now()[..4].parse().unwrap_or(2026);
    let mut found = Found {
        asked: queries.len(),
        queries: queries.to_vec(),
        pages: Vec::new(),
        used: BTreeSet::new(),
        notes: Vec::new(),
        rounds: 0,
    };
    let mut pool = Pool::default();
    let mut tried: HashSet<String> = HashSet::new();
    let mut current = queries.to_vec();
    for round in 0..level.rounds {
        if Instant::now() >= deadline || d.abort.load(Ordering::Relaxed) {
            break;
        }
        search(d, ctx, &current, site, base.as_deref(), &mut pool, &mut found);
        let n = if round == 0 { level.first } else { level.later };
        let mut todo: Vec<Candidate> = pool.best(&extra, &tried, n);
        if round > 0 {
            let links: Vec<(String, Vec<String>)> =
                found.pages.iter().map(|p| (p.url.clone(), p.links.clone())).collect();
            for url in rank::cited(&links, &tried).into_iter().take(CITED_MAX) {
                todo.push(Candidate { url, title: String::new(), score: 0.0, engines: HashSet::new() });
            }
        }
        tried.extend(todo.iter().map(|c| rank::normalize(&c.url)));
        found.pages.extend(fetch(&todo, deadline));
        found.rounds = round + 1;
        if round + 1 < level.rounds {
            let (all, scores) = ranked(&found.pages, queries, &[], year);
            let mut order: Vec<usize> = (0..all.len()).collect();
            order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
            let top: Vec<&Passage> =
                order.iter().take(EXPAND_FROM).filter(|i| scores[**i] > 0.0).map(|i| &all[*i]).collect();
            let terms = rank::expand(&top, &all, &query_words(queries), EXPAND_TERMS);
            if terms.is_empty() {
                current = Vec::new();
            } else {
                current = queries.iter().map(|q| format!("{q} {}", terms.join(" "))).collect();
                found.queries.extend(current.iter().cloned());
            }
        }
    }
    found
}

fn search(
    d: &Daemon,
    ctx: &mut Ctx,
    queries: &[String],
    site: Option<&str>,
    base: Option<&str>,
    pool: &mut Pool,
    found: &mut Found,
) {
    let (tx, rx) = mpsc::channel();
    let mut asked = 0;
    for e in ENGINES.iter().filter(|e| !engines::blocked(&d.home, e.name)) {
        for q in queries {
            let Some(q) = engines::site_query(e, q, site) else { continue };
            let url = engines::address(e, &q, base);
            let tx = tx.clone();
            asked += 1;
            let _ = thread::Builder::new().name("research".into()).spawn(move || {
                let _ = tx.send((e, engines::ask(e, &url)));
            });
        }
    }
    drop(tx);
    let until = Instant::now() + SEARCH_WAIT;
    let mut pages_answered = false;
    for _ in 0..asked {
        let Ok((e, outcome)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) else { break };
        match outcome {
            Outcome::Hits(hits) if !hits.is_empty() => {
                engines::mark(&d.home, e.name, false);
                pages_answered |= e.kind == Kind::Page;
                found.used.insert(e.name.to_string());
                let pairs: Vec<(String, String)> = hits.into_iter().map(|h| (h.url, h.title)).collect();
                pool.add(e.name, &pairs);
            }
            Outcome::Hits(_) => {}
            Outcome::Blocked(why) => {
                engines::mark(&d.home, e.name, true);
                let said = format!("{} blocked", e.name);
                if found.notes.iter().any(|n| n.starts_with(&said)) {
                    continue;
                }
                found.notes.push(format!("{} blocked ({}), skipped for 30 min", e.name, web::head(&why, 40)));
            }
            Outcome::Failed => {}
        }
    }
    if !pages_answered && base.is_none() && web::any_browser(d) {
        browser_search(d, ctx, queries, site, pool, found);
    }
}

fn browser_search(
    d: &Daemon,
    ctx: &mut Ctx,
    queries: &[String],
    site: Option<&str>,
    pool: &mut Pool,
    found: &mut Found,
) {
    let Some(q) = queries.first() else { return };
    let q = site.map_or_else(|| q.clone(), |s| format!("{q} site:{s}"));
    let Ok(text) = web::browser_js(d, ctx, &format!("{GOOGLE}{}", engines::quote(&q)), GOOGLE_JS) else { return };
    let pairs: Vec<(String, String)> = text
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter(|(u, t)| u.starts_with("http") && !engines::host_of(u).contains("google.") && !t.trim().is_empty())
        .map(|(u, t)| (u.to_string(), t.trim().to_string()))
        .collect();
    if !pairs.is_empty() {
        pool.add("google", &pairs);
        found.used.insert("google (your browser)".into());
        found.notes.push("every search page was blocked, so Google ran in your browser".into());
    }
}

fn fetch(todo: &[Candidate], deadline: Instant) -> Vec<Page> {
    let (tx, rx) = mpsc::channel();
    for (i, c) in todo.iter().enumerate() {
        let tx = tx.clone();
        let url = c.url.clone();
        let _ = thread::Builder::new().name("research-read".into()).spawn(move || {
            let _ = tx.send((i, web::fetch_page(&url)));
        });
    }
    drop(tx);
    let until = deadline.min(Instant::now() + PAGE_WAIT);
    let mut out = Vec::new();
    for _ in 0..todo.len() {
        let Ok((i, got)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) else { break };
        let Ok(f) = got else { continue };
        if f.text.trim().chars().count() < 200 {
            continue;
        }
        let c = &todo[i];
        let title = if f.title.trim().is_empty() { c.title.clone() } else { f.title.trim().to_string() };
        out.push(Page {
            date: rank::date(&f.place, &f.head),
            url: c.url.clone(),
            title,
            text: f.text,
            links: f.links,
            score: c.score.max(0.001),
            engines: c.engines.len(),
        });
    }
    out
}

fn query_words(queries: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    queries.iter().flat_map(|q| rank::words(q)).filter(|w| seen.insert(w.clone())).collect()
}

fn ranked(pages: &[Page], queries: &[String], extra: &[String], year: i32) -> (Vec<Passage>, Vec<f64>) {
    let all: Vec<Passage> = pages.iter().enumerate().flat_map(|(i, p)| rank::passages(i, &p.text)).collect();
    let words = query_words(queries);
    let more: Vec<String> = query_words(extra).into_iter().filter(|w| !words.contains(w)).collect();
    let bonus = rank::bm25(&all, &more);
    let phrases: Vec<String> = queries.iter().map(|q| q.to_lowercase()).collect();
    let bm = rank::bm25(&all, &words);
    let technical = rank::technical(queries);
    let fresh = rank::wants_fresh(queries);
    let top = pages.iter().map(|p| p.score).fold(0.001, f64::max);
    let scores = all
        .iter()
        .zip(bm.into_iter().zip(bonus))
        .map(|(p, (s, b))| {
            let page = &pages[p.page];
            let age = if fresh { rank::freshness(page.date.as_deref(), year) } else { 1.0 };
            let trust = 1.0 - PAGE_WEIGHT + PAGE_WEIGHT * page.score / top;
            (s + EXPAND_WEIGHT * b) * rank::boost(p, &phrases, &words, technical) * trust * age
        })
        .collect();
    (all, scores)
}

fn one_line(text: &str, n: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= n {
        flat
    } else {
        format!("{} …", flat.chars().take(n).collect::<String>().trim_end())
    }
}

fn report(found: &Found, year: i32, took: Duration) -> String {
    let (all, scores) = ranked(&found.pages, &found.queries[..found.asked], &found.queries[found.asked..], year);
    let mut best: Vec<(usize, f64)> = (0..found.pages.len())
        .map(|i| {
            let top = all.iter().zip(&scores).filter(|(p, _)| p.page == i).map(|(_, s)| *s).fold(0.0, f64::max);
            (i, top)
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();
    best.sort_by(|a, b| b.1.total_cmp(&a.1));
    let asked: Vec<String> = found.queries.iter().take(found.asked).map(|q| format!("\"{q}\"")).collect();
    let mut out = vec![format!(
        "research {} ({} round{}, {} pages, {:.1} s, {})",
        asked.join(" + "),
        found.rounds,
        if found.rounds == 1 { "" } else { "s" },
        found.pages.len(),
        took.as_secs_f64(),
        found.used.iter().cloned().collect::<Vec<_>>().join(", ")
    )];
    out.extend(found.notes.iter().map(|n| format!("! {n}")));
    let mut agreed_from: Vec<(String, &str)> = Vec::new();
    for (n, (i, _)) in best.iter().take(REPORT_PAGES).enumerate() {
        let page = &found.pages[*i];
        let mut meta = vec![engines::host_of(&page.url)];
        meta.extend(page.date.clone());
        if page.engines > 1 {
            meta.push(format!("{} engines", page.engines));
        }
        out.push(format!("[{}] {} · {} · {}", n + 1, one_line(&page.title, 80), page.url, meta.join(" · ")));
        let mut own: Vec<(usize, f64)> = all
            .iter()
            .enumerate()
            .filter(|(_, p)| p.page == *i)
            .map(|(k, _)| (k, scores[k]))
            .filter(|(_, s)| *s > 0.0)
            .collect();
        own.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (k, _) in own.iter().take(PASSAGES_PER_PAGE) {
            out.push(format!("    > {}", one_line(&all[*k].text, PASSAGE_CHARS)));
        }
    }
    let mut order: Vec<usize> = (0..all.len()).filter(|k| scores[*k] > 0.0).collect();
    order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
    for k in order.into_iter().take(AGREED_FROM) {
        agreed_from.push((engines::host_of(&found.pages[all[k].page].url), all[k].text.as_str()));
    }
    let agreed = rank::agreed(&agreed_from, &query_words(&found.queries[..found.asked]), AGREED_MAX);
    if !agreed.is_empty() {
        let list: Vec<String> = agreed.iter().map(|(name, n)| format!("{name} ({n})")).collect();
        out.push(format!("said by several sources: {}", list.join(", ")));
    }
    out.join("\n")
}

fn slug(queries: &[String], site: Option<&str>) -> String {
    let mut words: Vec<String> =
        queries.iter().flat_map(|q| rank::words(q)).collect::<BTreeSet<_>>().into_iter().collect();
    words.extend(site.map(|s| format!("site-{}", engines::host_of(s))));
    let raw: String = words.join("-").chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' }).collect();
    let cut: String = raw.chars().take(SLUG_MAX).collect();
    let cut = cut.trim_matches('-').to_string();
    if cut.is_empty() { "research".into() } else { cut }
}

fn save(file: &Path, queries: &[String], now: &str, body: &str) -> bool {
    let Some(dir) = file.parent() else { return false };
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let list: Vec<String> = queries.iter().map(|q| format!("  - \"{}\"", q.replace('"', "'"))).collect();
    let text = format!(
        "---\ntype: research\ndate: {now}\nqueries:\n{}\n---\n\n# Research: {}\n\n{REPORT_MARK}\n{body}\n",
        list.join("\n"),
        queries.first().map_or("", String::as_str)
    );
    fs::write(file, text).is_ok()
}

fn save_texts(home: &Path, pages: &[Page]) -> PathBuf {
    let path = out_file(&home.join("out"), "research", "md");
    let text: Vec<String> = pages.iter().map(|p| format!("# {}\n{}\n\n{}\n", p.title, p.url, p.text)).collect();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(&path, text.join("\n\n"));
    path
}

fn domains(home: &Path) -> Vec<(String, f64)> {
    let Ok(text) = fs::read_to_string(home.join(DOMAINS)) else { return Vec::new() };
    text.lines()
        .map(str::trim)
        .filter_map(|l| {
            let (sign, host) = l.split_at(l.find(|c: char| c != '+' && c != '-')?);
            let v = match sign {
                "+" => 0.3,
                "++" => 0.6,
                "-" => -0.5,
                "--" => -1.0,
                _ => return None,
            };
            Some((engines::host_of(host.trim()), v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_domain_lists() {
        assert_eq!(
            slug(&["Folia Scheduler".into(), "folia scheduler library".into()], None),
            "folia-library-scheduler"
        );
        assert_eq!(slug(&["a".into()], Some("https://github.com")), "site-github-com");
        let home = std::env::temp_dir().join(format!("wisply-research-{}", std::process::id()));
        fs::create_dir_all(home.join("research")).unwrap();
        fs::write(home.join(DOMAINS), "+docs.example.com\n--spam.example\nnot a rule\n").unwrap();
        assert_eq!(domains(&home), [("docs.example.com".to_string(), 0.3), ("spam.example".to_string(), -1.0)]);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn lines_are_cut() {
        assert_eq!(one_line("a\n  b", 10), "a b");
        assert_eq!(one_line("abcdef", 3), "abc …");
    }
}
