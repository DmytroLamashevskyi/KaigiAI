//! Speaker diarization (etap 3, design in docs/PROJECT.md §10.6).
//!
//! One VAD segment = one utterance, so diarization only has to *label* each
//! segment with a speaker. A [`Diarizer`] is created per recording session and
//! keeps its clustering state internal, so labels (`Speaker 1`, `Speaker 2`, …)
//! are stable within a conversation but never leak across sessions.
//!
//! [`NullDiarizer`] is the default (no model configured → `speaker = None`).
//! The ONNX embedding implementation (via the `ort` crate) lands in a later
//! step; until then the trait and wiring are in place with zero runtime cost.

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
