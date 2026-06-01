//! Speaker diarization (etap 3, design in docs/PROJECT.md §10.6).
//!
//! One VAD segment = one utterance, so diarization only has to *label* each
//! segment with a speaker. A [`Diarizer`] is created per recording session and
//! keeps its clustering state internal, so labels (`Speaker 1`, `Speaker 2`, …)
//! are stable within a conversation but never leak across sessions.
//!
//! [`NullDiarizer`] is the default (no model configured → `speaker = None`).
//! [`OnnxDiarizer`] runs an on-device speaker-embedding ONNX model (wespeaker /
//! 3D-Speaker style: 80-dim fbank in, L2-normalizable embedding out) via the
//! `ort` crate and clusters online by cosine similarity.

use ort::session::Session;
use ort::value::Tensor;

/// Assigns a stable speaker label to each utterance of a session.
pub trait Diarizer: Send {
    /// Label the speaker of one mono `f32` PCM segment, or `None` if diarization
    /// is disabled / cannot decide. Stateful: successive calls within a session
    /// reuse and extend the same speaker set.
    fn label(&mut self, pcm: &[f32], sample_rate: u32) -> Option<String>;
}

/// Default no-op diarizer: always `None`. Selected whenever no embedding model
/// is configured, keeping `message.speaker` NULL exactly as before.
pub struct NullDiarizer;

impl Diarizer for NullDiarizer {
    fn label(&mut self, _pcm: &[f32], _sample_rate: u32) -> Option<String> {
        None
    }
}

// --- fbank front-end -------------------------------------------------------
// Kaldi-style log-mel filterbank, the de-facto input for wespeaker/3D-Speaker
// embedding models: 16 kHz, 25 ms window (400 samples), 10 ms hop (160), 80 mel
// bins over 0..8000 Hz, power spectrum via a 512-point FFT, natural log, then
// per-dimension mean normalization (CMN) over the utterance.

const N_FFT: usize = 512;
const N_MELS: usize = 80;
const WIN: usize = 400;
const HOP: usize = 160;

fn hz_to_mel(hz: f32) -> f32 {
    1127.0 * (1.0 + hz / 700.0).ln()
}

/// Kaldi pre-emphasis coefficient (high-pass tilt applied before windowing).
const PREEMPH: f32 = 0.97;

/// Triangular mel filterbank: `N_MELS` filters over the `N_FFT/2 + 1`
/// power-spectrum bins. The triangles are linear in the **mel** domain (Kaldi
/// convention), not in Hz — interpolating the slopes in Hz (as a naive
/// implementation does) distorts the higher filters and measurably worsens the
/// resulting speaker embeddings.
fn mel_filters(sample_rate: u32) -> Vec<Vec<f32>> {
    let n_bins = N_FFT / 2 + 1;
    let f_max = sample_rate as f32 / 2.0;
    let mel_min = hz_to_mel(0.0);
    let mel_max = hz_to_mel(f_max);
    // N_MELS+2 equally spaced points in the mel domain → triangle edges.
    let mel_pts: Vec<f32> = (0..N_MELS + 2)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (N_MELS as f32 + 1.0))
        .collect();
    // Mel value of each FFT bin's centre frequency.
    let bin_mel: Vec<f32> = (0..n_bins)
        .map(|k| hz_to_mel(k as f32 * sample_rate as f32 / N_FFT as f32))
        .collect();
    let mut filters = vec![vec![0.0f32; n_bins]; N_MELS];
    for m in 0..N_MELS {
        let (lo, ce, hi) = (mel_pts[m], mel_pts[m + 1], mel_pts[m + 2]);
        for (k, &bm) in bin_mel.iter().enumerate() {
            let up = (bm - lo) / (ce - lo);
            let dn = (hi - bm) / (hi - ce);
            let w = up.min(dn);
            if w > 0.0 {
                filters[m][k] = w;
            }
        }
    }
    filters
}

/// In-place iterative radix-2 Cooley–Tukey FFT (`re`/`im` length must be `N_FFT`).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    // bit-reversal permutation
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let a = i + k;
                let b = i + k + len / 2;
                let tr = cr * re[b] - ci * im[b];
                let ti = cr * im[b] + ci * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Compute the `frames x N_MELS` log-mel filterbank of `pcm`, mean-normalized
