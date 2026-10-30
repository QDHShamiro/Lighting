use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use regex::Regex;
use serde_json::{Map, Value, json};

use super::{Paths, neutral, num, rules, text};

const GEMINI: &str = "https://generativelanguage.googleapis.com/v1beta/models/{m}:streamGenerateContent?alt=sse";
pub const GROQ_CHAT: &str = "https://api.groq.com/openai/v1/chat/completions";
const SMALL_SEARCH: &str = "openai/gpt-oss-20b";
const MIN_START: usize = 7;
const MIN_SENTENCE: usize = 12;
const COOL_S: f64 = 60.0;
const DEFAULT_RPM: usize = 10;
const PERSONA_CHARS: usize = 1500;
const CHAT_WAIT: Duration = Duration::from_secs(60);
const SEARCH_WAIT: Duration = Duration::from_secs(40);
const ERR_CHARS: usize = 120;
const NO_RESULT: &str = "Hab grad nichts Sicheres dazu gefunden, sorry.";
const SEARCH_RULE: &str = "\nDu bist im Discord-Voice-Call. Antworte in 1 bis 2 kurzen gesprochenen Sätzen auf Deutsch mit dem Ergebnis, ohne Quellen, Links, Listen oder Markdown.";

static SENTENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^(.+?[^\d\s][.!?…])(\s+|$)").expect("sentence"));
static SENTENCES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)(.+?[^\d\s][.!?…])(\s+|$)").expect("sentences"));
static QUERY_CUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\n\-–]|\s{2,}").expect("query"));
static LINKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"【[^】]*】|\[[^\]]*\]\([^)]*\)|https?://\S+|[*#_`]").expect("links"));
static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(false).build().into());

pub enum Miss {
    Limit,
    Other(String),
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

pub fn post(
    url: &str,
    body: &Value,
    auth: (&str, &str),
    wait: Duration,
) -> Result<ureq::http::Response<ureq::Body>, Miss> {
    let resp = AGENT
        .post(url)
        .config()
        .timeout_global(Some(wait))
        .build()
        .header("User-Agent", "lighting")
        .header(auth.0, auth.1)
        .header("Content-Type", "application/json")
        .send(body.to_string().as_bytes())
        .map_err(|e| Miss::Other(e.to_string()))?;
    match resp.status().as_u16() {
        200 => Ok(resp),
        429 => Err(Miss::Limit),
        code => Err(Miss::Other(format!("HTTP {code}"))),
    }
}

fn sse(resp: ureq::http::Response<ureq::Body>, mut each: impl FnMut(&Value) -> bool) -> Result<bool, Miss> {
    let reader = BufReader::new(resp.into_body().into_reader());
    for line in reader.lines() {
        let line = line.map_err(|e| Miss::Other(e.to_string()))?;
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else { continue };
        if data.trim_end().ends_with("[DONE]") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
        if !each(&v) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub struct Brain {
    cfg: Map<String, Value>,
    keys: Map<String, Value>,
    p: Paths,
    pub turns: Vec<Value>,
    calls: HashMap<String, Vec<f64>>,
    cool: HashMap<String, f64>,
    rpm: HashMap<String, usize>,
    usage: [u64; 4],
    file: PathBuf,
    system: String,
    mtime: Option<SystemTime>,
    last: (u64, u64),
}

fn key<'a>(keys: &'a Map<String, Value>, name: &str) -> &'a str {
    keys.get(name).and_then(Value::as_str).unwrap_or("")
}

impl Brain {
    pub fn new(cfg: &Map<String, Value>, keys: &Map<String, Value>, p: Paths, target: &str) -> Brain {
        let rpm = text(cfg, "rpm")
            .split(',')
            .filter_map(|x| x.split_once('=').and_then(|(m, n)| Some((m.to_string(), n.trim().parse().ok()?))))
            .collect();
        let file = p.persona_file(target);
        let mut b = Brain {
            cfg: cfg.clone(),
            keys: keys.clone(),
            p,
            turns: Vec::new(),
            calls: HashMap::new(),
            cool: HashMap::new(),
            rpm,
            usage: [0; 4],
            file,
            system: String::new(),
            mtime: None,
            last: (0, 0),
        };
        b.system = b.persona() + "\n" + &rules(&b.cfg);
        b.mtime = fs::metadata(&b.file).and_then(|m| m.modified()).ok();
        b
    }

    fn persona(&self) -> String {
        let text = fs::read_to_string(&self.file).unwrap_or_default();
        let text = text.trim();
        if text.is_empty() { neutral().to_string() } else { text.to_string() }
    }

    fn reload(&mut self) {
        let Ok(m) = fs::metadata(&self.file).and_then(|m| m.modified()) else { return };
        if Some(m) != self.mtime {
            self.mtime = Some(m);
            self.system = self.persona() + "\n" + &rules(&self.cfg);
            self.p.log("persona reloaded");
        }
    }

