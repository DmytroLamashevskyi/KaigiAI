//! Audio capture pipeline: cpal microphone input -> downmix/resample to mono
//! 16 kHz -> energy VAD -> speech segments. Provider/Tauri-independent so the
//! same pipeline can drive a future Axum server (docs/PROJECT.md §6, §10.3).

pub mod capture;
pub mod vad;
pub mod wav;

pub use capture::{list_input_devices, AudioCapture, AudioSource};
pub use vad::{VadConfig, SAMPLE_RATE};
pub use wav::encode_wav_pcm16;