/// per dimension. Returns a flat row-major buffer plus the frame count.
fn fbank(pcm: &[f32], sample_rate: u32) -> (Vec<f32>, usize) {
    if pcm.len() < WIN {
        return (Vec::new(), 0);
    }
    let filters = mel_filters(sample_rate);
    // Povey window (Kaldi default for fbank).
    let window: Vec<f32> = (0..WIN)
        .map(|i| {
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (WIN as f32 - 1.0)).cos();
            w.powf(0.85)
        })
        .collect();

    let n_frames = 1 + (pcm.len() - WIN) / HOP;
    let mut out = vec![0.0f32; n_frames * N_MELS];
    let mut re = vec![0.0f32; N_FFT];
    let mut im = vec![0.0f32; N_FFT];
    for f in 0..n_frames {
        let start = f * HOP;
        re.iter_mut().for_each(|v| *v = 0.0);
        im.iter_mut().for_each(|v| *v = 0.0);
        // Kaldi frame pipeline: remove DC offset, pre-emphasis, then window.
        let mut mean = 0.0f32;
        for i in 0..WIN {
            mean += pcm[start + i];
        }
        mean /= WIN as f32;
        // Pre-emphasis baseline for i=0 is the (DC-removed) first sample, so
        // sample 0 becomes (1 - PREEMPH) * x0, matching Kaldi.
        let mut prev = pcm[start] - mean;
        for i in 0..WIN {
            let x = pcm[start + i] - mean;
            let pe = x - PREEMPH * prev;
            prev = x;
            re[i] = pe * window[i];
        }
        fft(&mut re, &mut im);
        for (m, filt) in filters.iter().enumerate() {
            let mut e = 0.0f32;
            for (k, &w) in filt.iter().enumerate() {
                if w != 0.0 {
                    let p = re[k] * re[k] + im[k] * im[k];
                    e += w * p;
                }
            }
            out[f * N_MELS + m] = (e + 1e-10).ln();
        }
    }
    // Cepstral mean normalization per mel dimension over the utterance.
    for m in 0..N_MELS {
        let mut mean = 0.0f32;
        for f in 0..n_frames {
            mean += out[f * N_MELS + m];
        }
        mean /= n_frames as f32;
        for f in 0..n_frames {
            out[f * N_MELS + m] -= mean;
        }
    }
    (out, n_frames)
}

// --- ONNX embedding diarizer ----------------------------------------------

/// Cosine-similarity threshold above which a segment joins an existing speaker.
const SIM_THRESHOLD: f32 = 0.50;

/// Minimum speech duration (seconds) for an embedding we trust enough to *open a
/// new speaker*. Short clips (a word or two) give unstable embeddings that would
/// otherwise spawn a spurious speaker per utterance; below this we attach the
/// segment to the nearest existing speaker instead of creating a new one.
const MIN_NEW_SPEAKER_SECS: f32 = 0.7;

pub struct OnnxDiarizer {
    session: Session,
    sample_rate: u32,
    /// L2-normalized centroid embedding per discovered speaker.
    centroids: Vec<Vec<f32>>,
}

impl OnnxDiarizer {
    /// Load the embedding model. `sample_rate` is the PCM rate the pipeline
    /// feeds in (16 kHz from VAD); fbank is computed at that rate.
    pub fn new(model_path: &str, sample_rate: u32) -> Result<Self, String> {
        let session = Session::builder()
            .map_err(|e| format!("ort init: {e}"))?
            .commit_from_file(model_path)
            .map_err(|e| format!("load diarization model: {e}"))?;
        Ok(Self {
            session,
            sample_rate,
            centroids: Vec::new(),
        })
    }

    /// Run the model on one segment and return its L2-normalized embedding.
    fn embed(&mut self, pcm: &[f32]) -> Result<Vec<f32>, String> {
        let (feats, n_frames) = fbank(pcm, self.sample_rate);
        if n_frames == 0 {
            return Err("segment too short for fbank".into());
        }
        let input = Tensor::from_array(([1usize, n_frames, N_MELS], feats))
            .map_err(|e| format!("build input tensor: {e}"))?;
        let input_name = self.session.inputs[0].name.clone();
        let inputs = ort::inputs![input_name.as_str() => input]
            .map_err(|e| format!("build inputs: {e}"))?;
        let outputs = self
            .session
            .run(inputs)
            .map_err(|e| format!("diarization inference: {e}"))?;
        let arr = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("extract embedding: {e}"))?;
        let mut emb: Vec<f32> = arr.iter().copied().collect();
        l2_normalize(&mut emb);
        Ok(emb)
    }
}

impl Diarizer for OnnxDiarizer {
    fn label(&mut self, pcm: &[f32], _sample_rate: u32) -> Option<String> {
        let emb = match self.embed(pcm) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("diarization skipped: {e}");
                return None;
            }
        };
        // Nearest existing speaker by cosine similarity (embeddings are L2-normed,
        // so the dot product is the cosine).
        let mut best = (-1.0f32, usize::MAX);
        for (i, c) in self.centroids.iter().enumerate() {
            let sim = dot(&emb, c);
            if sim > best.0 {
                best = (sim, i);
            }
        }
        let long_enough =
            pcm.len() as f32 / self.sample_rate as f32 >= MIN_NEW_SPEAKER_SECS;
        let idx = if best.0 >= SIM_THRESHOLD {
            // Confident match: fold the sample into the centroid (running mean)
            // and renormalize.
            let c = &mut self.centroids[best.1];
            for (cv, ev) in c.iter_mut().zip(&emb) {
                *cv = 0.9 * *cv + 0.1 * ev;
            }
            l2_normalize(c);
            best.1
        } else if long_enough || self.centroids.is_empty() {
            // Below threshold but a trustworthy (long) clip, or the very first
            // utterance: this is a genuinely new speaker.
            self.centroids.push(emb);
            self.centroids.len() - 1
        } else {
            // Short, low-confidence clip: don't spawn a speaker on weak evidence.
            // Attach to the nearest existing one without polluting its centroid.
            best.1
        };
        Some(format!("Speaker {}", idx + 1))
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-8 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}
