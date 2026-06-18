//! Energy-based voice activity detection.
//!
//! A frame-based state machine that turns a stream of mono 16 kHz `f32` samples
//! into discrete speech segments. Deliberately free of cpal/Tauri so it can be
//! unit-tested with synthetic signals and reused by a future server.

use std::collections::VecDeque;

/// Sample rate the rest of the pipeline (whisper) expects.
pub const SAMPLE_RATE: u32 = 16_000;

/// Milliseconds of audio per analysis frame. Public so the recording layer can
/// translate the user's silence setting (ms) into [`VadConfig::end_frames`].
pub const FRAME_MS: usize = 20;
/// Samples per analysis frame at [`SAMPLE_RATE`] (20 ms = 320 samples).
pub const FRAME_SAMPLES: usize = SAMPLE_RATE as usize * FRAME_MS / 1000;

/// What the VAD reports as it consumes audio. Besides finalized utterances it
/// announces pauses so the UI can show a live "silence countdown" while the
/// speaker might still continue (docs/PROJECT.md §10.8).
#[derive(Clone, Debug, PartialEq)]
pub enum VadEvent {
    /// Speech just paused. The UI can fill a bar over `hangover_ms`; if the
    /// speaker resumes before it elapses the pause is aborted, otherwise the
    /// utterance finalizes into a [`VadEvent::Segment`].
    SilenceStarted { hangover_ms: u32 },
    /// A pending pause ended without finalizing — speech resumed, or the segment
    /// was too short and got discarded. The UI should drop its placeholder.
    PendingAborted,
    /// A finalized utterance (mono 16 kHz), ready for STT.
    Segment(Vec<f32>),
}

