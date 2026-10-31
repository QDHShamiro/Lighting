use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, LazyLock};
use std::thread;
use std::time::Duration;

use regex::{Regex, RegexBuilder};
use serde_json::{Map, Value, json};

use super::brain::{Brain, now};
use super::ears::{Ear, Utterance};
use super::mouth::Mouth;
use super::{Paths, defaults, keys, num, text};

const GROQ_ASR: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const ASR_RATE: u32 = 16_000;
const ASR_WAIT: Duration = Duration::from_secs(15);
const ASR_COOL_S: f64 = 60.0;
const POLL: Duration = Duration::from_millis(200);
const BATCH_POLL: Duration = Duration::from_millis(50);
const BATCH_MAX_S: f64 = 12.0;
const BYE_WAIT_S: f64 = 6.0;
const BUSY_TICK: Duration = Duration::from_millis(50);
const BUSY_TICKS: usize = 200;
const FIRST_TICKS: usize = 60;
const NUDGES: u32 = 3;
const ERR_CHARS: usize = 120;

static JUNK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)untertitel|amara\.org|vielen dank f(ü|ue)rs zuschauen|copyright|swr \d|zdf|^\W*$").expect("junk")
});
static QUESTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\?|^(wer|was|wann|wie|wo|warum|wieso|welche[rsnm]?|weißt du|kennst du|stimmt es)\b").expect("q")
});
static YOU: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(du|dir|dich|dein\w*)\b").expect("you"));
static CLOSED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[.!?…]\s*$").expect("closed"));
static OPEN_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(und|also|weil|dass|aber|dann|oder)\W*$").expect("open"));

