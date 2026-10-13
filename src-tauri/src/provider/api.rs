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

    /// Whether this provider talks to the on-device whisper.cpp / llama.cpp
    /// sidecars (built with the whisper.cpp `/inference` route) rather than a
    /// cloud OpenAI-compatible API.
    fn is_local(&self) -> bool {
        self.stt_endpoint == "inference"
    }

    /// One chat completion. `max_tokens` is a hard ceiling on the reply: small
    /// local models occasionally loop and never emit end-of-turn (seen with
    /// Qwen2.5-3B translating into Mongolian: 16k tokens until the client gave
    /// up), which holds a llama-server slot — and slows every other request —
    /// until the 90 s timeout.
    ///
    /// `grammar` (GBNF) constrains the output; only llama-server understands
    /// it, so callers pass one only for the local server ([`Self::is_local`]).
    async fn chat(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
        grammar: Option<&str>,
    ) -> ProviderResult<String> {
        let mut body = serde_json::json!({
            "model": self.llm_model,
            "temperature": 0.2,
            "max_tokens": max_tokens,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        if let Some(g) = grammar {
            body["grammar"] = serde_json::Value::String(g.to_string());
        }
        // llama-server answers 500 "Failed to parse input" when the model emitted
        // an invalid UTF-8 sequence it can't parse back (~1 in 360 translations
        // with Qwen2.5-3B, §10.19). Sampling is random, so a retry usually
        // succeeds (2 of 3 in a repro) — up to two retries for that error only.
        let mut attempt = 0;
        let resp = loop {
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
            if resp.status().is_success() {
                break resp;
            }
            let code = resp.status();
            let raw = resp.text().await.unwrap_or_default();
            if is_retryable_parse_error(code, &raw) && attempt < 2 {
                attempt += 1;
                log::warn!("LLM output parse error, retrying ({attempt}/2)");
                continue;
            }
            return Err(format!("API error {code}: {}", short_detail(raw)));
        };

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
        // Full language names, not ISO codes: small models follow "Russian" far
        // better than 'ru' (§10.19).
        let (from_name, to_name) = (language_name(from), language_name(to));
        let mut system = format!(
            "You are a professional translator. Translate the user's message from \
             {from_name} to {to_name}. Preserve meaning, tone and register. \
             Output ONLY the translation, with no quotes, labels or explanations."
        );
        if !context.is_empty() {
            system.push_str(&format!(
                "\n\nRecent conversation so far, for consistency of terminology, names \
                 and pronouns. Do NOT translate or repeat it; use it only as context:\n{context}"
            ));
        }
        let max_tokens = translation_max_tokens(text);
        // Small models (Qwen2.5-3B) drift into Chinese mid-sentence: CJK
        // characters showed up in ~25% of translations into Russian in the
        // FLEURS benchmark, and neither the prompt nor a retry fixes it — the
        // model consistently prefers Chinese there. On the local llama-server a
        // grammar forbids those characters outright for targets written without
        // them; cloud APIs don't take a grammar, so they get one retry with an
        // explicit instruction instead (keeping whichever answer is cleaner).
        let grammar = (!writes_cjk(to) && self.is_local()).then_some(NO_CJK_GRAMMAR);
        let out = self.chat(&system, text, max_tokens, grammar).await?;
        if writes_cjk(to) || cjk_count(&out) == 0 {
            return Ok(out);
        }
        log::warn!("translation to {to} contains CJK characters, retrying");
        system.push_str(&format!(
            "\n\nWrite the translation entirely in {to_name}. Do not use any Chinese, \
             Japanese or Korean characters; transliterate names into {to_name}."
        ));
        match self.chat(&system, text, max_tokens, None).await {
            Ok(retry) if cjk_count(&retry) < cjk_count(&out) => Ok(retry),
            _ => Ok(out),
        }
    }

    async fn summarize(&self, transcript: &str, lang: &str) -> ProviderResult<String> {
        let system = format!(
            "Summarize the following bilingual conversation transcript in language '{lang}'. \
             Produce concise markdown: a short overview, key points as bullets, and an \
             'Action items' list. Output only the markdown."
        );
        self.chat(&system, transcript, 2048, None).await
    }

    async fn title(&self, transcript: &str, lang: &str) -> ProviderResult<String> {
        let system = format!(
            "Generate a very short title (3 to 6 words) for this conversation transcript, \
             in language '{lang}'. Capture the main topic. Output ONLY the title text — no \
             quotes, no punctuation at the end, no labels."
        );
        let raw = self.chat(&system, transcript, 48, None).await?;
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
        let text = if self.stt_endpoint == "inference" {
            join_whisper_cpp_segments(&parsed.text)
        } else {
            parsed.text
        };
        Ok(Transcript {
            text: text.trim().to_string(),
            lang,
        })
    }
}

/// English name for an ISO code ("ru" → "Russian") from [`WHISPER_LANGS`];
/// unknown codes pass through unchanged.
fn language_name(code: &str) -> String {
    WHISPER_LANGS
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(name, _)| {
            let mut ch = name.chars();
            ch.next()
                .map(|f| f.to_uppercase().chain(ch).collect())
                .unwrap_or_default()
        })
        .unwrap_or_else(|| code.to_string())
}

