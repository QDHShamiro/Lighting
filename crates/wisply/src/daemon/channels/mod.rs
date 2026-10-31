pub mod github;
pub mod ledger;

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use wisply_core::{Call, Fail};

use super::ctx::Ctx;
use super::media::{gemini, tools};
use super::memory::chat::api_key;
use super::{Daemon, Out, config, web};

const GH_CHECK: Duration = Duration::from_secs(600);
const GH_WAIT: Duration = Duration::from_secs(8);
const HOSTS: usize = 8;

static GH_LOGIN: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

fn gh_ready() -> Option<bool> {
    let gh = tools::which("gh")?;
    let mut seen = GH_LOGIN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, ok)) = *seen
        && at.elapsed() < GH_CHECK
    {
        return Some(ok);
    }
    let ok = tools::run(&gh, &["auth".into(), "status".into()], GH_WAIT).is_ok_and(|r| r.ok);
    *seen = Some((Instant::now(), ok));
    Some(ok)
}

fn way(name: &str, ready: bool, why: &str) -> String {
    if ready { name.to_string() } else { format!("{name} ({why})") }
}

fn line(home: &Path, name: &str, ways: &[String], key: &str) -> String {
    let last = ledger::last(home, key).map(|l| format!("\n         last: {l}")).unwrap_or_default();
    format!("{name:<8} {}{last}", ways.join(" > "))
}

pub fn report(d: &Daemon) -> String {
    let home = &d.home;
    let browser = web::any_browser(d);
    let brave = |what: &str| way(what, browser, "no browser connected");
    let ytdlp = tools::ytdlp(home).is_ok();
    let ffmpeg = tools::ffmpeg(home).is_some();
    let gem = gemini::gemini(home).is_ok();
    let cloud = ["GROQ_API_KEY", "OPENAI_API_KEY"].iter().any(|k| api_key(k).is_some());
    let local = !config::text(&config::load(home), "whisper_api").is_empty();
    let gh = match gh_ready() {
        Some(true) => "gh".to_string(),
        Some(false) => "gh (not logged in: gh auth login)".to_string(),
        None => "gh (not installed)".to_string(),
    };
    let mut out = vec!["channels (ways in the order Wisply tries them; read only, nothing is posted)".to_string()];
    out.push(line(
        home,
        "github",
        &[gh, "api.github.com (60 calls/h without login)".into(), brave("browser")],
        "github",
    ));
    out.push(line(home, "web", &["direct".into(), brave("browser")], "web"));
    let sites = ledger::hosts(home, "web:");
    if !sites.is_empty() {
        let shown: Vec<String> = sites.into_iter().take(HOSTS).collect();
        out.push(format!("         per site: {}", shown.join(", ")));
    }
    out.push(line(
        home,
        "video",
        &[
            way("gemini", gem, "no GEMINI_API_KEY"),
            way("yt-dlp + ffmpeg frames", ytdlp && ffmpeg, "missing yt-dlp or ffmpeg"),
            brave("storyboard in the browser"),
        ],
        "video",
    ));
    out.push(line(
        home,
        "audio",
        &[
            way("captions (yt-dlp)", ytdlp, "no yt-dlp"),
            way("speech-to-text", cloud || local, "no GROQ_API_KEY or whisper_api"),
            brave("captions on the page"),
        ],
        "audio",
    ));
    out.join("\n")
}

pub fn run(d: &Daemon, _: &mut Ctx, c: &Call) -> Result<Out, Fail> {
    if let Some(extra) = c.pos.first() {
        return Err(Fail::hint(format!("channels takes no argument ({extra})"), "wisply channels"));
    }
    Ok(Out::Text(report(d)))
}

pub fn site(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("");
    host.split(':').next().unwrap_or("").trim_start_matches("www.").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_bare_host_names() {
        assert_eq!(site("https://www.Example.com:8080/a?b"), "example.com");
        assert_eq!(site("http://user@docs.rs/x"), "docs.rs");
    }
}
