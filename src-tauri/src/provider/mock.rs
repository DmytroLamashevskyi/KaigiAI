//! Keyless placeholder provider for development and tests. No network, no models.

use async_trait::async_trait;

use super::{ProviderResult, SttProvider, TranslationProvider, Transcript};

pub struct MockProvider;

#[async_trait]
impl SttProvider for MockProvider {
    async fn transcribe(
        &self,
        pcm: &[f32],
        sample_rate: u32,
        hint_langs: &[String],
    ) -> ProviderResult<Transcript> {
        let secs = pcm.len() as f32 / sample_rate.max(1) as f32;
        let lang = hint_langs.first().cloned().unwrap_or_else(|| "en".into());
        Ok(Transcript {
            text: format!("(mock STT, {secs:.1}s)"),
            lang,
        })
    }
}

#[async_trait]
impl TranslationProvider for MockProvider {
    async fn translate(&self, text: &str, _from: &str, to: &str, _context: &str) -> ProviderResult<String> {
        Ok(format!("[{}] {}", to.to_uppercase(), text))
    }

    async fn summarize(&self, _transcript: &str, _lang: &str) -> ProviderResult<String> {
        Ok("## (mock summary)\n- Key point 1\n- Key point 2\n- Action item".into())
    }
}
