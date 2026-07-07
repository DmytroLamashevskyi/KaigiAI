//! OpenAI-compatible provider: works against Groq, Gemini (OpenAI shim),
//! Ollama/LM Studio, or any corporate/self-hosted endpoint exposing
//! `/chat/completions` and `/audio/transcriptions`.
//!
//! The transcription path is configurable (`stt_endpoint`) because the local
//! whisper.cpp server is NOT fully OpenAI-shaped: it serves transcription at
//! `/inference` (at the server root, not under `/v1`) rather than
//! `/v1/audio/transcriptions`. Its JSON response (`text` + `language`) already
//! matches what we parse, so only the route differs.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use super::{ProviderResult, SttProvider, TranslationProvider, Transcript};

pub struct ApiProvider {
    client: Client,
    base_url: String,
    api_key: String,
    stt_model: String,
    llm_model: String,
    /// Transcription route appended to `base_url`. OpenAI/cloud:
    /// `"audio/transcriptions"`; local whisper.cpp server: `"inference"`.
    stt_endpoint: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}
#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}
#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

impl ApiProvider {
    pub fn new(
        base_url: String,
        api_key: String,
        stt_model: String,
        llm_model: String,
        stt_endpoint: String,
    ) -> Self {
        // A finite request timeout so a hung sidecar (whisper/llama that stops
        // responding — e.g. GPU thrashing near the VRAM limit) fails the segment
        // instead of leaving its placeholder spinning forever. Normal STT /
        // translation of one utterance completes in seconds; 90 s is a generous
        // backstop. Falls back to an untimed client if the builder fails.
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            stt_model,
            llm_model,
            stt_endpoint: stt_endpoint.trim_matches('/').to_string(),
        }
    }

    async fn chat(&self, system: &str, user: &str) -> ProviderResult<String> {
        let body = serde_json::json!({
            "model": self.llm_model,
            "temperature": 0.2,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        if !resp.status().is_success() {
            let code = resp.status();
            let detail = short_detail(resp.text().await.unwrap_or_default());
            return Err(format!("API error {code}: {detail}"));
        }

        let parsed: ChatResponse = resp.json().await.map_err(|e| format!("bad response: {e}"))?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content.trim().to_string())
            .ok_or_else(|| "empty completion".to_string())
    }
}

#[async_trait]
impl TranslationProvider for ApiProvider {
    async fn translate(&self, text: &str, from: &str, to: &str, context: &str) -> ProviderResult<String> {
        let mut system = format!(
            "You are a professional translator. Translate the user's message from \
             language '{from}' to language '{to}'. Preserve meaning, tone and register. \
             Output ONLY the translation, with no quotes, labels or explanations."
        );
        if !context.is_empty() {
            system.push_str(&format!(
                "\n\nRecent conversation so far, for consistency of terminology, names \
                 and pronouns. Do NOT translate or repeat it; use it only as context:\n{context}"
            ));
        }
        self.chat(&system, text).await
    }

    async fn summarize(&self, transcript: &str, lang: &str) -> ProviderResult<String> {
        let system = format!(
            "Summarize the following bilingual conversation transcript in language '{lang}'. \
             Produce concise markdown: a short overview, key points as bullets, and an \
             'Action items' list. Output only the markdown."
        );
        self.chat(&system, transcript).await
    }

    async fn title(&self, transcript: &str, lang: &str) -> ProviderResult<String> {
        let system = format!(
            "Generate a very short title (3 to 6 words) for this conversation transcript, \
             in language '{lang}'. Capture the main topic. Output ONLY the title text — no \
             quotes, no punctuation at the end, no labels."
        );
        let raw = self.chat(&system, transcript).await?;
        // Models sometimes wrap the title in quotes or add a trailing period.
        Ok(raw.trim().trim_matches(|c| c == '"' || c == '«' || c == '»' || c == '.').trim().to_string())
    }
}

#[async_trait]
impl SttProvider for ApiProvider {
    async fn transcribe(
        &self,
        pcm: &[f32],
        sample_rate: u32,
        hint_langs: &[String],
    ) -> ProviderResult<Transcript> {
        let wav = crate::audio::encode_wav_pcm16(pcm, sample_rate);
        let part = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", self.stt_model.clone())
            .text("response_format", "verbose_json")
            .part("file", part);
        // whisper.cpp's server defaults `language` to "en" when the field is
        // absent, which FORCES an English decode on every segment: Russian and
        // Japanese speech then comes back as (mis)translated English and the
        // detected language is always "en", so A/B routing collapses to one
        // side. Sending "auto" restores real language auto-detection. We only
        // do this for the local whisper.cpp route ("inference"); the cloud
        // OpenAI endpoint auto-detects when the field is omitted and rejects
        // the literal "auto".
        if self.stt_endpoint == "inference" {
            form = form.text("language", "auto");
        }

        let mut req = self
            .client
            .post(format!("{}/{}", self.base_url, self.stt_endpoint))
            .multipart(form);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        if !resp.status().is_success() {
            let code = resp.status();
            let detail = short_detail(resp.text().await.unwrap_or_default());
            // A 404 on the transcription route usually means the provider has no
            // speech-to-text endpoint at all (e.g. Gemini's OpenAI-compatible
            // layer) — a common misconfiguration worth calling out explicitly.
            if code == reqwest::StatusCode::NOT_FOUND {
                return Err(format!(
                    "endpoint not found (404) — this provider may not support speech \
                     recognition. Gemini, for one, has no audio transcription endpoint; \
                     use Groq or local whisper for speech. Details: {detail}"
                ));
            }
            return Err(format!("API error {code}: {detail}"));
        }

        let parsed: TranscriptionResponse =
            resp.json().await.map_err(|e| format!("bad response: {e}"))?;
        let fallback = hint_langs.first().cloned().unwrap_or_else(|| "en".into());
        let lang = parsed
            .language
            .as_deref()
            .map(normalize_lang)
            .unwrap_or(fallback);
        Ok(Transcript {
            text: parsed.text.trim().to_string(),
            lang,
        })
    }
}

/// Trim a provider error body to a short, single-line snippet so a giant HTML/
/// JSON error page doesn't fill the UI toast (the full body is still logged
/// upstream via the recording error path).
fn short_detail(body: String) -> String {
    let one_line: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = one_line.chars().take(200).collect();
    if one_line.chars().count() > 200 {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

/// Whisper APIs may return the language as a full word ("english") or an ISO
/// code ("en"). Map the common spellings to ISO codes; anything else passes
/// through unchanged — `resolve_lang_n` in recording.rs does the real snapping
/// onto the conversation languages downstream.
fn normalize_lang(raw: &str) -> String {
    let l = raw.trim().to_lowercase();
    let code = match l.as_str() {
        "en" | "english" => "en",
        "ru" | "russian" => "ru",
        "ja" | "japanese" => "ja",
        "zh" | "chinese" | "mandarin" => "zh",
        "es" | "spanish" => "es",
        "fr" | "french" => "fr",
        "de" | "german" => "de",
        "it" | "italian" => "it",
        "pt" | "portuguese" => "pt",
        "ko" | "korean" => "ko",
        "uk" | "ukrainian" => "uk",
        "pl" | "polish" => "pl",
        "tr" | "turkish" => "tr",
        "id" | "indonesian" => "id",
        "ar" | "arabic" => "ar",
        other => other,
    };
    code.to_string()
}
