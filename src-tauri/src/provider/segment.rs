//! Speaker-turn segmentation (docs/PROJECT.md §10.15).
//!
//! Detects WITHIN one VAD segment the points where the active speaker changes
//! (back-to-back turns with no pause, which pause-based VAD cannot split), so
//! the pipeline can cut the segment and run STT / language detection /
//! translation / diarization per part.
//!
//! Model: pyannote **segmentation-3.0** (the sherpa-onnx ONNX build, MIT).
//! Input: 10 s windows of 16 kHz mono PCM, shape `(1, 1, samples)`. Output:
//! `(1, frames, 7)` powerset activations — up to three window-local speakers
//! including overlaps. Local speaker identities are NOT stable across windows
//! (and are never used as identities here): this module only extracts *change
//! points*; who each part belongs to is decided downstream by the wespeaker
//! diarizer (§10.6).

use ort::session::Session;
use ort::value::Tensor;

/// Powerset decoding of segmentation-3.0's 7 output classes: the set of active
/// window-local speakers per class index, in pyannote's subset order.
const POWERSET: [&[u8]; 7] = [&[], &[0], &[1], &[2], &[0, 1], &[0, 2], &[1, 2]];

/// Analysis window: the model is trained on 10-second chunks.
const WINDOW_SECS: usize = 10;

/// Anti-flicker: a single-speaker run must last at least this long to count as
/// a stable turn (shorter blips are decoder noise, not real turns). 0.7 s
/// mirrors the diarizer's MIN_NEW_SPEAKER_SECS: pitch swings and emphatic
/// speech make the model hallucinate brief second speakers on a single voice,
/// and every false turn cuts a phrase mid-word.
const MIN_RUN_SECS: f32 = 0.7;

/// A cut must leave at least this much audio on each side. Whisper needs ~2 s
/// of context for reliable decoding — shorter parts transcribe noticeably
/// worse — and a false cut producing a scrap this small has no value anyway.
const MIN_PART_SECS: f32 = 2.0;

/// Upper bound on cuts per VAD segment (audio::vad caps a segment at 25 s —
/// `max_segment_samples`; more cuts than this means the decoder is flickering,
/// not that ten people spoke).
const MAX_CUTS: usize = 4;

pub struct OnnxSegmenter {
    session: Session,
    sample_rate: u32,
}

impl OnnxSegmenter {
    pub fn new(model_path: &str, sample_rate: u32) -> Result<Self, String> {
        // The model is trained on 16 kHz mono ONLY. Other rates wouldn't error —
        // they'd silently return garbage activations (pitch-shifted audio), so
        // fail loudly at load time instead (the factory logs + disables).
        if sample_rate != 16_000 {
            return Err(format!(
                "segmentation-3.0 expects 16 kHz mono input, got {sample_rate} Hz"
            ));
        }
        let session = Session::builder()
            .map_err(|e| format!("ort init: {e}"))?
            .commit_from_file(model_path)
            .map_err(|e| format!("load segmentation model: {e}"))?;
        Ok(Self {
            session,
            sample_rate,
        })
    }

    /// Sample offsets (ascending, exclusive of 0 and `pcm.len()`) where the
    /// active speaker changes. Empty = keep the segment whole (single speaker,
    /// too short, or inference failed — all degrade to the previous behavior).
    pub fn change_points(&mut self, pcm: &[f32]) -> Vec<usize> {
        let window = WINDOW_SECS * self.sample_rate as usize;
        let min_part = (MIN_PART_SECS * self.sample_rate as f32) as usize;
        if pcm.len() < 2 * min_part {
            return Vec::new();
        }
        let mut cuts: Vec<usize> = Vec::new();
        let mut start = 0usize;
        loop {
            let end = (start + window).min(pcm.len());
            match self.window_classes(&pcm[start..end], window) {
                Ok((classes, samples_per_frame)) => {
                    // Frames past the real audio (zero padding) decode as
                    // silence and produce no runs, so no trimming is needed.
                    let min_run =
                        ((MIN_RUN_SECS * self.sample_rate as f32) as usize / samples_per_frame).max(1);
                    for frame in cuts_from_classes(&classes, min_run) {
                        cuts.push(start + frame * samples_per_frame);
                    }
                }
                Err(e) => {
                    log::warn!("speaker segmentation skipped: {e}");
                    return Vec::new();
                }
            }
            if end == pcm.len() {
                break;
            }
            // Half-window overlap: a change straddling a window seam (or inside
            // the anti-flicker dead band at a window's edge) is well interior to
            // the neighboring window, so it can't be systematically missed. The
            // near-duplicate estimates from overlapping windows collapse in the
            // spacing filter below.
            start += window / 2;
        }
        cuts.sort_unstable();
        filter_cuts(cuts, min_part, pcm.len())
    }

