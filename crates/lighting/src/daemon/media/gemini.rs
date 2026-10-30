use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use base64::Engine;
use lt_core::Fail;
use serde_json::{Map, Value, json};

use super::super::config;
use super::super::memory::chat::{gemini_key, wait_turn};

const API: &str = "https://generativelanguage.googleapis.com";
const ANSWER_WAIT: Duration = Duration::from_secs(300);
const UPLOAD_WAIT: Duration = Duration::from_secs(600);
const READY_WAIT: Duration = Duration::from_secs(300);
const READY_TICK: Duration = Duration::from_secs(2);
const FILE_HOURS: f64 = 46.0;
const INLINE_BYTES: u64 = 14 * 1024 * 1024;
const TEMPERATURE: f64 = 0.2;
const DETAIL_CHARS: usize = 200;
const KEY_HINT: &str = "set GEMINI_API_KEY (free key: aistudio.google.com/apikey) or put it in ~/.config/watch/.env";

static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(false).build().into());

pub struct Gemini {
    key: String,
    pub model: String,
}

pub struct Answer {
    pub text: String,
    pub tokens: i64,
    pub calls: i64,
}

pub fn gemini(home: &Path) -> Result<Gemini, Fail> {
    let cfg = config::load(home);
    if !config::on(&cfg, "gemini") {
        return Err(Fail::hint("Gemini is off", "lighting config gemini on"));
    }
    let key = gemini_key().ok_or_else(|| Fail::hint("no Gemini key", KEY_HINT))?;
    Ok(Gemini { key, model: config::text(&cfg, "gemini_model") })
}

fn detail(text: &str) -> String {
    let msg = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| text.to_string());
    msg.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(DETAIL_CHARS).collect()
}

fn refused(status: u16, text: &str) -> Fail {
    match status {
        429 => Fail::hint(
            format!("Gemini free quota is used up for now ({})", detail(text)),
            "wait a minute, or lighting watch ... --local",
        ),
        401 | 403 => Fail::hint("the Gemini key was refused", KEY_HINT),
        _ => Fail::new(format!("Gemini answered {status}: {}", detail(text))),
    }
}

pub fn inline(mime: &str, raw: &[u8]) -> Value {
    json!({"inline_data": {"mime_type": mime, "data": base64::engine::general_purpose::STANDARD.encode(raw)}})
}

pub fn linked(mime: Option<&str>, uri: &str) -> Value {
    match mime {
        Some(mime) => json!({"file_data": {"mime_type": mime, "file_uri": uri}}),
        None => json!({"file_data": {"file_uri": uri}}),
    }
}

impl Gemini {
    pub fn ask(&self, home: &Path, parts: Vec<Value>, sharp: bool) -> Result<Answer, Fail> {
        let mut gen_cfg = Map::new();
        gen_cfg.insert("temperature".into(), json!(TEMPERATURE));
        let level = if sharp { "MEDIA_RESOLUTION_MEDIUM" } else { "MEDIA_RESOLUTION_LOW" };
        gen_cfg.insert("mediaResolution".into(), json!(level));
        if self.model.contains("flash") {
            gen_cfg.insert("thinkingConfig".into(), json!({"thinkingBudget": 0}));
        }
        let body = json!({"contents": [{"role": "user", "parts": parts}], "generationConfig": gen_cfg});
        let url = format!("{API}/v1beta/models/{}:generateContent", self.model);
        wait_turn();
        let mut resp = AGENT
            .post(&url)
            .config()
            .timeout_global(Some(ANSWER_WAIT))
            .build()
            .header("x-goog-api-key", &self.key)
            .header("Content-Type", "application/json")
            .send(&body.to_string())
            .map_err(|e| Fail::new(format!("Gemini not reachable ({e})")))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().with_config().limit(u64::MAX).read_to_string().unwrap_or_default();
        if status != 200 {
            return Err(refused(status, &text));
        }
        let data: Value = serde_json::from_str(&text).map_err(|_| Fail::new("Gemini sent no JSON"))?;
        let answer: String = data
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .map(|parts| parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect())
            .unwrap_or_default();
        if answer.trim().is_empty() {
            let why = data.pointer("/candidates/0/finishReason").and_then(Value::as_str).unwrap_or("no text");
            return Err(Fail::new(format!("Gemini gave no answer ({why})")));
        }
        let tokens = data.pointer("/usageMetadata/totalTokenCount").and_then(Value::as_i64).unwrap_or(0);
        let calls = count(home, &self.model, tokens);
        Ok(Answer { text: answer.trim().to_string(), tokens, calls })
    }

    pub fn file_part(&self, path: &Path, mime: &str, memo: &Path) -> Result<Value, Fail> {
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size <= INLINE_BYTES {
            let raw = fs::read(path).map_err(|e| Fail::new(format!("cannot read {} ({e})", path.display())))?;
            return Ok(inline(mime, &raw));
        }
        let fresh = fs::read_to_string(memo)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(|v| v["at"].as_f64().is_some_and(|at| lt_memory::now() - at < FILE_HOURS * 3600.0))
            .and_then(|v| v["uri"].as_str().map(String::from));
        let uri = match fresh {
            Some(uri) => uri,
            None => {
                let uri = self.upload(path, mime, size)?;
                let _ = fs::write(memo, json!({"uri": uri, "at": lt_memory::now()}).to_string());
                uri
            }
        };
        Ok(linked(Some(mime), &uri))
    }

