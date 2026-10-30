use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use lt_memory::VEC_DIM;
use serde_json::{Value, json};

use super::super::config;

const QUERY_PREFIX: &str = "Instruct: Given a question, find the notes that answer it\nQuery: ";
const EMBED_CHARS: usize = 1200;
const PROBE_LOCAL: Duration = Duration::from_millis(1500);
const PROBE_REMOTE: Duration = Duration::from_secs(3);
const RECHECK: Duration = Duration::from_secs(60);
const BACKOFF: Duration = Duration::from_secs(30);
const LOCAL_HOSTS: &[&str] = &["127.0.0.1", "localhost", "[::1]"];

struct State {
    probed: Option<(Instant, String, bool)>,
    down: Option<(Instant, String)>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State { probed: None, down: None }));
static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(true).build().into());

fn api(home: &Path) -> Option<(String, String)> {
    let cfg = config::load(home);
    let base = config::text(&cfg, "memory_api");
    let model = config::text(&cfg, "memory_api_embed");
    let base = base.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    (!base.is_empty() && !model.is_empty()).then(|| (base.to_string(), model))
}

fn local(base: &str) -> bool {
    let host = base.split("://").nth(1).unwrap_or(base).split(['/', ':']).next().unwrap_or("");
    LOCAL_HOSTS.contains(&host) || base.contains("://[::1]")
}

fn probe(base: &str) -> bool {
    let wait = if local(base) { PROBE_LOCAL } else { PROBE_REMOTE };
    AGENT.get(format!("{base}/v1/models")).config().timeout_global(Some(wait)).build().call().is_ok()
}

fn reachable(base: &str) -> bool {
    let fresh =
        STATE.lock().ok().and_then(|s| s.probed.clone()).filter(|(at, b, _)| b == base && at.elapsed() < RECHECK);
    if let Some((_, _, ok)) = fresh {
        return ok;
    }
    let ok = probe(base);
    if let Ok(mut s) = STATE.lock() {
        s.probed = Some((Instant::now(), base.to_string(), ok));
    }
    ok
}

fn reason(err: &ureq::Error) -> String {
    match err {
        ureq::Error::StatusCode(503) => "the model is paused".into(),
        ureq::Error::StatusCode(code) => format!("the model answered {code}"),
        ureq::Error::Timeout(_) => "the model did not answer in time".into(),
        _ => "the model is not reachable".into(),
    }
}

fn normalized(raw: &Value) -> Option<Vec<f32>> {
    let mut v: Vec<f32> = raw.as_array()?.iter().take(VEC_DIM).map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
    if v.len() != VEC_DIM {
        return None;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    Some(v)
}

pub fn embed(home: &Path, texts: &[String], query: bool, budget: Duration) -> Result<Vec<Vec<f32>>, String> {
    if let Some((until, why)) = STATE.lock().ok().and_then(|s| s.down.clone())
        && Instant::now() < until
    {
        return Err(why);
    }
    let Some((base, model)) = api(home) else { return Err("the model is not running".into()) };
    if !reachable(&base) {
        return Err("the model is not reachable".into());
    }
    let prefix = if query { QUERY_PREFIX } else { "" };
    let items: Vec<String> =
        texts.iter().map(|t| format!("{prefix}{}", t.chars().take(EMBED_CHARS).collect::<String>())).collect();
    let body = json!({"model": model, "input": items}).to_string();
    let sent = AGENT
        .post(format!("{base}/v1/embeddings"))
        .config()
        .timeout_global(Some(budget))
        .build()
        .header("Content-Type", "application/json")
        .send(&body);
    let text = match sent.and_then(|mut r| r.body_mut().read_to_string()) {
        Ok(text) => text,
        Err(e) => {
            let why = reason(&e);
            if let Ok(mut s) = STATE.lock() {
                s.down = Some((Instant::now() + BACKOFF, why.clone()));
            }
            return Err(why);
        }
    };
    let data: Value = serde_json::from_str(&text).map_err(|_| "the model sent no embeddings".to_string())?;
    let mut rows: Vec<&Value> =
        data.get("data").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    rows.sort_by_key(|d| d.get("index").and_then(Value::as_i64).unwrap_or(0));
    let vecs: Option<Vec<Vec<f32>>> = rows.iter().map(|d| d.get("embedding").and_then(normalized)).collect();
    match vecs {
        Some(v) if v.len() == texts.len() => Ok(v),
        _ => Err("the model sent no embeddings".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_are_cut_and_normalized() {
        let raw = json!((0..300).map(|i| if i < 2 { 3.0 } else { 0.0 }).collect::<Vec<f64>>());
        let v = normalized(&raw).unwrap();
        assert_eq!(v.len(), VEC_DIM);
        assert!((v[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && v[2] == 0.0);
        assert!(normalized(&json!([1.0, 2.0])).is_none());
        assert!(local("http://127.0.0.1:9930") && local("http://localhost/v1") && !local("https://api.example.com"));
    }
}