    /// Run the model on one window (zero-padded to `window` samples) and return
    /// the per-frame argmax class plus the samples-per-frame ratio.
    fn window_classes(
        &mut self,
        chunk: &[f32],
        window: usize,
    ) -> Result<(Vec<usize>, usize), String> {
        let mut padded = chunk.to_vec();
        padded.resize(window, 0.0);
        let input = Tensor::from_array(([1usize, 1usize, window], padded))
            .map_err(|e| format!("build input tensor: {e}"))?;
        let input_name = self.session.inputs[0].name.clone();
        let inputs = ort::inputs![input_name.as_str() => input]
            .map_err(|e| format!("build inputs: {e}"))?;
        let outputs = self
            .session
            .run(inputs)
            .map_err(|e| format!("segmentation inference: {e}"))?;
        let arr = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("extract activations: {e}"))?;
        let shape = arr.shape().to_vec();
        // Expect (1, frames, 7); tolerate (frames, 7).
        let (frames, classes_n) = match shape.as_slice() {
            [1, f, c] => (*f, *c),
            [f, c] => (*f, *c),
            other => return Err(format!("unexpected output shape {other:?}")),
        };
        // `frames > window` would make samples-per-frame zero (division below)
        // — only possible with a wrong-but-shape-compatible model; reject it.
        if classes_n != POWERSET.len() || frames == 0 || frames > window {
            return Err(format!("unexpected output shape {shape:?}"));
        }
        let flat: Vec<f32> = arr.iter().copied().collect();
        let classes: Vec<usize> = (0..frames)
            .map(|f| {
                let row = &flat[f * classes_n..(f + 1) * classes_n];
                row.iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            })
            .collect();
        Ok((classes, window / frames))
    }
}

/// Safeguard filter over raw (sorted) cut candidates: every resulting part must
/// be at least `min_part` samples (dropping near-duplicates from overlapping
/// windows along the way), and the cut count is capped at [`MAX_CUTS`].
fn filter_cuts(cuts: Vec<usize>, min_part: usize, len: usize) -> Vec<usize> {
    let mut kept: Vec<usize> = Vec::new();
    for c in cuts {
        let prev = *kept.last().unwrap_or(&0);
        if c >= prev + min_part && c + min_part <= len {
            kept.push(c);
        }
    }
    kept.truncate(MAX_CUTS);
    kept
}