    fn upload(&self, path: &Path, mime: &str, size: u64) -> Result<String, Fail> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let lost = |e: ureq::Error| Fail::new(format!("Gemini upload failed ({e})"));
        let start = AGENT
            .post(format!("{API}/upload/v1beta/files"))
            .header("x-goog-api-key", &self.key)
            .header("X-Goog-Upload-Protocol", "resumable")
            .header("X-Goog-Upload-Command", "start")
            .header("X-Goog-Upload-Header-Content-Length", size.to_string())
            .header("X-Goog-Upload-Header-Content-Type", mime)
            .header("Content-Type", "application/json")
            .send(&json!({"file": {"display_name": name}}).to_string())
            .map_err(lost)?;
        let status = start.status().as_u16();
        let target = start.headers().get("x-goog-upload-url").and_then(|v| v.to_str().ok()).map(String::from);
        let Some(target) = target.filter(|_| status == 200) else {
            let mut start = start;
            return Err(refused(status, &start.body_mut().read_to_string().unwrap_or_default()));
        };
        let raw = fs::read(path).map_err(|e| Fail::new(format!("cannot read {} ({e})", path.display())))?;
        let mut done = AGENT
            .post(&target)
            .config()
            .timeout_global(Some(UPLOAD_WAIT))
            .build()
            .header("X-Goog-Upload-Offset", "0")
            .header("X-Goog-Upload-Command", "upload, finalize")
            .send(&raw[..])
            .map_err(lost)?;
        let status = done.status().as_u16();
        let text = done.body_mut().read_to_string().unwrap_or_default();
        if status != 200 {
            return Err(refused(status, &text));
        }
        let file: Value = serde_json::from_str(&text).map_err(|_| Fail::new("Gemini upload sent no JSON"))?;
        let id = file.pointer("/file/name").and_then(Value::as_str).unwrap_or_default().to_string();
        let uri = file.pointer("/file/uri").and_then(Value::as_str).unwrap_or_default().to_string();
        let mut state = file.pointer("/file/state").and_then(Value::as_str).unwrap_or("ACTIVE").to_string();
        let waited = std::time::Instant::now();
        while state == "PROCESSING" && waited.elapsed() < READY_WAIT {
            thread::sleep(READY_TICK);
            let mut resp =
                AGENT.get(format!("{API}/v1beta/{id}")).header("x-goog-api-key", &self.key).call().map_err(lost)?;
            let info: Value =
                serde_json::from_str(&resp.body_mut().read_to_string().unwrap_or_default()).unwrap_or_default();
            state = info["state"].as_str().unwrap_or("FAILED").to_string();
        }
        if state != "ACTIVE" || uri.is_empty() {
            return Err(Fail::new(format!("Gemini could not process the file ({})", state.to_lowercase())));
        }
        Ok(uri)
    }
}

fn count(home: &Path, model: &str, tokens: i64) -> i64 {
    let path = home.join("ai").join("usage.json");
    let today = lt_memory::date(lt_memory::now());
    let mut data: Value =
        fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if data["day"].as_str() != Some(today.as_str()) {
        data = json!({"day": today, "calls": {}, "tokens": {}});
    }
    let calls = data["calls"][model].as_i64().unwrap_or(0) + 1;
    data["calls"][model] = json!(calls);
    data["tokens"][model] = json!(data["tokens"][model].as_i64().unwrap_or(0) + tokens);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(&path, data.to_string());
    calls
}

pub fn usage(home: &Path) -> Option<String> {
    let data: Value = serde_json::from_str(&fs::read_to_string(home.join("ai").join("usage.json")).ok()?).ok()?;
    if data["day"].as_str() != Some(lt_memory::date(lt_memory::now()).as_str()) {
        return None;
    }
    let calls = data["calls"].as_object()?;
    let parts: Vec<String> = calls
        .iter()
        .map(|(m, n)| {
            format!("{m} {} calls, {} tokens", n.as_i64().unwrap_or(0), data["tokens"][m].as_i64().unwrap_or(0))
        })
        .collect();
    (!parts.is_empty()).then(|| format!("Gemini today: {}", parts.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_counts_calls_per_day_and_errors_read_plainly() {
        let home = std::env::temp_dir().join(format!("lt-gem-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        assert_eq!(count(&home, "m", 10), 1);
        assert_eq!(count(&home, "m", 5), 2);
        assert_eq!(usage(&home).as_deref(), Some("Gemini today: m 2 calls, 15 tokens"));
        let _ = fs::remove_dir_all(&home);
        let quota = refused(429, r#"{"error":{"message":"Resource has been exhausted"}}"#).text();
        assert!(
            quota.starts_with("err: Gemini free quota is used up for now (Resource has been exhausted)"),
            "{quota}"
        );
        assert_eq!(refused(500, "x  y").text(), "err: Gemini answered 500: x y");
        assert_eq!(linked(None, "u"), json!({"file_data": {"file_uri": "u"}}));
    }
}
