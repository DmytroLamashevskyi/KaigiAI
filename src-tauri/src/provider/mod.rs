//! AI provider abstraction (STT + translation/summary).
//!
//! Tauri-independent, like the persistence core: the same providers are driven
//! by the desktop shell today and by a future Axum server (docs/PROJECT.md §10.3).
//! Two interchangeable implementations:
//!   - [`mock::MockProvider`] — keyless echo/placeholder for development/tests.
//!   - [`api::ApiProvider`]   — any OpenAI-compatible endpoint (Groq, Gemini,
//!     Ollama/LM Studio, corporate self-host).
//! Local mode reuses [`api::ApiProvider`] pointed at on-device whisper.cpp /
//! llama.cpp servers running on `localhost` (spawned as sidecars). We don't link
//! whisper/llama in-process: both `-sys` crates vendor their own `ggml` and the
//! duplicate symbols fail to link. See docs/PROJECT.md §10.3.

pub mod api;
pub mod diarize;
pub mod mock;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub type ProviderResult<T> = Result<T, String>;

/// Output of speech-to-text for one segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    /// ISO code of the detected language ("ru", "en", ...). Drives A/B routing.
    pub lang: String,
}

#[async_trait]
pub trait SttProvider: Send + Sync {
    /// Transcribe mono `f32` PCM (range -1.0..=1.0) and detect its language.
    /// `hint_langs` is the conversation's language pair, used to bias detection.
    async fn transcribe(
        &self,
        pcm: &[f32],
        sample_rate: u32,
        hint_langs: &[String],
    ) -> ProviderResult<Transcript>;
}

#[async_trait]
pub trait TranslationProvider: Send + Sync {
    /// Translate `text` from `from` to `to`. `context` is recent conversation
    /// text (may be empty) used only to keep terminology, names and pronouns
    /// consistent across turns — it must not be translated or echoed.
    async fn translate(
        &self,
        text: &str,
        from: &str,
        to: &str,
        context: &str,
    ) -> ProviderResult<String>;
    async fn summarize(&self, transcript: &str, lang: &str) -> ProviderResult<String>;
    /// A short (3–6 word) conversation title in `lang`, derived from the
    /// transcript. Used by the "auto-title" action.
    async fn title(&self, transcript: &str, lang: &str) -> ProviderResult<String>;
}

/// The mode of one pipeline stage ("local" or "api"). STT and translation are
/// selected independently (`sttMode` / `translationMode`) so a user can run
/// e.g. local whisper for speech but a cloud LLM for translation. Falls back to
/// the legacy single `providerMode` for settings saved before the split, then
/// to "local".
fn stage_mode(settings: &serde_json::Value, mode_key: &str) -> String {
    let s = |k: &str| settings.get(k).and_then(|v| v.as_str());
    s(mode_key)
        .or_else(|| s("providerMode"))
        .unwrap_or("local")
        .to_string()
}

/// Whether the API base URL + key are present (shared by both stages when either
/// runs against a cloud endpoint).
fn has_api_creds(settings: &serde_json::Value) -> bool {
    let s = |k: &str| settings.get(k).and_then(|v| v.as_str()).unwrap_or("");
    !s("apiBaseUrl").is_empty() && !s("apiKey").is_empty()
}

/// True when the given stage selects a usable API provider (mode == "api" with
/// credentials). Otherwise the caller falls back to local or mock.
fn is_api(settings: &serde_json::Value, mode_key: &str) -> bool {
    stage_mode(settings, mode_key) == "api" && has_api_creds(settings)
}

/// Setting key naming each stage's mode.
const STT_MODE: &str = "sttMode";
const TRANSLATION_MODE: &str = "translationMode";

fn api_from_settings(settings: &serde_json::Value) -> api::ApiProvider {
    let s = |k: &str| settings.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    api::ApiProvider::new(
        s("apiBaseUrl"),
        s("apiKey"),
        s("sttModel"),
        s("llmModel"),
        "audio/transcriptions".to_string(),
    )
}

