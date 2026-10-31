use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use serde_json::{Map, Value};

use super::audio::{self, BLOCK, Kind, Player, RATE};
use super::brain::now;
use super::mouth::Mouth;
use super::{Paths, num, text};

const DECIMATE: usize = 3;
const PRE_BLOCKS: usize = 8;
const FLOOR0: f32 = 0.004;
const TALK_S: f64 = 0.45;

pub type Utterance = (f64, Vec<u8>, bool);

pub struct Vad {
    pause: usize,
    longest: usize,
    barge: usize,
    min_speech: usize,
    talk_after: f64,
    level: f32,
    speech: Vec<Vec<f32>>,
    pre: Vec<Vec<f32>>,
    quiet: usize,
    floor: f32,
    loud_run: usize,
}

pub struct Heard {
    pub talking: bool,
    pub barge: bool,
    pub done: Option<Vec<u8>>,
}

impl Vad {
    pub fn new(cfg: &Map<String, Value>) -> Vad {
        let per_s = f64::from(RATE) / BLOCK as f64;
        let blocks = |k: &str| (num(cfg, k) * per_s) as usize;
        Vad {
            pause: blocks("end_pause_s"),
            longest: blocks("max_utterance_s"),
            barge: blocks("barge_in_s"),
            min_speech: blocks("min_speech_s"),
            talk_after: TALK_S * per_s,
            level: num(cfg, "level") as f32,
            speech: Vec::new(),
            pre: Vec::new(),
            quiet: 0,
            floor: FLOOR0,
            loud_run: 0,
        }
    }

    pub fn feed(&mut self, mono: &[f32]) -> Heard {
        let rms = (mono.iter().map(|x| x * x).sum::<f32>() / mono.len().max(1) as f32).sqrt();
        let loud = rms > self.level.max(self.floor * 3.0);
        if !loud && self.speech.is_empty() {
            self.floor = self.floor * 0.995 + rms * 0.005;
        }
        let strong = rms > (self.level * 1.8).max(self.floor * 4.0);
        self.loud_run = if strong { self.loud_run + 1 } else { self.loud_run.saturating_sub(2) };
        let talking = !self.speech.is_empty() && (self.speech.len() - self.pre.len()) as f64 > self.talk_after;
        let barge = self.loud_run == self.barge;
        let small: Vec<f32> = mono.iter().step_by(DECIMATE).copied().collect();
        if self.speech.is_empty() {
            self.pre.push(small.clone());
            let extra = self.pre.len().saturating_sub(PRE_BLOCKS);
            self.pre.drain(..extra);
        }
        if loud || !self.speech.is_empty() {
            if self.speech.is_empty() {
                self.speech.clone_from(&self.pre);
            }
            self.speech.push(small);
            self.quiet = if loud { 0 } else { self.quiet + 1 };
        }
        let mut done = None;
        if !self.speech.is_empty() && (self.quiet >= self.pause || self.speech.len() >= self.longest) {
            let voiced = self.speech.len().saturating_sub(self.quiet + self.pre.len());
            done = (voiced > self.min_speech).then(|| {
                self.speech
                    .iter()
                    .flatten()
                    .flat_map(|x| ((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
                    .collect::<Vec<u8>>()
            });
            self.speech.clear();
            self.quiet = 0;
        }
        Heard { talking, barge, done }
    }
}

pub struct Ear {
    pub cfg: Map<String, Value>,
    pub out: Sender<Utterance>,
    pub stop: Arc<AtomicBool>,
    pub mouth: Arc<Mouth>,
    pub talk: Arc<AtomicBool>,
    pub owner: bool,
}

impl Ear {
    pub fn run(self, p: &Paths) {
        let (kind, want) = if self.owner {
            (Kind::Mic, text(&self.cfg, "owner_mic"))
        } else {
            (Kind::Loopback, text(&self.cfg, "listen_device"))
        };
        let fwd_name = text(&self.cfg, "monitor_device");
        let fwd = if self.owner || fwd_name.is_empty() { None } else { Player::open(&fwd_name, 2).ok() };
        let mut vad = Vad::new(&self.cfg);
        let done = audio::record(&kind, &want, 2, |block| {
            if self.stop.load(Ordering::SeqCst) {
                return false;
            }
            if let Some(f) = &fwd {
                f.play(block);
            }
            let mono: Vec<f32> = block.as_chunks::<2>().0.iter().map(|[l, r]| (l + r) / 2.0).collect();
            let heard = vad.feed(&mono);
            self.talk.store(heard.talking, Ordering::SeqCst);
            if heard.barge && self.mouth.busy() {
                p.log("barge-in: someone talks, I stop");
                self.mouth.stop();
            }
            if let Some(pcm) = heard.done {
                let _ = self.out.send((now(), pcm, self.owner));
            }
            true
        });
        if let Err(e) = done {
            p.log(&format!("ears failed: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn speech_between_pauses_becomes_one_utterance() {
        let cfg = super::super::defaults();
        let mut vad = Vad::new(&cfg);
        let quiet = vec![0.0f32; BLOCK];
        let loud: Vec<f32> = (0..BLOCK).map(|i| if i % 2 == 0 { 0.2 } else { -0.2 }).collect();
        for _ in 0..20 {
            let h = vad.feed(&quiet);
            assert!(!h.talking && !h.barge && h.done.is_none());
        }
        let barges = (0..50).filter(|_| vad.feed(&loud).barge).count();
        assert_eq!(barges, 1);
        let mut got = None;
        for _ in 0..30 {
            if let Some(pcm) = vad.feed(&quiet).done {
                got = Some(pcm);
                break;
            }
        }
        let pcm = got.expect("an utterance after the pause");
        assert_eq!(pcm.len(), (8 + 50 + 25) * (BLOCK / 3) * 2);
        let mut short = Vad::new(
            &json!({"end_pause_s": 0.5, "max_utterance_s": 15, "barge_in_s": 0.8, "min_speech_s": 0.6, "level": 0.012})
                .as_object()
                .cloned()
                .unwrap(),
        );
        for _ in 0..10 {
            short.feed(&loud);
        }
        for _ in 0..30 {
            assert!(short.feed(&quiet).done.is_none());
        }
    }
}
