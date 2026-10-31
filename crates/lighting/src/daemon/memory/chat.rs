use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::super::config;

const GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";
const KEY_NAME: &str = "GEMINI_API_KEY";
const KEY_FILE: [&str; 3] = [".config", "watch", ".env"];
const GEMINI_WAIT: Duration = Duration::from_secs(120);
const LOCAL_WAIT: Duration = Duration::from_secs(600);
const GEMINI_GAP: Duration = Duration::from_secs(7);
const ANSWER_TOKENS: u32 = 1500;
const TEMPERATURE: f64 = 0.2;
const DETAIL_CHARS: usize = 200;
const LOCAL_HOSTS: &[&str] = &["://127.0.0.1", "://localhost", "://[::1]"];

static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(false).build().into());
static LAST_GEMINI: Mutex<Option<Instant>> = Mutex::new(None);

pub enum Provider {
    Gemini { key: String, model: String },
    Api { base: String, model: String },
}

pub enum ChatError {
    NotReady(String),
    Failed(String),
}

impl Provider {
    pub fn local(&self) -> bool {
        match self {
            Provider::Gemini { .. } => false,
            Provider::Api { base, .. } => LOCAL_HOSTS.iter().any(|h| base.contains(h)),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Provider::Gemini { model, .. } => format!("Gemini {model}"),
            Provider::Api { base, model } => {
                let host = base.split_once("://").map_or(base.as_str(), |(_, r)| r).split('/').next().unwrap_or("");
                format!("{model} at {}", host.rsplit_once('@').map_or(host, |(_, h)| h))
            }
        }
    }
}

fn user_home() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

pub fn gemini_key() -> Option<String> {
    api_key(KEY_NAME)
}

pub fn api_key(name: &str) -> Option<String> {
    if let Some(k) = env::var(name).ok().filter(|k| !k.trim().is_empty()) {
        return Some(k.trim().to_string());
    }
    let file = KEY_FILE.iter().fold(user_home(), |p, part| p.join(part));
    let text = fs::read_to_string(file).ok()?;
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        let v = v.trim().trim_matches(['"', '\'']);
        (k.trim() == name && !v.is_empty()).then(|| v.to_string())
    })
}

pub fn provider(home: &Path) -> Option<Provider> {
    let cfg = config::load(home);
    if config::on(&cfg, "gemini")
        && let Some(key) = gemini_key()
    {
        return Some(Provider::Gemini { key, model: config::text(&cfg, "gemini_model") });
    }
    let base = config::text(&cfg, "memory_api");
    let model = config::text(&cfg, "memory_api_model");
    let base = base.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    (!base.is_empty() && !model.is_empty()).then(|| Provider::Api { base: base.to_string(), model })
}

pub fn wait_turn() {
    let Ok(mut last) = LAST_GEMINI.lock() else { return };
    if let Some(at) = *last {
        let left = GEMINI_GAP.saturating_sub(at.elapsed());
        if !left.is_zero() {
            std::thread::sleep(left);
        }
    }
    *last = Some(Instant::now());
}

pub fn answer_text(raw: &str) -> String {
    let mut out = raw.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</think>".len(), "");
    }
    out.trim().to_string()
}

pub fn chat(p: &Provider, system: &str, user: &str, schema: &Value) -> Result<String, ChatError> {
    let messages = json!([{"role": "system", "content": system}, {"role": "user", "content": user}]);
    let format = json!({"type": "json_schema", "json_schema": {"name": "memory", "schema": schema}});
    let mut body = json!({"temperature": TEMPERATURE, "max_tokens": ANSWER_TOKENS, "messages": messages, "response_format": format});
    let (url, wait, auth) = match p {
        Provider::Gemini { key, model } => {
            body["model"] = json!(model);
            body["reasoning_effort"] = json!("none");
            wait_turn();
            (GEMINI_URL.to_string(), GEMINI_WAIT, Some(format!("Bearer {key}")))
        }
        Provider::Api { base, model } => {
            body["model"] = json!(model);
            body["chat_template_kwargs"] = json!({"enable_thinking": false});
            (format!("{base}/v1/chat/completions"), LOCAL_WAIT, None)
        }
    };
    let mut req =
        AGENT.post(&url).config().timeout_global(Some(wait)).build().header("Content-Type", "application/json");
    if let Some(auth) = auth {
        req = req.header("Authorization", auth);
    }
    let mut resp =
        req.send(&body.to_string()).map_err(|e| ChatError::NotReady(format!("{}: not reachable ({e})", p.name())))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if status != 200 {
        let detail: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(DETAIL_CHARS).collect();
        let why = format!("{} answered {status}: {detail}", p.name());
        return Err(if [429, 500, 502, 503, 504].contains(&status) {
            ChatError::NotReady(why)
        } else {
            ChatError::Failed(why)
        });
    }
    let data: Value =
        serde_json::from_str(&text).map_err(|_| ChatError::Failed(format!("{} sent no JSON", p.name())))?;
    let content = data.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or("");
    Ok(answer_text(content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_lose_their_thinking() {
        assert_eq!(answer_text("<think>hmm</think> {\"a\":1} "), "{\"a\":1}");
        assert_eq!(answer_text("plain"), "plain");
        let api = Provider::Api { base: "https://u:k@api.example.com".into(), model: "brain".into() };
        assert_eq!(api.name(), "brain at api.example.com");
        let near = Provider::Api { base: "http://127.0.0.1:9930".into(), model: "brain".into() };
        assert!(!api.local() && near.local() && !Provider::Gemini { key: "k".into(), model: "m".into() }.local());
    }
}