#[derive(Clone, Debug)]
pub struct VadConfig {
    /// Absolute RMS floor below which audio is always treated as silence. The
    /// effective threshold is the max of this and an adaptive noise floor, so
    /// both quiet and noisy rooms behave.
    pub energy_threshold: f32,
    /// Consecutive voiced frames required to open a segment (debounces clicks).
    pub start_frames: usize,
    /// Consecutive silence frames required to close a segment (the total
    /// hangover that keeps natural pauses inside one utterance).
    pub end_frames: usize,
    /// Consecutive silence frames before the pause is *announced* to the UI
    /// (the [`VadEvent::SilenceStarted`] countdown bar). A grace window: shorter
    /// pauses inside a sentence pass without ever flashing a bar, so the
    /// countdown only appears once the speaker has plausibly finished. Must be
    /// `< end_frames`; the bar then counts down the remaining
    /// `end_frames - reveal_frames` frames.
    pub reveal_frames: usize,
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
            // Total hangover before an utterance finalizes. Overridden
            // per-session from the user's "silence" setting (recording.rs) as
            // reveal_frames + the configured bar countdown; the trailing
            // hangover silence is trimmed before STT so the long wait doesn't
            // bloat the audio.
            end_frames: 225,                                  // 1.5 s grace + 3 s bar
            // ~1.5 s of silence must pass before the countdown bar appears, so
            // ordinary mid-sentence pauses never flash it.
            reveal_frames: 75,                                // ~1.5 s
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
    /// Whether the current pause has been announced (a [`VadEvent::SilenceStarted`]
    /// bar is on screen), so we know to abort it if speech resumes or the segment
    /// is discarded.
    pause_announced: bool,
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
            pause_announced: false,
            noise_floor: 0.0,
            cfg,
        }
    }

    /// Feed mono 16 kHz samples. Returns any events (pause start/abort, finalized
    /// segments) that occurred within this call.
    pub fn push(&mut self, samples: &[f32]) -> Vec<VadEvent> {
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

    /// Finalize any in-progress segment. Call when capture stops so a final
    /// utterance that never saw trailing silence is not lost. Mirrors
    /// [`Self::close_segment`]: emits the [`VadEvent::Segment`] if long enough, or
    /// a [`VadEvent::PendingAborted`] if a countdown bar was showing but the
    /// segment is discarded — so the UI placeholder is never left orphaned.
    pub fn flush(&mut self) -> Vec<VadEvent> {
        let mut out = Vec::new();
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
            // Drop the trailing hangover silence (see close_segment).
            let trim = self.silence_run.saturating_mul(FRAME_SAMPLES);
            let keep = self.segment.len().saturating_sub(trim);
            self.segment.truncate(keep);
            let announced = self.pause_announced;
            let seg = std::mem::take(&mut self.segment);
            self.reset_runs();
            if seg.len() >= self.cfg.min_segment_samples {
                out.push(VadEvent::Segment(seg));
            } else if announced {
                out.push(VadEvent::PendingAborted);
            }
        }
        out
    }

    fn process_frame(&mut self, frame: &[f32], out: &mut Vec<VadEvent>) {
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
                    // Speech resumed. If a countdown bar was already showing,
                    // abort it; pauses still inside the grace window never showed
                    // one, so nothing to drop.
                    if self.pause_announced {
                        out.push(VadEvent::PendingAborted);
                        self.pause_announced = false;
                    }
                    self.silence_run = 0;
                } else {
                    self.silence_run += 1;
                    // Past the grace window: announce the countdown bar, which
                    // fills over the remaining frames until the segment closes.
                    if self.silence_run == self.cfg.reveal_frames
                        && self.cfg.reveal_frames < self.cfg.end_frames
                    {
                        let remaining = self.cfg.end_frames - self.cfg.reveal_frames;
                        let hangover_ms = (remaining * FRAME_MS) as u32;
                        out.push(VadEvent::SilenceStarted { hangover_ms });
                        self.pause_announced = true;
                    }
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

    fn close_segment(&mut self, out: &mut Vec<VadEvent>) {
        // Trim the trailing hangover silence we accumulated while waiting: it's
        // pure silence and, with a multi-second hangover, would otherwise bloat
        // the STT input and invite hallucinated tokens.
        let trim = self.silence_run.saturating_mul(FRAME_SAMPLES);
        let keep = self.segment.len().saturating_sub(trim);
        self.segment.truncate(keep);
        // Whether we already announced a pause for this segment (so a discard
        // must clear that placeholder).
        let announced = self.pause_announced;
        let seg = std::mem::take(&mut self.segment);
        self.state = State::Silence;
        self.reset_runs();
        if seg.len() >= self.cfg.min_segment_samples {
            out.push(VadEvent::Segment(seg));
        } else if announced {
            out.push(VadEvent::PendingAborted);
        }
    }

    fn reset_runs(&mut self) {
        self.voiced_run = 0;
        self.silence_run = 0;
        self.pause_announced = false;
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

    /// Keep only finalized segments from a batch of events (the tests care about
    /// utterance boundaries, not the live pause/abort signalling).
    fn segs(events: Vec<VadEvent>) -> Vec<Vec<f32>> {
        events
            .into_iter()
            .filter_map(|e| match e {
                VadEvent::Segment(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn detects_single_utterance_between_silence() {
        let mut vad = Vad::new(VadConfig::default());
        let mut segments = Vec::new();

        // 1 s silence, 1 s tone, 1 s silence.
        segments.extend(segs(vad.push(&vec![0.0f32; SAMPLE_RATE as usize])));
        segments.extend(segs(vad.push(&sine(SAMPLE_RATE as usize, 220.0, 0.3))));
        segments.extend(segs(vad.push(&vec![0.0f32; SAMPLE_RATE as usize])));
        segments.extend(segs(vad.flush()));

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
        assert!(vad.flush().is_empty());
    }

    #[test]
    fn short_pause_inside_speech_shows_no_bar() {
        // A sub-grace pause (here ~0.4 s) between two bursts must NOT flash a
        // countdown bar — no SilenceStarted/PendingAborted, and it stays one
        // segment. Guards the anti-flicker grace window.
        let mut vad = Vad::new(VadConfig::default()); // reveal ~1.5 s
        let mut events = Vec::new();
        events.extend(vad.push(&sine(SAMPLE_RATE as usize, 220.0, 0.3)));
        events.extend(vad.push(&vec![0.0f32; SAMPLE_RATE as usize * 2 / 5])); // 0.4 s gap
        events.extend(vad.push(&sine(SAMPLE_RATE as usize, 220.0, 0.3)));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, VadEvent::SilenceStarted { .. } | VadEvent::PendingAborted)),
            "a short pause must not announce or abort a countdown bar"
        );
        assert!(segs(events).is_empty(), "speech must not finalize across a short pause");
    }

    #[test]
    fn long_pause_announces_then_finalizes() {
        // reveal=2 frames (40 ms), close at 5 frames (100 ms): a pause past the
        // grace window announces a bar, then finalizes into one segment.
        let cfg = VadConfig {
            reveal_frames: 2,
            end_frames: 5,
            ..VadConfig::default()
        };
        let mut vad = Vad::new(cfg);
        let mut events = Vec::new();
        events.extend(vad.push(&sine(SAMPLE_RATE as usize / 2, 220.0, 0.3))); // 0.5 s speech
        events.extend(vad.push(&vec![0.0f32; SAMPLE_RATE as usize])); // 1 s silence > close
        let started = events
            .iter()
            .filter(|e| matches!(e, VadEvent::SilenceStarted { .. }))
            .count();
        assert_eq!(started, 1, "exactly one countdown bar announced");
        assert_eq!(segs(events).len(), 1, "pause past grace must finalize the utterance");
    }

    #[test]
    fn flush_aborts_announced_pause_when_segment_discarded() {
        // Stop recording mid-pause with too little speech to keep: flush must emit
        // PendingAborted (not a Segment) so the on-screen countdown bar is cleared.
        let cfg = VadConfig {
            reveal_frames: 2,
            end_frames: 50,
            min_segment_samples: SAMPLE_RATE as usize, // require ~1 s; our speech is short
            ..VadConfig::default()
        };
        let mut vad = Vad::new(cfg);
        let _ = vad.push(&sine(SAMPLE_RATE as usize / 10, 220.0, 0.3)); // ~0.1 s speech
        let ev = vad.push(&vec![0.0f32; FRAME_SAMPLES * 3]); // 3 silent frames > reveal
        assert!(
            ev.iter().any(|e| matches!(e, VadEvent::SilenceStarted { .. })),
            "the pause should have been announced"
        );
        let flushed = vad.flush();
        assert!(
            flushed.iter().any(|e| matches!(e, VadEvent::PendingAborted)),
            "flush must abort the announced pause when the segment is discarded"
        );
        assert!(segs(flushed).is_empty(), "the too-short segment must not be emitted");
    }

    #[test]
    fn long_speech_is_split_at_cap() {
        let cfg = VadConfig {
            max_segment_samples: SAMPLE_RATE as usize, // 1 s cap
            ..VadConfig::default()
        };
        let mut vad = Vad::new(cfg);
        // 3 s of continuous tone should split into ~3 capped segments.
        let mut segments = segs(vad.push(&sine(SAMPLE_RATE as usize * 3, 220.0, 0.3)));
        segments.extend(segs(vad.flush()));
        assert!(segments.len() >= 2, "expected long speech to be split, got {}", segments.len());
    }
}