    fn budget(&mut self, model: &str) -> bool {
        let t = now();
        let recent: Vec<f64> =
            self.calls.get(model).into_iter().flatten().copied().filter(|c| t - c < COOL_S).collect();
        let fits = recent.len() < self.rpm.get(model).copied().unwrap_or(DEFAULT_RPM);
        self.calls.insert(model.to_string(), recent);
        self.cool.get(model).copied().unwrap_or(0.0) < t && fits
    }

    pub fn remember(&mut self, role: &str, said: &str) {
        match self.turns.last_mut() {
            Some(last) if last["role"] == role => {
                let joined = format!("{}\n{said}", last["parts"][0]["text"].as_str().unwrap_or(""));
                last["parts"][0]["text"] = json!(joined);
            }
            _ => self.turns.push(json!({"role": role, "parts": [{"text": said}]})),
        }
        let keep = num(&self.cfg, "history") as usize;
        if self.turns.len() > keep {
            self.turns.drain(..self.turns.len() - keep);
        }
        while self.turns.first().is_some_and(|t| t["role"] != "user") {
            self.turns.remove(0);
        }
    }

    fn research(&mut self, query: &str) -> Result<String, Miss> {
        let persona: String =
            self.system.split("Du bist live").next().unwrap_or("").chars().take(PERSONA_CHARS).collect();
        let auth = format!("Bearer {}", key(&self.keys, "GROQ_API_KEY"));
        let first = text(&self.cfg, "search_model");
        let mut data = Value::Null;
        for model in [first.as_str(), SMALL_SEARCH] {
            let body = json!({"model": model, "tools": [{"type": "browser_search"}], "tool_choice": "required",
                "reasoning_effort": "low", "max_tokens": 900, "messages": [
                    {"role": "system", "content": persona.clone() + SEARCH_RULE},
                    {"role": "user", "content": format!("Such im Web und beantworte: {query}")}]});
            match post(GROQ_CHAT, &body, ("Authorization", &auth), SEARCH_WAIT) {
                Ok(resp) => {
                    data = resp
                        .into_body()
                        .read_to_string()
                        .ok()
                        .and_then(|t| serde_json::from_str(&t).ok())
                        .ok_or_else(|| Miss::Other("no JSON".into()))?;
                    break;
                }
                Err(Miss::Limit) if model == first => {
                    self.p.log(&format!("research: {model} busy, trying the small model"))
                }
                Err(e) => return Err(e),
            }
        }
        let u = &data["usage"];
        self.usage[0] += u["prompt_tokens"].as_u64().unwrap_or(0);
        self.usage[1] += u["completion_tokens"].as_u64().unwrap_or(0);
        self.usage[3] += 1;
        let answer = data["choices"][0]["message"]["content"].as_str().unwrap_or("");
        Ok(LINKS.replace_all(answer, "").trim().to_string())
    }

    fn pieces(&mut self, model: &str, mut each: impl FnMut(&str) -> bool) -> Result<bool, Miss> {
        let mut last = (0, 0);
        let mut searches = 0;
        let done = if let Some(m) = model.strip_prefix("groq:") {
            let mut msgs = vec![json!({"role": "system", "content": self.system})];
            msgs.extend(self.turns.iter().map(|t| {
                json!({"role": if t["role"] == "model" { "assistant" } else { "user" }, "content": t["parts"][0]["text"]})
            }));
            let body = json!({"model": m, "messages": msgs, "temperature": 0.9, "max_tokens": 400, "stream": true,
                "reasoning_effort": "low", "include_reasoning": false, "stream_options": {"include_usage": true}});
            let auth = format!("Bearer {}", key(&self.keys, "GROQ_API_KEY"));
            let resp = post(GROQ_CHAT, &body, ("Authorization", &auth), CHAT_WAIT)?;
            sse(resp, |data| {
                let u = if data["usage"].is_object() { &data["usage"] } else { &data["x_groq"]["usage"] };
                if u.is_object() {
                    last = (u["prompt_tokens"].as_u64().unwrap_or(0), u["completion_tokens"].as_u64().unwrap_or(0));
                }
                for ch in data["choices"].as_array().into_iter().flatten() {
                    if let Some(piece) = ch["delta"]["content"].as_str().filter(|s| !s.is_empty())
                        && !each(piece)
                    {
                        return false;
                    }
                }
                true
            })?
        } else {
            let body = json!({"systemInstruction": {"parts": [{"text": self.system}]}, "contents": self.turns,
                "generationConfig": {"temperature": 0.9, "maxOutputTokens": 160, "thinkingConfig": {"thinkingBudget": 0}}});
            let url = GEMINI.replace("{m}", model);
            let resp = post(&url, &body, ("x-goog-api-key", key(&self.keys, "GEMINI_API_KEY")), CHAT_WAIT)?;
            sse(resp, |data| {
                let meta = &data["usageMetadata"];
                if meta.is_object() {
                    last = (
                        meta["promptTokenCount"].as_u64().unwrap_or(0),
                        meta["candidatesTokenCount"].as_u64().unwrap_or(0),
                    );
                }
                let cand = &data["candidates"][0];
                if cand["groundingMetadata"]["webSearchQueries"].as_array().is_some_and(|q| !q.is_empty()) {
                    searches += 1;
                }
                for part in cand["content"]["parts"].as_array().into_iter().flatten() {
                    if let Some(piece) = part["text"].as_str().filter(|s| !s.is_empty())
                        && !each(piece)
                    {
                        return false;
                    }
                }
                true
            })?
        };
        self.last = last;
        self.usage[3] += searches;
        Ok(done)
    }

