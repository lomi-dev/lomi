//! Gate over per-frame speech probabilities (the model runs elsewhere).

use std::collections::VecDeque;

pub const FRAME_SAMPLES: usize = 512;
const FRAME_MS: u32 = 32;

#[derive(Debug, Clone, Copy)]
pub struct VadParams {
    pub threshold: f32,
    pub pre_roll_ms: u32,
    pub onset_ms: u32,
    pub hangover_ms: u32,
}

impl Default for VadParams {
    fn default() -> Self {
        Self {
            threshold: 0.3,
            pre_roll_ms: 450,
            onset_ms: 60,
            hangover_ms: 450,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateEvent {
    Start,
    Speech(Vec<f32>),
    End,
}

pub struct VadGate {
    threshold: f32,
    onset: usize,
    hangover: usize,
    ring_cap: usize,
    ring: VecDeque<Vec<f32>>,
    in_speech: bool,
    voiced: usize,
    unvoiced: usize,
    speech_frames: u64,
}

fn frames(ms: u32) -> usize {
    (ms.div_ceil(FRAME_MS) as usize).max(1)
}

impl VadGate {
    pub fn new(p: VadParams) -> Self {
        let onset = frames(p.onset_ms);
        Self {
            threshold: p.threshold,
            onset,
            hangover: frames(p.hangover_ms),
            // The ring must hold the onset frames so they are never lost.
            ring_cap: frames(p.pre_roll_ms).max(onset),
            ring: VecDeque::new(),
            in_speech: false,
            voiced: 0,
            unvoiced: 0,
            speech_frames: 0,
        }
    }

    /// Frames whose length is not `FRAME_SAMPLES` are ignored.
    pub fn push(&mut self, frame: &[f32], prob: f32) -> Vec<GateEvent> {
        let mut ev = Vec::new();
        if frame.len() != FRAME_SAMPLES {
            return ev;
        }
        let voiced = prob > self.threshold;
        if self.in_speech {
            self.emit(frame.to_vec(), &mut ev);
            self.unvoiced = if voiced { 0 } else { self.unvoiced + 1 };
            if self.unvoiced >= self.hangover {
                ev.push(GateEvent::End);
                self.in_speech = false;
                self.voiced = 0;
            }
            return ev;
        }
        if self.ring.len() == self.ring_cap {
            self.ring.pop_front();
        }
        self.ring.push_back(frame.to_vec());
        self.voiced = if voiced { self.voiced + 1 } else { 0 };
        if self.voiced >= self.onset {
            ev.push(GateEvent::Start);
            self.flush_ring(&mut ev);
            self.in_speech = true;
            self.unvoiced = 0;
        }
        ev
    }

    /// Ends the stream: keeps a too-short utterance at the end, closes open speech.
    pub fn finish(&mut self) -> Vec<GateEvent> {
        let mut ev = Vec::new();
        if !self.in_speech && self.voiced > 0 {
            ev.push(GateEvent::Start);
            self.flush_ring(&mut ev);
            ev.push(GateEvent::End);
        } else if self.in_speech {
            ev.push(GateEvent::End);
        }
        self.ring.clear();
        self.in_speech = false;
        self.voiced = 0;
        ev
    }

    pub fn speech_ms(&self) -> u64 {
        self.speech_frames * u64::from(FRAME_MS)
    }

    fn emit(&mut self, pcm: Vec<f32>, ev: &mut Vec<GateEvent>) {
        self.speech_frames += 1;
        ev.push(GateEvent::Speech(pcm));
    }

    fn flush_ring(&mut self, ev: &mut Vec<GateEvent>) {
        self.speech_frames += self.ring.len() as u64;
        ev.push(GateEvent::Speech(self.ring.drain(..).flatten().collect()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds probabilities; returns (events as compact tags, speech frames).
    fn run(probs: &[f32], finish: bool) -> (Vec<String>, usize, VadGate) {
        let mut g = VadGate::new(VadParams::default());
        let mut tags = Vec::new();
        let mut total = 0;
        let frame = vec![0.0; FRAME_SAMPLES];
        let mut handle = |evs: Vec<GateEvent>, tags: &mut Vec<String>| {
            for e in evs {
                match e {
                    GateEvent::Start => tags.push("S".into()),
                    GateEvent::End => tags.push("E".into()),
                    GateEvent::Speech(p) => {
                        total += p.len() / FRAME_SAMPLES;
                        tags.push(format!("p{}", p.len() / FRAME_SAMPLES));
                    }
                }
            }
        };
        for &p in probs {
            handle(g.push(&frame, p), &mut tags);
        }
        if finish {
            handle(g.finish(), &mut tags);
        }
        (tags, total, g)
    }

    fn seq(parts: &[(f32, usize)]) -> Vec<f32> {
        parts.iter().flat_map(|&(p, n)| vec![p; n]).collect()
    }

    #[test]
    fn silence_emits_nothing() {
        let (tags, total, g) = run(&seq(&[(0.0, 100)]), true);
        assert!(tags.is_empty());
        assert_eq!(total, 0);
        assert_eq!(g.speech_ms(), 0);
    }

    #[test]
    fn burst_with_pre_roll_and_hangover() {
        // 20 silent, 10 voiced, 20 silent: pre-roll is 15 frames (incl. 2 onset).
        let (tags, total, g) = run(&seq(&[(0.0, 20), (0.9, 10), (0.0, 20)]), false);
        // Start at 2nd voiced frame: ring = 13 silent + 2 voiced.
        assert_eq!(tags[0], "S");
        assert_eq!(tags[1], "p15");
        // 8 remaining voiced + 15 hangover frames, then End.
        assert_eq!(tags.iter().filter(|t| *t == "p1").count(), 8 + 15);
        assert_eq!(tags.last().unwrap(), "E");
        assert_eq!(total, 15 + 8 + 15);
        assert_eq!(g.speech_ms(), 38 * 32);
    }

    #[test]
    fn short_gap_keeps_speech_open() {
        let (tags, _, _) = run(&seq(&[(0.9, 5), (0.0, 14), (0.9, 5), (0.0, 3)]), false);
        assert!(!tags.contains(&"E".to_string()));
        assert_eq!(tags.iter().filter(|t| *t == "S").count(), 1);
    }

    #[test]
    fn single_frame_blip_is_dropped() {
        let (tags, total, _) = run(
            &seq(&[(0.0, 5), (0.9, 1), (0.0, 5), (0.9, 1), (0.0, 5)]),
            false,
        );
        assert!(tags.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn finish_keeps_short_tail() {
        let probs = seq(&[(0.0, 3), (0.9, 1)]);
        let (tags, total, g) = run(&probs, false);
        assert!(tags.is_empty());
        let _ = (total, g);
        let (tags, total, g) = run(&probs, true);
        assert_eq!(tags, ["S", "p4", "E"]);
        assert_eq!(total, 4);
        assert_eq!(g.speech_ms(), 4 * 32);
    }

    #[test]
    fn finish_closes_open_speech_and_ignores_bad_frames() {
        let (tags, _, mut g) = run(&seq(&[(0.9, 3)]), true);
        assert_eq!(tags.last().unwrap(), "E");
        assert!(g.push(&[0.0; 10], 1.0).is_empty());
    }
}