/// Default base URLs for the on-device sidecar servers. The Tauri layer injects
/// `localSttBaseUrl` / `localLlmBaseUrl` once it knows the actually-bound ports
/// (see [`crate::sidecar`]); these constants are the fallback. The whisper.cpp
/// server serves transcription at `/inference` on the server root (NOT under
/// `/v1`), so the local STT base URL has no `/v1` suffix; llama.cpp is fully
/// OpenAI-shaped and keeps `/v1`.
pub const DEFAULT_LOCAL_STT_URL: &str = "http://127.0.0.1:8771";
pub const DEFAULT_LOCAL_LLM_URL: &str = "http://127.0.0.1:8770/v1";

/// whisper.cpp server's native transcription route (server root, no `/v1`).
pub const LOCAL_STT_ENDPOINT: &str = "inference";

/// True when the given stage selects on-device models (mode == "local"). The
/// actual servers are user-installed and spawned as sidecars by the Tauri layer.
fn is_local(settings: &serde_json::Value, mode_key: &str) -> bool {
    stage_mode(settings, mode_key) == "local"
}

/// Whether speech recognition runs on a local whisper.cpp sidecar.
pub fn stt_is_local(settings: &serde_json::Value) -> bool {
    is_local(settings, STT_MODE)
}

/// Whether translation/summary runs on a local llama.cpp sidecar.
pub fn translation_is_local(settings: &serde_json::Value) -> bool {
    is_local(settings, TRANSLATION_MODE)
}

/// Build an ApiProvider for a local sidecar server (no API key needed). The STT
/// path is whisper.cpp's `/inference`; translation ignores it (uses
/// `/chat/completions`), so the same value is harmless there.
fn local_provider(settings: &serde_json::Value, base_url_key: &str, default_url: &str) -> api::ApiProvider {
    let s = |k: &str| settings.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let base = match settings.get(base_url_key).and_then(|v| v.as_str()) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => default_url.to_string(),
    };
    api::ApiProvider::new(base, String::new(), s("sttModel"), s("llmModel"), LOCAL_STT_ENDPOINT.to_string())
}

/// Select the translation/summary provider for the given settings blob.
pub fn translation_from_settings(settings: &serde_json::Value) -> Box<dyn TranslationProvider> {
    if is_local(settings, TRANSLATION_MODE) {
        Box::new(local_provider(settings, "localLlmBaseUrl", DEFAULT_LOCAL_LLM_URL))
    } else if is_api(settings, TRANSLATION_MODE) {
        Box::new(api_from_settings(settings))
    } else {
        Box::new(mock::MockProvider)
    }
}

/// Select the speech-to-text provider for the given settings blob.
pub fn stt_from_settings(settings: &serde_json::Value) -> Box<dyn SttProvider> {
    if is_local(settings, STT_MODE) {
        Box::new(local_provider(settings, "localSttBaseUrl", DEFAULT_LOCAL_STT_URL))
    } else if is_api(settings, STT_MODE) {
        Box::new(api_from_settings(settings))
    } else {
        Box::new(mock::MockProvider)
    }
}

/// Build the per-session diarizer. Returns [`diarize::NullDiarizer`] (speaker =
/// None) unless `diarizationModelPath` points to a loadable ONNX embedding
/// model. See docs/PROJECT.md §10.6.
pub fn diarizer_from_settings(settings: &serde_json::Value) -> Box<dyn diarize::Diarizer> {
    let path = settings
        .get("diarizationModelPath")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !path.is_empty() {
        match diarize::OnnxDiarizer::new(path, crate::audio::SAMPLE_RATE) {
            Ok(d) => return Box::new(d),
            Err(e) => log::error!("diarization disabled: {e}"),
        }
    }
    Box::new(diarize::NullDiarizer)
}