/// Pure change-point extraction from per-frame powerset classes. Returns frame
/// indices of cuts: stable single-speaker runs (≥ `min_run` frames) are found
/// first; a cut lands at the midpoint of whatever separates two consecutive
/// stable runs of DIFFERENT speakers (silence, overlap, or a direct boundary).
fn cuts_from_classes(classes: &[usize], min_run: usize) -> Vec<usize> {
    // Dominant single speaker per frame: silence and overlaps are "glue" (None)
    // — they separate turns but belong to no one.
    let dominant: Vec<Option<u8>> = classes
        .iter()
        .map(|&c| match POWERSET.get(c) {
            Some(set) if set.len() == 1 => Some(set[0]),
            _ => None,
        })
        .collect();
    // Stable runs: (start_frame, end_frame_exclusive, speaker).
    let mut runs: Vec<(usize, usize, u8)> = Vec::new();
    let mut i = 0;
    while i < dominant.len() {
        match dominant[i] {
            Some(spk) => {
                let start = i;
                while i < dominant.len() && dominant[i] == Some(spk) {
                    i += 1;
                }
                if i - start >= min_run {
                    runs.push((start, i, spk));
                }
            }
            None => i += 1,
        }
    }
    // Merge consecutive runs of the same speaker (separated by short glue),
    // then cut between neighbors with different speakers.
    let mut cuts = Vec::new();
    let mut prev: Option<(usize, usize, u8)> = None;
    for run in runs {
        if let Some(p) = prev {
            if p.2 != run.2 {
                cuts.push((p.1 + run.0) / 2);
            }
        }
        prev = Some(run);
    }
    cuts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames: 0 = silence, 1 = speaker A ({0}), 2 = speaker B ({1}), 4 = A+B.
    fn frames(spec: &[(usize, usize)]) -> Vec<usize> {
        let mut v = Vec::new();
        for &(class, n) in spec {
            v.extend(std::iter::repeat(class).take(n));
        }
        v
    }

    #[test]
    fn no_cut_for_single_speaker() {
        let f = frames(&[(0, 5), (1, 100), (0, 5)]);
        assert!(cuts_from_classes(&f, 10).is_empty());
    }

    #[test]
    fn cut_between_two_speakers_lands_mid_gap() {
        // A for 100 frames, 20 frames silence, B for 100 frames.
        let f = frames(&[(1, 100), (0, 20), (2, 100)]);
        assert_eq!(cuts_from_classes(&f, 10), vec![110]);
    }

    #[test]
    fn back_to_back_turn_without_silence_is_cut() {
        let f = frames(&[(1, 80), (2, 80)]);
        assert_eq!(cuts_from_classes(&f, 10), vec![80]);
    }

    #[test]
    fn overlap_between_turns_is_glue_not_a_speaker() {
        // A, then both speak (overlap class 4), then B: one cut mid-overlap.
        let f = frames(&[(1, 80), (4, 20), (2, 80)]);
        assert_eq!(cuts_from_classes(&f, 10), vec![90]);
    }

    #[test]
    fn short_blips_are_ignored() {
        // A brief B-flicker inside A's turn must not produce cuts.
        let f = frames(&[(1, 60), (2, 3), (1, 60)]);
        assert!(cuts_from_classes(&f, 10).is_empty());
    }

    #[test]
    fn same_speaker_resuming_after_silence_is_not_cut() {
        let f = frames(&[(1, 60), (0, 30), (1, 60)]);
        assert!(cuts_from_classes(&f, 10).is_empty());
    }

    #[test]
    fn filter_cuts_dedupes_near_duplicates_from_overlapping_windows() {
        // Two estimates of the same boundary (100 apart, < min_part): keep first.
        assert_eq!(filter_cuts(vec![16_000, 16_100, 48_000], 16_000, 64_000), vec![16_000, 48_000]);
    }

    #[test]
    fn filter_cuts_enforces_min_part_at_edges() {
        // Too close to the start / to the end of the segment: dropped.
        assert_eq!(filter_cuts(vec![500, 32_000, 63_900], 16_000, 64_000), vec![32_000]);
    }

    #[test]
    fn filter_cuts_caps_the_cut_count() {
        let cuts: Vec<usize> = (1..=10).map(|i| i * 20_000).collect();
        assert_eq!(filter_cuts(cuts, 16_000, 400_000).len(), MAX_CUTS);
    }

    /// Integration check against the real pyannote model: skipped when the
    /// model isn't installed (CI), validating the ONNX I/O contract when it is
    /// (input rank/name, output (1, frames, 7), silence → no cuts).
    #[test]
    fn real_model_runs_and_silence_yields_no_cuts() {
        let path = std::env::var("SEG_MODEL_PATH").unwrap_or_else(|_| {
            r"C:\KaigiAI\models\pyannote-segmentation-3-0\model.onnx".into()
        });
        if !std::path::Path::new(&path).exists() {
            eprintln!("segmentation model not installed — skipping");
            return;
        }
        let mut seg = OnnxSegmenter::new(&path, 16_000).expect("load model");
        // 12 s of silence spans two windows (one full, one padded).
        let silence = vec![0.0f32; 12 * 16_000];
        assert!(seg.change_points(&silence).is_empty());
        // Low-level noise shouldn't produce speaker turns either.
        let noise: Vec<f32> = (0..12 * 16_000)
            .map(|i| ((i * 2654435761usize) as f32 / usize::MAX as f32 - 0.5) * 0.01)
            .collect();
        assert!(seg.change_points(&noise).is_empty());
    }
}
