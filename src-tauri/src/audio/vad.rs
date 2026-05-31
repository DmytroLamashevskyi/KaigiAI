//! Energy-based voice activity detection.
//!
//! A frame-based state machine that turns a stream of mono 16 kHz `f32` samples
//! into discrete speech segments. Deliberately free of cpal/Tauri so it can be
//! unit-tested with synthetic signals and reused by a future server.

use std::collections::VecDeque;

/// Sample rate the rest of the pipeline (whisper) expects.
pub const SAMPLE_RATE: u32 = 16_000;

const FRAME_MS: usize = 20;
/// Samples per analysis frame at [`SAMPLE_RATE`] (20 ms = 320 samples).
pub const FRAME_SAMPLES: usize = SAMPLE_RATE as usize * FRAME_MS / 1000;

#[derive(Clone, Debug)]
pub struct VadConfig {
    /// Absolute RMS floor below which audio is always treated as silence. The
    /// effective threshold is the max of this and an adaptive noise floor, so
    /// both quiet and noisy rooms behave.
    pub energy_threshold: f32,
    /// Consecutive voiced frames required to open a segment (debounces clicks).
    pub start_frames: usize,
    /// Consecutive silence frames required to close a segment (the hangover
    /// that keeps natural pauses inside one utterance).
    pub end_frames: usize,
    /// Samples of recent audio prepended before the trigger so word onsets are
    /// not clipped.
    pub preroll_samples: usize,
    /// Hard cap on a single segment; forces a flush during long monologues.
    pub max_segment_samples: usize,
    /// Segments shorter than this (after pre-roll) are discarded as blips.
    pub min_segment_samples: usize,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 0.012,
            start_frames: 2,                                  // ~40 ms
            end_frames: 30,                                   // ~600 ms hangover
            preroll_samples: FRAME_SAMPLES * 5,               // ~100 ms
            max_segment_samples: SAMPLE_RATE as usize * 25,   // 25 s
            min_segment_samples: SAMPLE_RATE as usize / 4,    // 250 ms
        }
    }
}

enum State {
    Silence,
    Speech,
}

pub struct Vad {
    cfg: VadConfig,
    state: State,
    partial: Vec<f32>,
    preroll: VecDeque<f32>,
    segment: Vec<f32>,
    voiced_run: usize,
    silence_run: usize,
    noise_floor: f32,
}

impl Vad {
    pub fn new(cfg: VadConfig) -> Self {
        Self {
            partial: Vec::with_capacity(FRAME_SAMPLES),
            preroll: VecDeque::with_capacity(cfg.preroll_samples + FRAME_SAMPLES),
            segment: Vec::new(),
            state: State::Silence,
            voiced_run: 0,
            silence_run: 0,
            noise_floor: 0.0,
            cfg,
        }
    }