/// GBNF for llama-server: any text without kana, CJK ideographs, hangul, CJK
/// punctuation (、。「」) or full-width forms (，！). Used for translations into
/// languages not written with them.
const NO_CJK_GRAMMAR: &str =
    r"root ::= [^　-ヿ㐀-鿿가-힯＀-￯]*";

/// Languages normally written with Han / kana / hangul.
fn writes_cjk(lang: &str) -> bool {
    matches!(lang, "ja" | "zh" | "ko" | "yue")
}

/// Number of Han, kana and hangul characters in `text`.
pub fn cjk_count(text: &str) -> usize {
    text.chars()
        .filter(|c| {
            matches!(c,
                '\u{3040}'..='\u{30FF}'      // hiragana, katakana
                | '\u{3400}'..='\u{9FFF}'    // CJK ideographs (+ ext. A)
                | '\u{AC00}'..='\u{D7AF}')   // hangul syllables
        })
        .count()
}

/// llama-server's 500 for a completion it generated but could not parse back.
fn is_retryable_parse_error(code: reqwest::StatusCode, body: &str) -> bool {
    code == reqwest::StatusCode::INTERNAL_SERVER_ERROR && body.contains("Failed to parse input")
}

/// Reply ceiling for translating `text`. A faithful translation stays within a
/// few tokens per source character in any script (CJK ≈ 1 token/char, Latin and
/// Cyrillic far fewer), so 3×chars + slack never truncates a real translation
/// but stops a runaway loop within a second or two.
fn translation_max_tokens(text: &str) -> u32 {
    (text.chars().count() as u32 * 3 + 64).clamp(128, 2048)
}

