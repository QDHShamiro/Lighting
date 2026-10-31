use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use regex::Regex;
use serde_json::{Map, Value, json};
use wisply_core::Fail;

use super::audio::{Player, RATE};
use super::brain::now;
use super::{Paths, text};

const NO_WINDOW: u32 = 0x0800_0000;
const STEP: usize = RATE as usize / 20;
const CACHE_CHARS: usize = 45;
const RECENT: usize = 6;
const WARM: &[&str] = &["Mhm.", "Ja safe."];
const SETTLE: Duration = Duration::from_millis(100);
const ERR_CHARS: usize = 120;

static WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\W+").expect("words"));

type Blocks = Vec<Vec<f32>>;

enum Pcm {
    Block(Vec<f32>),
    End,
}

pub fn tts_exe(home: &Path) -> Result<PathBuf, Fail> {
    let venv = home.join("venv").join("Scripts").join("edge-tts.exe");
    crate::daemon::which("edge-tts").or_else(|| venv.is_file().then_some(venv)).ok_or_else(|| {
        Fail::hint(
            "voice speaks through edge-tts, which is not installed",
            "pipx install edge-tts (or pip install edge-tts)",
        )
    })
}

#[derive(Clone)]
struct Tools {
    tts: PathBuf,
    ffmpeg: PathBuf,
    voice: String,
    rate: String,
}

struct Pair {
    tts: Child,
    ff: Child,
}

impl Tools {
    fn spawn(&self) -> Option<Pair> {
        let mut tts = Command::new(&self.tts)
            .args(["-f", "-", "--voice", &self.voice, &format!("--rate={}", self.rate), "--write-media", "-"])
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(NO_WINDOW)
            .spawn()
            .ok()?;
        let mp3 = tts.stdout.take()?;
        let rate = RATE.to_string();
        let ff = Command::new(&self.ffmpeg)
            .args(["-v", "quiet", "-probesize", "32", "-analyzeduration", "0", "-f", "mp3", "-i", "pipe:0"])
            .args(["-f", "f32le", "-ac", "1", "-ar", &rate, "pipe:1"])
            .stdin(Stdio::from(mp3))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(NO_WINDOW)
            .spawn();
        match ff {
            Ok(ff) => Some(Pair { tts, ff }),
            Err(_) => {
                let _ = tts.kill();
                None
            }
        }
    }
}

impl Pair {
    fn stop(mut self) {
        let _ = self.tts.kill();
        let _ = self.ff.kill();
        let _ = self.tts.wait();
        let _ = self.ff.wait();
    }
}

pub struct Mouth {
    lines: Sender<String>,
    cut: Arc<AtomicBool>,
    speaking: Arc<AtomicBool>,
    pending: Arc<AtomicI64>,
    epoch: Arc<AtomicU64>,
    pub first_audio: Arc<Mutex<Option<f64>>>,
    recent: Mutex<Vec<String>>,
    p: Paths,
}

struct Voice {
    tools: Tools,
    spare: Option<Pair>,
    cache: Arc<Mutex<HashMap<String, Blocks>>>,
    cut: Arc<AtomicBool>,
    p: Paths,
}

