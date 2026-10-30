use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use lt_core::Fail;
use regex::Regex;
use serde_json::Value;
use ureq::tls::{RootCerts, TlsConfig};

use super::super::media::tools;
use super::ledger;

const GH_WAIT: Duration = Duration::from_secs(30);
const API_WAIT: Duration = Duration::from_secs(20);
const API: &str = "https://api.github.com/repos/";
const LIST: usize = 20;
const RELEASES: usize = 10;
const BODY_CHARS: usize = 3000;
const COMMENT_CHARS: usize = 500;
const COMMENTS: usize = 10;
const KEY: &str = "github";

static PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^https?://(?:www\.)?github\.com/([\w.-]+)/([\w.-]+)/(issues|pulls|pull|releases)(?:/(?:tag/)?([^/?#]+))?/?(?:[?#].*)?$",
    )
    .expect("github page")
});
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(API_WAIT))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
});

#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    Issues,
    Issue(String),
    Pulls,
    Pull(String),
    Releases,
    Release(String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Page {
    pub repo: String,
    pub kind: Kind,
}

pub fn parse(url: &str) -> Option<Page> {
    let m = PAGE.captures(url.trim())?;
    let repo = format!("{}/{}", &m[1], &m[2]);
    let item = m.get(4).map(|x| x.as_str().to_string()).filter(|x| !x.is_empty());
    let number = item.clone().filter(|x| x.chars().all(|c| c.is_ascii_digit()));
    let kind = match (m[3].to_lowercase().as_str(), item) {
        ("issues", None) => Kind::Issues,
        ("issues", Some(_)) => Kind::Issue(number?),
        ("pulls" | "pull", None) => Kind::Pulls,
        ("pulls" | "pull", Some(_)) => Kind::Pull(number?),
        ("releases", None) => Kind::Releases,
        (_, Some(tag)) => Kind::Release(tag),
        _ => return None,
    };
    Some(Page { repo, kind })
}

fn text(v: &Value, keys: &[&str]) -> String {
    keys.iter().find_map(|k| v.pointer(k).and_then(Value::as_str)).unwrap_or("").to_string()
}

fn day(v: &Value, keys: &[&str]) -> String {
    text(v, keys).chars().take(10).collect()
}

fn cut(s: &str, n: usize) -> String {
    let flat = s.replace("\r\n", "\n");
    let flat = flat.trim();
    if flat.chars().count() <= n {
        flat.to_string()
    } else {
        format!("{}...", flat.chars().take(n).collect::<String>())
    }
}

fn author(v: &Value) -> String {
    text(v, &["/author/login", "/user/login"])
}

fn comment_count(v: &Value) -> usize {
    match &v["comments"] {
        Value::Array(a) => a.len(),
        Value::Number(n) => usize::try_from(n.as_u64().unwrap_or(0)).unwrap_or(0),
        _ => 0,
    }
}

fn state(v: &Value) -> String {
    let s = text(v, &["/state"]).to_lowercase();
    let merged = !text(v, &["/mergedAt", "/merged_at"]).is_empty();
    let draft = v["isDraft"].as_bool().or_else(|| v["draft"].as_bool()) == Some(true);
    match (merged, draft) {
        (true, _) => "merged".into(),
        (_, true) => format!("{s}, draft"),
        _ => s,
    }
}

pub fn format(page: &Page, data: &Value, comments: &Value, how: &str) -> String {
    let repo = &page.repo;
    match &page.kind {
        Kind::Issues | Kind::Pulls => {
            let what = if page.kind == Kind::Issues { "issues" } else { "pull requests" };
            let rows: Vec<String> = data
                .as_array()
                .into_iter()
                .flatten()
                .filter(|i| page.kind != Kind::Issues || i.get("pull_request").is_none())
                .take(LIST)
                .map(|i| {
                    format!(
                        "#{} {} {} ({}, {} comments, {})",
                        i["number"],
                        state(i),
                        text(i, &["/title"]),
                        author(i),
                        comment_count(i),
                        day(i, &["/updatedAt", "/updated_at"])
                    )
                })
                .collect();
            let body = if rows.is_empty() { format!("no open {what}") } else { rows.join("\n") };
            format!("[github] {repo} open {what}, newest first ({how})\n{body}")
        }
        Kind::Issue(n) | Kind::Pull(n) => {
            let mut out = format!(
                "[github] {repo}#{n} {} ({}, by {}, {}; {how})",
                text(data, &["/title"]),
                state(data),
                author(data),
                day(data, &["/createdAt", "/created_at"])
            );
            if let Some(files) = data["changedFiles"].as_u64().or_else(|| data["changed_files"].as_u64()) {
                out.push_str(&format!("\n{files} files, +{} -{}", data["additions"], data["deletions"]));
            }
            let labels: Vec<String> =
                data["labels"].as_array().into_iter().flatten().map(|l| text(l, &["/name"])).collect();
            if !labels.is_empty() {
                out.push_str(&format!("\nlabels: {}", labels.join(", ")));
            }
            let body = cut(&text(data, &["/body"]), BODY_CHARS);
            out.push_str(&format!("\n\n{}", if body.is_empty() { "(no description)" } else { &body }));
            let all: Vec<&Value> = comments.as_array().into_iter().flatten().collect();
            if !all.is_empty() {
                out.push_str(&format!("\n\ncomments ({}, last {}):", all.len(), COMMENTS.min(all.len())));
                for c in all.iter().skip(all.len().saturating_sub(COMMENTS)) {
                    let when = day(c, &["/createdAt", "/created_at"]);
                    out.push_str(&format!("\n- {} ({when}): {}", author(c), cut(&text(c, &["/body"]), COMMENT_CHARS)));
                }
            }
            out
        }
        Kind::Releases => {
            let rows: Vec<String> = data
                .as_array()
                .into_iter()
                .flatten()
                .take(RELEASES)
                .map(|r| {
                    let mut flags = Vec::new();
                    if r["isLatest"].as_bool() == Some(true) {
                        flags.push("latest");
                    }
                    if r["isPrerelease"].as_bool().or_else(|| r["prerelease"].as_bool()) == Some(true) {
                        flags.push("pre-release");
                    }
                    let flags = if flags.is_empty() { String::new() } else { format!(" [{}]", flags.join(", ")) };
                    format!(
                        "{} {}{flags} ({})",
                        text(r, &["/tagName", "/tag_name"]),
                        text(r, &["/name"]),
                        day(r, &["/publishedAt", "/published_at"])
                    )
                })
                .collect();
            let body = if rows.is_empty() { "no releases".to_string() } else { rows.join("\n") };
            format!("[github] {repo} releases ({how})\n{body}")
        }
        Kind::Release(tag) => {
            let assets: Vec<String> =
                data["assets"].as_array().into_iter().flatten().map(|a| text(a, &["/name"])).collect();
            let mut out = format!(
                "[github] {repo} release {}: {} ({}; {how})",
                if tag == "latest" { text(data, &["/tagName", "/tag_name"]) } else { tag.clone() },
                text(data, &["/name"]),
                day(data, &["/publishedAt", "/published_at"])
            );
            if !assets.is_empty() {
                out.push_str(&format!("\nfiles: {}", assets.join(", ")));
            }
            let body = cut(&text(data, &["/body"]), BODY_CHARS);
            out.push_str(&format!("\n\n{}", if body.is_empty() { "(no notes)" } else { &body }));
            out
        }
    }
}

fn gh_args(page: &Page) -> Vec<String> {
    let r = page.repo.as_str();
    let list = LIST.to_string();
    let rel = RELEASES.to_string();
    let args: Vec<&str> = match &page.kind {
        Kind::Issues => {
            vec!["issue", "list", "-R", r, "-L", &list, "--json", "number,title,state,author,comments,updatedAt"]
        }
        Kind::Issue(n) => {
            vec![
                "issue",
                "view",
                n.as_str(),
                "-R",
                r,
                "--json",
                "number,title,state,author,body,comments,createdAt,labels",
            ]
        }
        Kind::Pulls => {
            vec!["pr", "list", "-R", r, "-L", &list, "--json", "number,title,state,author,comments,updatedAt,isDraft"]
        }
        Kind::Pull(n) => vec![
            "pr",
            "view",
            n.as_str(),
            "-R",
            r,
            "--json",
            "number,title,state,author,body,comments,createdAt,labels,additions,deletions,changedFiles,mergedAt,isDraft",
        ],
        Kind::Releases => {
            vec!["release", "list", "-R", r, "-L", &rel, "--json", "tagName,name,publishedAt,isLatest,isPrerelease"]
        }
        Kind::Release(tag) => {
            let mut v = vec!["release", "view"];
            if tag != "latest" {
                v.push(tag.as_str());
            }
            v.extend(["-R", r, "--json", "tagName,name,body,publishedAt,assets"]);
            v
        }
    };
    args.into_iter().map(String::from).collect()
}

fn by_gh(gh: &Path, page: &Page) -> Result<String, String> {
    let ran = tools::run(gh, &gh_args(page), GH_WAIT).map_err(|f| f.text())?;
    if !ran.ok {
        return Err(tools::last_line(&ran.err));
    }
    let data: Value = serde_json::from_str(&ran.out).map_err(|_| "gh sent no JSON".to_string())?;
    let comments = data["comments"].clone();
    Ok(format(page, &data, &comments, "gh"))
}

fn get(url: &str) -> Result<Value, String> {
    let mut resp = AGENT
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("lighting/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().with_config().limit(8_000_000).read_to_string().unwrap_or_default();
    if status != 200 {
        let why = serde_json::from_str::<Value>(&body).ok().and_then(|v| v["message"].as_str().map(String::from));
        return Err(format!("api.github.com answered {status}{}", why.map(|w| format!(" ({w})")).unwrap_or_default()));
    }
    serde_json::from_str(&body).map_err(|_| "api.github.com sent no JSON".into())
}

fn by_api(page: &Page) -> Result<String, String> {
    let base = format!("{API}{}", page.repo);
    let (data, comments) = match &page.kind {
        Kind::Issues => (get(&format!("{base}/issues?per_page={}", LIST * 2))?, Value::Null),
        Kind::Pulls => (get(&format!("{base}/pulls?per_page={LIST}"))?, Value::Null),
        Kind::Issue(n) => {
            (get(&format!("{base}/issues/{n}"))?, get(&format!("{base}/issues/{n}/comments?per_page=100"))?)
        }
        Kind::Pull(n) => {
            (get(&format!("{base}/pulls/{n}"))?, get(&format!("{base}/issues/{n}/comments?per_page=100"))?)
        }
        Kind::Releases => (get(&format!("{base}/releases?per_page={RELEASES}"))?, Value::Null),
        Kind::Release(tag) if tag == "latest" => (get(&format!("{base}/releases/latest"))?, Value::Null),
        Kind::Release(tag) => (get(&format!("{base}/releases/tags/{tag}"))?, Value::Null),
    };
    Ok(format(page, &data, &comments, "api.github.com"))
}

pub fn read(home: &Path, page: &Page) -> Result<String, Fail> {
    let mut why = Vec::new();
    if let Some(gh) = tools::which("gh") {
        match by_gh(&gh, page) {
            Ok(text) => {
                ledger::note(home, KEY, "gh", true, "");
                return Ok(text);
            }
            Err(e) => {
                ledger::note(home, KEY, "gh", false, &e);
                why.push(format!("gh: {e}"));
            }
        }
    }
    match by_api(page) {
        Ok(text) => {
            ledger::note(home, KEY, "api.github.com", true, "");
            let note = if why.is_empty() { String::new() } else { format!("\n! {}", why.join("; ")) };
            Ok(format!("{text}{note}"))
        }
        Err(e) => {
            ledger::note(home, KEY, "api.github.com", false, &e);
            why.push(e);
            Err(Fail::new(why.join("; ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn github_pages_become_short_lists() {
        let p = |u: &str| parse(u).map(|p| (p.repo, p.kind));
        assert_eq!(p("https://github.com/a/b/issues"), Some(("a/b".into(), Kind::Issues)));
        assert_eq!(
            p("https://github.com/a/b.rs/issues/12#issuecomment-1"),
            Some(("a/b.rs".into(), Kind::Issue("12".into())))
        );
        assert_eq!(p("https://github.com/a/b/pull/7/files"), None);
        assert_eq!(p("https://github.com/a/b/pull/7"), Some(("a/b".into(), Kind::Pull("7".into()))));
        assert_eq!(p("https://github.com/a/b/pulls?q=is%3Aopen"), Some(("a/b".into(), Kind::Pulls)));
        assert_eq!(
            p("https://github.com/a/b/releases/tag/v2.0.0"),
            Some(("a/b".into(), Kind::Release("v2.0.0".into())))
        );
        assert_eq!(p("https://github.com/a/b"), None);
        assert_eq!(p("https://gitlab.com/a/b/issues"), None);
        let page = parse("https://github.com/a/b/issues").unwrap();
        let rest = json!([
            {"number": 3, "title": "Crash on start", "state": "open", "user": {"login": "x"}, "comments": 2, "updated_at": "2026-10-30T10:00:00Z"},
            {"number": 2, "title": "A PR", "state": "open", "user": {"login": "y"}, "comments": 0, "updated_at": "2026-10-29T10:00:00Z", "pull_request": {}}
        ]);
        assert_eq!(
            format(&page, &rest, &Value::Null, "api.github.com"),
            "[github] a/b open issues, newest first (api.github.com)\n#3 open Crash on start (x, 2 comments, 2026-10-30)"
        );
        let page = parse("https://github.com/a/b/pull/7").unwrap();
        let gh = json!({"number": 7, "title": "Fix", "state": "MERGED", "author": {"login": "z"}, "body": "Body",
            "createdAt": "2026-10-01T00:00:00Z", "mergedAt": "2026-10-02T00:00:00Z", "changedFiles": 2, "additions": 5,
            "deletions": 1, "labels": [{"name": "bug"}], "comments": [{"author": {"login": "q"}, "body": "ok", "createdAt": "2026-10-02"}]});
        let out = format(&page, &gh, &gh["comments"], "gh");
        assert_eq!(
            out,
            "[github] a/b#7 Fix (merged, by z, 2026-10-01; gh)\n2 files, +5 -1\nlabels: bug\n\nBody\n\ncomments (1, last 1):\n- q (2026-10-02): ok"
        );
    }
}