/// whisper.cpp's server joins its decoder segments with `\n`, and a segment
/// boundary can fall INSIDE a word ("солн\nечная"). Each segment that starts a
/// new word already begins with a space, so dropping the newlines restores the
/// text exactly; with the UI's `pre-wrap` cells they otherwise render as line
/// breaks mid-word. (Found by the FLEURS benchmark, §10.19.)
fn join_whisper_cpp_segments(text: &str) -> String {
    text.replace(['\r', '\n'], "")
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

/// Whisper's language names → ISO 639-1 codes (whisper's own table, with
/// `jw` corrected to ISO `jv`). whisper.cpp reports the detected language as
/// the full name ("mongolian"), cloud APIs often as the code.
const WHISPER_LANGS: &[(&str, &str)] = &[
    ("english", "en"), ("chinese", "zh"), ("german", "de"), ("spanish", "es"),
    ("russian", "ru"), ("korean", "ko"), ("french", "fr"), ("japanese", "ja"),
    ("portuguese", "pt"), ("turkish", "tr"), ("polish", "pl"), ("catalan", "ca"),
    ("dutch", "nl"), ("arabic", "ar"), ("swedish", "sv"), ("italian", "it"),
    ("indonesian", "id"), ("hindi", "hi"), ("finnish", "fi"), ("vietnamese", "vi"),
    ("hebrew", "he"), ("ukrainian", "uk"), ("greek", "el"), ("malay", "ms"),
    ("czech", "cs"), ("romanian", "ro"), ("danish", "da"), ("hungarian", "hu"),
    ("tamil", "ta"), ("norwegian", "no"), ("thai", "th"), ("urdu", "ur"),
    ("croatian", "hr"), ("bulgarian", "bg"), ("lithuanian", "lt"), ("latin", "la"),
    ("maori", "mi"), ("malayalam", "ml"), ("welsh", "cy"), ("slovak", "sk"),
    ("telugu", "te"), ("persian", "fa"), ("latvian", "lv"), ("bengali", "bn"),
    ("serbian", "sr"), ("azerbaijani", "az"), ("slovenian", "sl"), ("kannada", "kn"),
    ("estonian", "et"), ("macedonian", "mk"), ("breton", "br"), ("basque", "eu"),
    ("icelandic", "is"), ("armenian", "hy"), ("nepali", "ne"), ("mongolian", "mn"),
    ("bosnian", "bs"), ("kazakh", "kk"), ("albanian", "sq"), ("swahili", "sw"),
    ("galician", "gl"), ("marathi", "mr"), ("punjabi", "pa"), ("sinhala", "si"),
    ("khmer", "km"), ("shona", "sn"), ("yoruba", "yo"), ("somali", "so"),
    ("afrikaans", "af"), ("occitan", "oc"), ("georgian", "ka"), ("belarusian", "be"),
    ("tajik", "tg"), ("sindhi", "sd"), ("gujarati", "gu"), ("amharic", "am"),
    ("yiddish", "yi"), ("lao", "lo"), ("uzbek", "uz"), ("faroese", "fo"),
    ("haitian creole", "ht"), ("pashto", "ps"), ("turkmen", "tk"), ("nynorsk", "nn"),
    ("maltese", "mt"), ("sanskrit", "sa"), ("luxembourgish", "lb"), ("myanmar", "my"),
    ("tibetan", "bo"), ("tagalog", "tl"), ("malagasy", "mg"), ("assamese", "as"),
    ("tatar", "tt"), ("hawaiian", "haw"), ("lingala", "ln"), ("hausa", "ha"),
    ("bashkir", "ba"), ("javanese", "jv"), ("sundanese", "su"), ("cantonese", "yue"),
    // Aliases seen from cloud providers.
    ("mandarin", "zh"), ("jw", "jv"),
];

/// Map a provider's language label to an ISO code: a full whisper name or an
/// alias is looked up, a code passes through. Anything unknown passes through
/// unchanged — `resolve_lang_n` in recording.rs does the real snapping onto the
/// conversation languages downstream. Before the full table only 15 names were
/// known, so e.g. Mongolian came back as "mongolian" and never matched "mn"
/// (found by the FLEURS benchmark, §10.19).
fn normalize_lang(raw: &str) -> String {
    let l = raw.trim().to_lowercase();
    WHISPER_LANGS
        .iter()
        .find(|(name, _)| *name == l)
        .map(|(_, code)| (*code).to_string())
        .unwrap_or(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_names_map_to_iso() {
        assert_eq!(normalize_lang("Mongolian"), "mn");
        assert_eq!(normalize_lang("javanese"), "jv");
        assert_eq!(normalize_lang("haitian creole"), "ht");
        assert_eq!(normalize_lang("mandarin"), "zh");
        assert_eq!(normalize_lang("ru"), "ru");
        assert_eq!(normalize_lang("klingon"), "klingon");
    }

    #[test]
    fn language_names_for_prompts() {
        assert_eq!(language_name("ru"), "Russian");
        assert_eq!(language_name("zh"), "Chinese");
        assert_eq!(language_name("ht"), "Haitian creole");
        assert_eq!(language_name("xx"), "xx");
    }

    #[test]
    fn cjk_detection() {
        assert_eq!(cjk_count("зданиями с стеклянными外墙 и"), 2);
        assert_eq!(cjk_count("こんにちは 한국"), 7);
        assert_eq!(cjk_count("Hello, мир"), 0);
        assert!(writes_cjk("ja") && !writes_cjk("ru"));
    }

    #[test]
    fn only_parse_errors_are_retried() {
        let e500 = reqwest::StatusCode::INTERNAL_SERVER_ERROR;
        let body = r#"{"error":{"code":500,"message":"Failed to parse input at pos 22: x"}}"#;
        assert!(is_retryable_parse_error(e500, body));
        assert!(!is_retryable_parse_error(e500, "out of memory"));
        assert!(!is_retryable_parse_error(reqwest::StatusCode::BAD_REQUEST, body));
    }

    #[test]
    fn translation_cap_scales_with_input() {
        assert_eq!(translation_max_tokens("Да"), 128);
        let sentence = "Тысячи лет назад человек по имени Аристарх сказал, что Солнечная система вращается вокруг Солнца.";
        assert_eq!(translation_max_tokens(sentence), sentence.chars().count() as u32 * 3 + 64);
        assert_eq!(translation_max_tokens(&"слово ".repeat(1000)), 2048);
    }

    #[test]
    fn whisper_cpp_segments_rejoin_mid_word() {
        assert_eq!(
            join_whisper_cpp_segments("что солн\nечная система\n вращается"),
            "что солнечная система вращается"
        );
        assert_eq!(join_whisper_cpp_segments("в иннинге,\n 70 ранов"), "в иннинге, 70 ранов");
    }
}