fn cut(s: &str) -> String {
    s.chars().take(ERR_CHARS).collect()
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let n = u32::try_from(pcm.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(pcm.len() + 44);
    out.extend(b"RIFF");
    out.extend((36 + n).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(ASR_RATE.to_le_bytes());
    out.extend((ASR_RATE * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(n.to_le_bytes());
    out.extend(pcm);
    out
}

pub fn clean(said: &str, names: &str) -> String {
    let bare = said.trim_matches([' ', '.', ',', '!', '?']).to_lowercase();
    if said.chars().count() < 2 || JUNK.is_match(said) || names.to_lowercase().contains(&bare) {
        String::new()
    } else {
        said.to_string()
    }
}

struct Asr {
    names: String,
    groq: Option<String>,
    cool_until: f64,
    p: Paths,
    warned: bool,
}

enum Posted {
    Ok(Value),
    Limit,
    Fail(String),
}

fn post(url: &str, auth: Option<&str>, fields: &[(&str, &str)], audio: &[u8]) -> Posted {
    let (body, ctype) = crate::daemon::multipart(fields, "u.wav", audio);
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut req = agent.post(url).config().timeout_global(Some(ASR_WAIT)).build().header("Content-Type", ctype);
    if let Some(a) = auth {
        req = req.header("Authorization", a);
    }
    match req.send(&body[..]) {
        Ok(mut r) if r.status().as_u16() == 200 => r
            .body_mut()
            .read_to_string()
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .map_or_else(|| Posted::Fail("no JSON".into()), Posted::Ok),
        Ok(r) if r.status().as_u16() == 429 => Posted::Limit,
        Ok(r) => Posted::Fail(format!("HTTP {}", r.status().as_u16())),
        Err(e) => Posted::Fail(e.to_string()),
    }
}

fn joined(data: &Value, keep: impl Fn(&Value) -> bool) -> String {
    let segs: Vec<Value> = data["segments"].as_array().cloned().unwrap_or_else(|| vec![json!({"text": data["text"]})]);
    segs.iter()
        .filter(|s| keep(s))
        .map(|s| s["text"].as_str().unwrap_or("").trim().to_string())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

impl Asr {
    fn local(&mut self, audio: &[u8]) -> Result<String, String> {
        let (base, model) = crate::daemon::whisper_service(&self.p.home);
        if base.is_empty() {
            if !self.warned {
                self.warned = true;
                self.p.log("asr: no local speech-to-text (wisply config whisper_api <LocalAI url>)");
            }
            return Ok(String::new());
        }
        let fields = [
            ("model", model.as_str()),
            ("language", "de"),
            ("response_format", "verbose_json"),
            ("prompt", self.names.as_str()),
        ];
        match post(&format!("{base}/v1/audio/transcriptions"), None, &fields, audio) {
            Posted::Ok(data) => {
                Ok(clean(&joined(&data, |s| s["no_speech_prob"].as_f64().unwrap_or(0.0) < 0.6), &self.names))
            }
            Posted::Limit => Err("local speech-to-text is busy".into()),
            Posted::Fail(e) => Err(e),
        }
    }

    fn transcribe(&mut self, pcm: &[u8]) -> Result<String, String> {
        let audio = wav(pcm);
        let Some(key) = self.groq.clone().filter(|_| now() >= self.cool_until) else { return self.local(&audio) };
        let fields = [
            ("model", "whisper-large-v3-turbo"),
            ("language", "de"),
            ("response_format", "verbose_json"),
            ("temperature", "0"),
            ("prompt", self.names.as_str()),
        ];
        match post(GROQ_ASR, Some(&format!("Bearer {key}")), &fields, &audio) {
            Posted::Ok(data) => Ok(clean(
                &joined(&data, |s| crate::daemon::spoken(s) || s.get("no_speech_prob").is_none()),
                &self.names,
            )),
            Posted::Limit => {
                self.cool_until = now() + ASR_COOL_S;
                self.p.log("asr: groq limit, using the local model for a minute");
                self.local(&audio)
            }
            Posted::Fail(e) => Err(e),
        }
    }
}

struct Call {
    p: Paths,
    brain: Brain,
    mouth: Arc<Mouth>,
    talk: Arc<AtomicBool>,
}

impl Call {
    fn respond(&mut self, event: &str, addressed: bool, t_heard: f64, asr_s: f64) -> bool {
        if self.mouth.busy() {
            if !addressed {
                self.brain.remember("user", event);
                return false;
            }
            for _ in 0..BUSY_TICKS {
                if !self.mouth.busy() {
                    break;
                }
                thread::sleep(BUSY_TICK);
            }
        }
        self.mouth.reset_first();
        let (mut spoke, mut dropped) = (false, false);
        let (mouth, talk, p) = (Arc::clone(&self.mouth), Arc::clone(&self.talk), self.p.clone());
        self.brain.stream(event, |model, t0, sentence| {
            if !spoke && talk.load(Ordering::SeqCst) && !addressed {
                p.log(&format!("dropped (someone started talking): {sentence}"));
                dropped = true;
                return false;
            }
            if !spoke {
                p.log(&format!("timing: asr {asr_s:.2}s, first sentence {:.2}s ({model})", now() - t0));
            }
            mouth.say(sentence);
            spoke = true;
            true
        });
        if dropped {
            self.brain.remember("model", "-");
            return false;
        }
        if spoke {
            for _ in 0..FIRST_TICKS {
                if let Some(first) = self.mouth.first() {
                    let latency = ((first - t_heard) * 100.0).round() / 100.0;
                    self.p.state(json!({"latency": latency}));
                    self.p.log(&format!("timing: end of speech to first audio {:.2}s", first - t_heard));
                    break;
                }
                thread::sleep(BUSY_TICK);
            }
        }
        spoke
    }
}

fn unfinished(said: &str) -> bool {
    !CLOSED.is_match(said) || OPEN_END.is_match(said)
}

fn pattern(raw: &str, fallback: &str) -> Regex {
    let src = if raw.is_empty() { fallback } else { raw };
    RegexBuilder::new(src).case_insensitive(true).build().unwrap_or_else(|_| Regex::new(fallback).expect("fallback"))
}

fn take_lines(p: &Paths) -> Vec<String> {
    let mut files: Vec<_> = fs::read_dir(p.say_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|f| f.extension().is_some_and(|x| x == "txt"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let said = fs::read_to_string(&f).unwrap_or_default().trim().to_string();
            let _ = fs::remove_file(&f);
            said
        })
        .collect()
}

pub fn call(p: &Paths, target: &str) -> Result<(), String> {
    let cfg: Map<String, Value> = p.config();
    let k = keys();
    let missing: Vec<&str> = ["GROQ_API_KEY", "GEMINI_API_KEY"].into_iter().filter(|n| !k.contains_key(*n)).collect();
    if !missing.is_empty() {
        return Err(format!("missing keys: {}", missing.join(", ")));
    }
    let _ = fs::create_dir_all(p.say_dir());
    p.state(json!({"pid": std::process::id(), "target": target, "started": now(), "status": "running", "last_heard": "", "last_said": ""}));
    let brain = Brain::new(&cfg, &k, p.clone(), target);
    let mouth = Mouth::new(&cfg, p)?;
    let owner = Some(text(&cfg, "owner_name")).filter(|o| !o.is_empty()).unwrap_or_else(|| "Owner".into());
    let base = defaults();
    let leave = pattern(&text(&cfg, "stop"), base["stop"].as_str().unwrap_or(""));
    let wake = pattern(&text(&cfg, "wake"), base["wake"].as_str().unwrap_or(""));
    let (utt_tx, utt_rx) = mpsc::channel::<Utterance>();
    let (heard_tx, heard_rx) = mpsc::channel::<(f64, f64, String)>();
    let stop = Arc::new(AtomicBool::new(false));
    let talk = Arc::new(AtomicBool::new(false));
    let mut ears = vec![false];
    if !text(&cfg, "owner_mic").is_empty() {
        ears.push(true);
    }
    for owner_ear in ears {
        let ear = Ear {
            cfg: cfg.clone(),
            out: utt_tx.clone(),
            stop: Arc::clone(&stop),
            mouth: Arc::clone(&mouth),
            talk: Arc::clone(&talk),
            owner: owner_ear,
        };
        let ep = p.clone();
        thread::spawn(move || ear.run(&ep));
    }
    let mut asr = Asr {
        names: text(&cfg, "names"),
        groq: k.get("GROQ_API_KEY").and_then(Value::as_str).map(String::from),
        cool_until: 0.0,
        p: p.clone(),
        warned: false,
    };
    let prefix = format!("{owner}: ");
    thread::spawn(move || {
        while let Ok((t_end, pcm, mine)) = utt_rx.recv() {
            let t0 = now();
            match asr.transcribe(&pcm) {
                Ok(said) if !said.is_empty() => {
                    let shown = if mine { format!("{prefix}{said}") } else { said };
                    let _ = heard_tx.send((t_end, now() - t0, shown));
                }
                Ok(_) => {}
                Err(e) => asr.p.log(&format!("asr failed: {}", cut(&e))),
            }
        }
    });
    let opening: Vec<String> = [super::disclosure(&cfg), text(&cfg, "greeting").trim().to_string()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    mouth.say(&opening.join(" "));
    let mut c = Call { p: p.clone(), brain, mouth: Arc::clone(&mouth), talk: Arc::clone(&talk) };
    let (mut last_voice, mut last_me, mut nudged) = (now(), 0.0f64, 0u32);
    loop {
        for said in take_lines(p) {
            if said == "__leave__" {
                stop.store(true, Ordering::SeqCst);
                p.state(json!({"status": "stopped"}));
                return Ok(());
            }
            if let Some(q) = said.strip_prefix('?') {
                c.respond(
                    &format!("[{owner} schreibt dir privat: {}] Sag dazu was in den Call.", q.trim()),
                    true,
                    now(),
                    0.0,
                );
            } else {
                c.brain.remember("model", &said);
                mouth.say(&said);
            }
            last_voice = now();
            last_me = last_voice;
        }
        let (t_end, mut asr_s, first) = match heard_rx.recv_timeout(POLL) {
            Ok(h) => h,
            Err(RecvTimeoutError::Timeout) => {
                let quiet_for = now() - last_voice.max(last_me);
                let limit = num(&cfg, "silence_s") * f64::from(2u32.pow(nudged));
                if !talk.load(Ordering::SeqCst) && !mouth.busy() && quiet_for > limit && nudged < NUDGES {
                    nudged += 1;
                    let event = format!(
                        "[Seit {} Sekunden sagt keiner was. Bring locker ein Thema rein, das zu vorher passt.]",
                        quiet_for as i64
                    );
                    if c.respond(&event, false, now(), 0.0) {
                        last_me = now();
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return Err("speech-to-text stopped".into()),
        };
        let (mut batch, t_first, mut t_last) = (vec![first.clone()], t_end, t_end);
        let settle = |s: &str| num(&cfg, if unfinished(s) { "open_settle_s" } else { "settle_s" });
        let mut deadline = now() + settle(&first);
        while now() < deadline || talk.load(Ordering::SeqCst) {
            match heard_rx.recv_timeout(BATCH_POLL) {
                Ok((t, a, more)) => {
                    t_last = t;
                    asr_s = asr_s.max(a);
                    deadline = now() + settle(&more);
                    batch.push(more);
                }
                Err(_) if now() - t_first > BATCH_MAX_S => break,
                Err(_) => {}
            }
        }
        let said = batch.join(" ");
        let words = said.split_whitespace().count();
        p.log(&format!("heard {words} words"));
        p.state(json!({"last_heard": format!("{words} words")}));
        last_voice = now();
        nudged = 0;
        let plain = said.replace(&format!("{owner}: "), "");
        if leave.is_match(&plain) {
            p.log("left: someone said the stop word");
            let bye = Some(text(&cfg, "bye")).filter(|b| !b.is_empty()).unwrap_or_else(|| text(&base, "bye"));
            mouth.say(&bye);
            let until = now() + BYE_WAIT_S;
            while mouth.busy() && now() < until {
                thread::sleep(Duration::from_millis(100));
            }
            stop.store(true, Ordering::SeqCst);
            p.state(json!({"status": "stopped", "reason": "stop word"}));
            return Ok(());
        }
        let asked = QUESTION.is_match(&said);
        let addressed = wake.is_match(&plain) || (asked && YOU.is_match(&plain));
        if !addressed && (plain.split_whitespace().count() as f64) < num(&cfg, "min_words") && !asked {
            c.brain.remember("user", &said);
            continue;
        }
        let ago = if last_me > 0.0 { format!("{} s", (now() - last_me) as i64) } else { "noch nie".into() };
        let event = format!(
            "{said}\n[angesprochen: {}, du hast zuletzt vor {ago} geredet]",
            if addressed { "ja" } else { "nein" }
        );
        if c.respond(&event, addressed, t_last, asr_s) {
            last_me = now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heard_text_is_cleaned_and_judged_like_2x() {
        assert_eq!(clean("Untertitel im Auftrag des ZDF", "Discord, KI"), "");
        assert_eq!(clean("KI.", "Discord, KI"), "");
        assert_eq!(clean("a", "Discord, KI"), "");
        assert_eq!(clean("Wie geht's dir?", "Discord, KI"), "Wie geht's dir?");
        assert!(unfinished("und dann") && unfinished("weil ich dachte, und.") && !unfinished("Das war gut."));
        assert!(QUESTION.is_match("wie spät ist es") && !QUESTION.is_match("ich weiß nicht wie"));
        assert!(YOU.is_match("kannst du das") && !YOU.is_match("dudelsack"));
        let w = wav(&[0, 0, 1, 0]);
        assert_eq!((&w[..4], w.len(), &w[36..40]), (&b"RIFF"[..], 48, &b"data"[..]));
    }
}