impl Voice {
    fn synth(&mut self, said: &str, out: &Sender<Pcm>, mut keep: Option<&mut Blocks>) -> Result<(), String> {
        let mut pair = match self.spare.take() {
            Some(p) => p,
            None => self.tools.spawn().ok_or("could not start edge-tts or ffmpeg")?,
        };
        let mut stdin = pair.tts.stdin.take().ok_or("edge-tts has no input")?;
        let wrote = stdin.write_all(said.as_bytes());
        drop(stdin);
        let mut pcm = pair.ff.stdout.take().ok_or("ffmpeg has no output")?;
        self.spare = self.tools.spawn();
        wrote.map_err(|e| e.to_string())?;
        let mut raw = vec![0u8; STEP * 4];
        let mut got_any = false;
        loop {
            if self.cut.load(Ordering::SeqCst) {
                break;
            }
            let mut filled = 0;
            while filled < raw.len() {
                match pcm.read(&mut raw[filled..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => filled += n,
                }
            }
            if filled < 4 {
                break;
            }
            let block: Vec<f32> =
                raw[..filled / 4 * 4].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
            got_any = true;
            if let Some(k) = keep.as_deref_mut() {
                k.push(block.clone());
            }
            let _ = out.send(Pcm::Block(block));
            if filled < raw.len() {
                break;
            }
        }
        pair.stop();
        if got_any || self.cut.load(Ordering::SeqCst) { Ok(()) } else { Err("edge-tts gave no audio".into()) }
    }

    fn warm(mut self, filler: &str) {
        for t in [filler].into_iter().chain(WARM.iter().copied()) {
            let mut keep = Vec::new();
            let (sink, _drain) = mpsc::channel();
            match self.synth(t, &sink, Some(&mut keep)) {
                Ok(()) => {
                    self.cache.lock().unwrap_or_else(PoisonError::into_inner).insert(t.to_string(), keep);
                }
                Err(e) => self.p.log(&format!("tts warmup failed: {}", e.chars().take(ERR_CHARS).collect::<String>())),
            }
        }
        if let Some(pair) = self.spare.take() {
            pair.stop();
        }
    }

    fn run(mut self, lines: &Receiver<String>, pcm: &Sender<Pcm>) {
        while let Ok(said) = lines.recv() {
            let cached = self.cache.lock().unwrap_or_else(PoisonError::into_inner).get(&said).cloned();
            if self.cut.load(Ordering::SeqCst) {
                let _ = pcm.send(Pcm::End);
                continue;
            }
            if let Some(blocks) = cached {
                for b in blocks {
                    let _ = pcm.send(Pcm::Block(b));
                }
            } else {
                let mut keep = (said.chars().count() <= CACHE_CHARS).then(Vec::new);
                match self.synth(&said, pcm, keep.as_mut()) {
                    Ok(()) => {
                        if let Some(k) = keep.filter(|k| !k.is_empty() && !self.cut.load(Ordering::SeqCst)) {
                            self.cache.lock().unwrap_or_else(PoisonError::into_inner).insert(said, k);
                        }
                    }
                    Err(e) => self.p.log(&format!("tts failed: {}", e.chars().take(ERR_CHARS).collect::<String>())),
                }
            }
            let _ = pcm.send(Pcm::End);
        }
    }
}

fn player(want: String, rx: Receiver<(u64, Vec<f32>)>, epoch: Arc<AtomicU64>, ready: Sender<Result<(), String>>) {
    let dev = match Player::open(&want, 1) {
        Ok(d) => {
            let _ = ready.send(Ok(()));
            d
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    while let Ok((tag, block)) = rx.recv() {
        if tag == epoch.load(Ordering::SeqCst) {
            dev.play(&block);
        }
    }
}

impl Mouth {
    pub fn new(cfg: &Map<String, Value>, p: &Paths) -> Result<Arc<Mouth>, String> {
        let tools = Tools {
            tts: tts_exe(&p.home).map_err(|_| "edge-tts is not installed".to_string())?,
            ffmpeg: crate::daemon::ffmpeg(&p.home).ok_or("ffmpeg is not installed")?,
            voice: text(cfg, "voice"),
            rate: text(cfg, "rate"),
        };
        let epoch = Arc::new(AtomicU64::new(0));
        let mut outs = Vec::new();
        for want in [text(cfg, "speak_device"), text(cfg, "monitor_device")].into_iter().filter(|w| !w.is_empty()) {
            let (tx, rx) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::channel();
            let e = Arc::clone(&epoch);
            thread::spawn(move || player(want, rx, e, ready_tx));
            ready_rx.recv().map_err(|e| e.to_string())??;
            outs.push(tx);
        }
        let (lines_tx, lines_rx) = mpsc::channel::<String>();
        let (pcm_tx, pcm_rx) = mpsc::channel::<Pcm>();
        let mouth = Arc::new(Mouth {
            lines: lines_tx,
            cut: Arc::new(AtomicBool::new(false)),
            speaking: Arc::new(AtomicBool::new(false)),
            pending: Arc::new(AtomicI64::new(0)),
            epoch,
            first_audio: Arc::new(Mutex::new(None)),
            recent: Mutex::new(Vec::new()),
            p: Paths::new(&p.home),
        });
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let warm = Voice {
            tools: tools.clone(),
            spare: None,
            cache: Arc::clone(&cache),
            cut: Arc::new(AtomicBool::new(false)),
            p: Paths::new(&p.home),
        };
        let filler = text(cfg, "search_filler");
        thread::spawn(move || warm.warm(&filler));
        let voice = Voice { tools, spare: None, cache, cut: Arc::clone(&mouth.cut), p: Paths::new(&p.home) };
        thread::spawn(move || voice.run(&lines_rx, &pcm_tx));
        let m = Arc::clone(&mouth);
        thread::spawn(move || m.playback(&pcm_rx, &outs));
        Ok(mouth)
    }

    fn playback(&self, pcm: &Receiver<Pcm>, outs: &[Sender<(u64, Vec<f32>)>]) {
        while let Ok(item) = pcm.recv() {
            match item {
                Pcm::End => {
                    if self.pending.fetch_sub(1, Ordering::SeqCst) <= 1 {
                        thread::sleep(SETTLE);
                        self.speaking.store(false, Ordering::SeqCst);
                        self.cut.store(false, Ordering::SeqCst);
                    }
                }
                Pcm::Block(_) if self.cut.load(Ordering::SeqCst) => {}
                Pcm::Block(block) => {
                    self.speaking.store(true, Ordering::SeqCst);
                    let mut first = self.first_audio.lock().unwrap_or_else(PoisonError::into_inner);
                    if first.is_none() {
                        *first = Some(now());
                    }
                    drop(first);
                    let tag = self.epoch.load(Ordering::SeqCst);
                    for o in outs {
                        let _ = o.send((tag, block.clone()));
                    }
                    thread::sleep(Duration::from_secs_f64(block.len() as f64 / f64::from(RATE) * 0.9));
                }
            }
        }
    }

    pub fn say(&self, said: &str) {
        let key = WORDS.replace_all(said, " ").trim().to_lowercase();
        {
            let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
            if !key.is_empty() && recent.contains(&key) {
                self.p.log(&format!("skipped repeat: {said}"));
                return;
            }
            if !key.is_empty() {
                recent.push(key);
                let extra = recent.len().saturating_sub(RECENT);
                recent.drain(..extra);
            }
        }
        if !said.is_empty() && said.trim() != "-" {
            self.p.log(&format!("say: {said}"));
            self.p.state(json!({"last_said": said}));
            self.pending.fetch_add(1, Ordering::SeqCst);
            let _ = self.lines.send(said.to_string());
        }
    }

    pub fn stop(&self) {
        if self.busy() {
            self.cut.store(true, Ordering::SeqCst);
            self.epoch.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn busy(&self) -> bool {
        self.speaking.load(Ordering::SeqCst) || self.pending.load(Ordering::SeqCst) > 0
    }

    pub fn reset_first(&self) {
        *self.first_audio.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    pub fn first(&self) -> Option<f64> {
        *self.first_audio.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