    /// Feed mono 16 kHz samples. Returns any speech segments that completed
    /// within this call (usually zero or one).
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        let mut out = Vec::new();
        for &s in samples {
            self.partial.push(s);
            if self.partial.len() == FRAME_SAMPLES {
                let frame = std::mem::replace(&mut self.partial, Vec::with_capacity(FRAME_SAMPLES));
                self.process_frame(&frame, &mut out);
            }
        }
        out
    }

    /// Emit any in-progress segment. Call when capture stops so a final
    /// utterance that never saw trailing silence is not lost.
    pub fn flush(&mut self) -> Option<Vec<f32>> {
        if !self.partial.is_empty() {
            if let State::Speech = self.state {
                let tail = std::mem::take(&mut self.partial);
                self.segment.extend_from_slice(&tail);
            } else {
                self.partial.clear();
            }
        }
        if let State::Speech = self.state {
            self.state = State::Silence;
            let seg = std::mem::take(&mut self.segment);
            self.reset_runs();
            if seg.len() >= self.cfg.min_segment_samples {
                return Some(seg);
            }
        }
        None
    }

    fn process_frame(&mut self, frame: &[f32], out: &mut Vec<Vec<f32>>) {
        let rms = rms(frame);
        let thresh = self.cfg.energy_threshold.max(self.noise_floor * 3.0);
        let voiced = rms > thresh;

        match self.state {
            State::Silence => {
                self.push_preroll(frame);
                // Track the noise floor only while idle so speech can't inflate it.
                self.noise_floor = 0.97 * self.noise_floor + 0.03 * rms;
                if voiced {
                    self.voiced_run += 1;
                    if self.voiced_run >= self.cfg.start_frames {
                        self.state = State::Speech;
                        self.segment.clear();
                        self.segment.extend(self.preroll.iter().copied());
                        self.preroll.clear();
                        self.silence_run = 0;
                        self.voiced_run = 0;
                    }
                } else {
                    self.voiced_run = 0;
                }
            }
            State::Speech => {
                self.segment.extend_from_slice(frame);
                if voiced {
                    self.silence_run = 0;
                } else {
                    self.silence_run += 1;
                    if self.silence_run >= self.cfg.end_frames {
                        self.close_segment(out);
                        return;
                    }
                }
                if self.segment.len() >= self.cfg.max_segment_samples {
                    self.close_segment(out);
                }
            }
        }
    }

    fn close_segment(&mut self, out: &mut Vec<Vec<f32>>) {
        let seg = std::mem::take(&mut self.segment);
        self.state = State::Silence;
        self.reset_runs();
        if seg.len() >= self.cfg.min_segment_samples {
            out.push(seg);
        }
    }

    fn reset_runs(&mut self) {
        self.voiced_run = 0;
        self.silence_run = 0;
        self.preroll.clear();
    }

    fn push_preroll(&mut self, frame: &[f32]) {
        for &s in frame {
            if self.preroll.len() == self.cfg.preroll_samples {
                self.preroll.pop_front();
            }
            self.preroll.push_back(s);
        }
    }
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = frame.iter().map(|s| s * s).sum();
    (sum_sq / frame.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, freq: f32, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    #[test]
    fn detects_single_utterance_between_silence() {
        let mut vad = Vad::new(VadConfig::default());
        let mut segments = Vec::new();

        // 1 s silence, 1 s tone, 1 s silence.
        segments.extend(vad.push(&vec![0.0f32; SAMPLE_RATE as usize]));
        segments.extend(vad.push(&sine(SAMPLE_RATE as usize, 220.0, 0.3)));
        segments.extend(vad.push(&vec![0.0f32; SAMPLE_RATE as usize]));
        if let Some(tail) = vad.flush() {
            segments.push(tail);
        }

        assert_eq!(segments.len(), 1, "expected exactly one speech segment");
        let len = segments[0].len();
        // ~1 s of speech plus pre-roll, well under 2 s.
        assert!(
            len > SAMPLE_RATE as usize / 2 && len < SAMPLE_RATE as usize * 2,
            "segment length {len} out of expected range"
        );
    }

    #[test]
    fn pure_silence_yields_nothing() {
        let mut vad = Vad::new(VadConfig::default());
        let out = vad.push(&vec![0.0f32; SAMPLE_RATE as usize * 3]);
        assert!(out.is_empty());
        assert!(vad.flush().is_none());
    }

    #[test]
    fn long_speech_is_split_at_cap() {
        let cfg = VadConfig {
            max_segment_samples: SAMPLE_RATE as usize, // 1 s cap
            ..VadConfig::default()
        };
        let mut vad = Vad::new(cfg);
        // 3 s of continuous tone should split into ~3 capped segments.
        let mut segments = vad.push(&sine(SAMPLE_RATE as usize * 3, 220.0, 0.3));
        if let Some(tail) = vad.flush() {
            segments.push(tail);
        }
        assert!(segments.len() >= 2, "expected long speech to be split, got {}", segments.len());
    }
}