    pub fn stream(&mut self, event: &str, mut say: impl FnMut(&str, f64, &str) -> bool) {
        self.reload();
        self.remember("user", event);
        let models: Vec<String> =
            text(&self.cfg, "models").split(',').filter(|m| !m.is_empty()).map(String::from).collect();
        for model in models {
            if !self.budget(&model) {
                continue;
            }
            self.calls.entry(model.clone()).or_default().push(now());
            self.last = (0, 0);
            let t0 = now();
            let (mut said, mut buf, mut stopped) = (Vec::<String>::new(), String::new(), false);
            let got = self.pieces(&model, |piece| {
                buf += piece;
                let start = buf.trim_start();
                if start.chars().count() < MIN_START || start.to_uppercase().starts_with("SUCHE") {
                    return true;
                }
                while let Some(m) = SENTENCE.captures(&buf) {
                    let sentence = m[1].to_string();
                    if sentence.chars().count() < MIN_SENTENCE {
                        break;
                    }
                    let end = m.get(0).map_or(0, |x| x.end());
                    said.push(sentence.trim().to_string());
                    buf.drain(..end);
                    if !say(&model, t0, sentence.trim()) {
                        stopped = true;
                        return false;
                    }
                }
                true
            });
            if stopped {
                return;
            }
            match got {
                Ok(_) => {
                    let mut rest = buf.trim().to_string();
                    if rest.to_uppercase().starts_with("SUCHE") {
                        let asked = rest.split_once(':').map_or(rest.as_str(), |(_, q)| q).trim().to_string();
                        let query = QUERY_CUT.split(&asked).next().unwrap_or("").trim().to_string();
                        self.p.log(&format!("research ({} words)", query.split_whitespace().count()));
                        if !say(&model, t0, &text(&self.cfg, "search_filler")) {
                            return;
                        }
                        rest = self.research(&query).unwrap_or_else(|e| {
                            let why = match e {
                                Miss::Limit => "HTTP 429".to_string(),
                                Miss::Other(s) => s,
                            };
                            self.p
                                .log(&format!("research failed: {}", why.chars().take(ERR_CHARS).collect::<String>()));
                            NO_RESULT.to_string()
                        });
                        for m in SENTENCES.captures_iter(&format!("{rest} ")) {
                            let s = m[1].trim().to_string();
                            said.push(s.clone());
                            if !say(&model, t0, &s) {
                                return;
                            }
                        }
                        if !said.is_empty() {
                            rest.clear();
                        }
                    }
                    if !rest.is_empty() && rest != "-" {
                        said.push(rest.clone());
                        if !say(&model, t0, &rest) {
                            return;
                        }
                    }
                    let joined = if said.is_empty() { "-".to_string() } else { said.join(" ") };
                    self.remember("model", &joined);
                    self.usage[0] += self.last.0;
                    self.usage[1] += self.last.1;
                    self.usage[2] += 1;
                    let [i, o, r, s] = self.usage;
                    self.p.state(json!({"usage": {"in": i, "out": o, "requests": r, "searches": s}}));
                    return;
                }
                Err(e) => {
                    if !said.is_empty() {
                        self.remember("model", &said.join(" "));
                        return;
                    }
                    let why = match e {
                        Miss::Limit => {
                            self.cool.insert(model.clone(), now() + COOL_S);
                            "HTTP 429 Too Many Requests".to_string()
                        }
                        Miss::Other(s) => s,
                    };
                    self.p.log(&format!("brain {model} failed: {}", why.chars().take(ERR_CHARS).collect::<String>()));
                }
            }
        }
        self.p.log("brain: every model is busy, staying quiet");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_split_like_2x() {
        let m = SENTENCE.captures("Na klar, das geht schon. Und dann").unwrap();
        assert_eq!(&m[1], "Na klar, das geht schon.");
        assert!(SENTENCE.captures("Version 1.2 ist").is_none());
        let all: Vec<String> =
            SENTENCES.captures_iter("Erste Antwort hier. Zweite? ").map(|m| m[1].to_string()).collect();
        assert_eq!(all, ["Erste Antwort hier.", "Zweite?"]);
        assert_eq!(QUERY_CUT.split("patch notes - minecraft").next(), Some("patch notes "));
        assert_eq!(LINKS.replace_all("Das **ist** [x](http://a) neu https://b.c 【1】", ""), "Das ist  neu  ");
    }
}
