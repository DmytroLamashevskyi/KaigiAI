//! OpenAI-compatible provider: works against Groq, Gemini (OpenAI shim),
//! Ollama/LM Studio, or any corporate/self-hosted endpoint exposing
//! `/chat/completions` and `/audio/transcriptions`.

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
    pub fn new(base_url: String, api_key: String, stt_model: String, llm_model: String) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            stt_model,
            llm_model,
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
            let detail = resp.text().await.unwrap_or_default();
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
    async fn translate(&self, text: &str, from: &str, to: &str) -> ProviderResult<String> {
        let system = format!(
            "You are a professional translator. Translate the user's message from \
             language '{from}' to language '{to}'. Preserve meaning, tone and register. \
             Output ONLY the translation, with no quotes, labels or explanations."
        );
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
}

#[async_trait]
impl SttProvider for ApiProvider {
    async fn transcribe(
        &self,
        pcm: &[f32],
        sample_rate: u32,
        hint_langs: &[String],
    ) -> ProviderResult<Transcript> {
        let wav = encode_wav_pcm16(pcm, sample_rate);
        let part = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new()
            .text("model", self.stt_model.clone())
            .text("response_format", "verbose_json")
            .part("file", part);

        let mut req = self
            .client
            .post(format!("{}/audio/transcriptions", self.base_url))
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
            let detail = resp.text().await.unwrap_or_default();
            return Err(format!("API error {code}: {detail}"));
        }

        let parsed: TranscriptionResponse =
            resp.json().await.map_err(|e| format!("bad response: {e}"))?;
        let fallback = hint_langs.first().cloned().unwrap_or_else(|| "en".into());
        let lang = parsed
            .language
            .as_deref()
            .map(|l| normalize_lang(l, hint_langs))
            .unwrap_or(fallback);
        Ok(Transcript {
            text: parsed.text.trim().to_string(),
            lang,
        })
    }
}

/// Whisper APIs may return the language as a full word ("english") or an ISO
/// code ("en"). Map the common cases; otherwise fall back to a hinted lang.
fn normalize_lang(raw: &str, hint_langs: &[String]) -> String {
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
        "ar" | "arabic" => "ar",
        other => other,
    };
    if hint_langs.iter().any(|h| h == code) {
        return code.to_string();
    }
    // Unknown/unhinted: prefer a hinted language to keep A/B routing sane.
    hint_langs
        .iter()
        .find(|h| h.as_str() == code)
        .cloned()
        .unwrap_or_else(|| code.to_string())
}

/// Minimal 16-bit PCM mono WAV encoder (f32 in -1.0..=1.0).
fn encode_wav_pcm16(pcm: &[f32], sample_rate: u32) -> Vec<u8> {
    let bytes_per_sample = 2u32;
    let data_len = pcm.len() as u32 * bytes_per_sample;
    let byte_rate = sample_rate * bytes_per_sample;
    let mut buf = Vec::with_capacity(44 + data_len as usize);

    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // channels = mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&(bytes_per_sample as u16).to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in pcm {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}
