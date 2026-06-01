//! Live recording orchestrator.
//!
//! Bridges the Tauri-independent audio pipeline ([`crate::audio`]) to the
//! providers and database: microphone -> VAD segments -> STT -> A/B-routed
//! translation -> persisted [`Message`] -> `transcript-message` event for the
//! UI to append in realtime.
//!
//! cpal's `Stream` is `!Send`, so the capture object lives on a dedicated
//! controller thread; segments cross thread boundaries as plain PCM.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::unbounded_channel;

use crate::audio::{AudioCapture, AudioSource, VadConfig, SAMPLE_RATE};
use crate::db::{Conversation, Db, Message};
use crate::provider;

/// Event name for non-fatal recording failures surfaced to the UI as a toast.
pub const RECORDING_ERROR_EVENT: &str = "recording-error";

/// Tauri-managed recording state. Holds the stop handle of the active session,
/// or `None` when idle.
#[derive(Default)]
pub struct Recorder {
    stop: Mutex<Option<Sender<()>>>,
}

impl Recorder {
    pub fn is_recording(&self) -> bool {
        self.stop.lock().unwrap().is_some()
    }

    /// Begin a session. `db`/`settings`/`conv` are resolved by the caller (async
    /// command) and passed in so this stays synchronous.
    pub fn start(
        &self,
        app: AppHandle,
        db: Db,
        conv: Conversation,
        settings: serde_json::Value,
    ) -> Result<(), String> {
        let mut guard = self.stop.lock().unwrap();
        if guard.is_some() {
            return Err("already recording".into());
        }

        let lang_a = conv.lang_a.clone();
        let lang_b = conv.lang_b.clone();
        let conv_id = conv.id.clone();
        let source = match settings.get("audioSource").and_then(|v| v.as_str()) {
            Some("system") => AudioSource::System,
            _ => AudioSource::Mic,
        };
        // Device selection only applies to a microphone; system audio is captured
        // from the default output endpoint via loopback.
        let device = if source == AudioSource::Mic {
            settings
                .get("audioDevice")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            None
        };

        // When `saveAudio` is on, each utterance's PCM is written as a WAV under
        // <app_data>/audio and linked to its message via the audio_clip table.
        let save_audio = settings
            .get("saveAudio")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let audio_dir = if save_audio {
            app.path().app_data_dir().ok().map(|d| d.join("audio"))
        } else {
            None
        };
        if let Some(dir) = &audio_dir {
            let _ = std::fs::create_dir_all(dir);
        }

        // Worker -> async processor: one PCM segment per utterance, tagged with
        // its end offset (ms since session start) so transcript rows carry a real
        // timeline instead of zeros.
        let (seg_tx, mut seg_rx) = unbounded_channel::<(Vec<f32>, i64)>();

        // STT + translation pipeline runs on the async runtime.
        let stt = provider::stt_from_settings(&settings);
        let translator = provider::translation_from_settings(&settings);
        // Per-session diarizer: labels are stable within this conversation only.
        let mut diarizer = provider::diarizer_from_settings(&settings);
        let hint = vec![lang_a.clone(), lang_b.clone()];
        let err_app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some((pcm, end_ms)) = seg_rx.recv().await {
                let dur_ms = (pcm.len() as f64 / SAMPLE_RATE as f64 * 1000.0) as i64;
                let start_ms = (end_ms - dur_ms).max(0);
                let transcript = match stt.transcribe(&pcm, SAMPLE_RATE, &hint).await {
                    Ok(t) => t,
                    Err(e) => {
                        log::error!("STT failed: {e}");
                        emit_error(&err_app, format!("Speech recognition failed: {e}"));
                        continue;
                    }
                };
                if transcript.text.is_empty() || is_noise(&transcript.text) {
                    continue;
                }
                // Route by detected language: translate into the *other* side.
                let (from, to) = if transcript.lang == lang_b {
                    (lang_b.clone(), lang_a.clone())
                } else {
                    (lang_a.clone(), lang_b.clone())
                };
                let context = recent_context(&db, &conv_id).await;
                let translated = match translator.translate(&transcript.text, &from, &to, &context).await {
                    Ok(t) => t,
                    Err(e) => {
                        log::error!("translation failed: {e}");
                        emit_error(&err_app, format!("Translation failed: {e}"));
                        String::new()
                    }
                };
                let speaker = diarizer.label(&pcm, SAMPLE_RATE);
                let now = now_ms();
                let msg = Message {
                    id: next_id(),
                    conversation_id: conv_id.clone(),
                    source: "audio".into(),
                    detected_lang: transcript.lang,
                    speaker,
                    original_text: transcript.text,
                    translated_text: translated,
                    start_ms,
                    end_ms,
                    created_at: now,
                };
                if let Err(e) = db.add_message(&msg).await {
                    log::error!("persist message failed: {e}");
                }
                if let Some(dir) = &audio_dir {
                    let path = dir.join(format!("{}.wav", msg.id));
                    let wav = crate::audio::encode_wav_pcm16(&pcm, SAMPLE_RATE);
                    match std::fs::write(&path, wav) {
                        Ok(()) => {
                            if let Err(e) = db
                                .add_audio_clip(&msg.id, &path.to_string_lossy(), dur_ms)
                                .await
                            {
                                log::error!("persist audio clip failed: {e}");
                            }
                        }
                        Err(e) => log::error!("write audio clip failed: {e}"),
                    }
                }
                if let Err(e) = app.emit("transcript-message", &msg) {
                    log::error!("emit failed: {e}");
                }
            }
        });

        // Controller thread owns the !Send cpal stream and parks until stopped.
        let (stop_tx, stop_rx) = channel::<()>();
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        std::thread::spawn(move || {
            // Marks t=0 of the recording; each segment is stamped with the wall
            // clock elapsed when VAD hands it over (its end), so rows get a real
            // timeline even though VAD drops the silence between utterances.
            let session_start = Instant::now();
            let capture =
                AudioCapture::start(source, device.as_deref(), VadConfig::default(), move |seg| {
                    let end_ms = session_start.elapsed().as_millis() as i64;
                    let _ = seg_tx.send((seg, end_ms));
                });
            match capture {
                Ok(cap) => {
                    let _ = ready_tx.send(Ok(()));
                    // Blocks until stop() drops the sender (Err) or signals it.
                    let _ = stop_rx.recv();
                    drop(cap); // stops the stream and flushes a trailing segment
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });

        ready_rx
            .recv()
            .map_err(|_| "capture controller thread died".to_string())??;

        *guard = Some(stop_tx);
        Ok(())
    }

    /// Stop the active session (no-op if idle). Dropping the stored sender wakes
    /// the controller thread, which drops the capture and flushes.
    pub fn stop(&self) {
        let mut guard = self.stop.lock().unwrap();
        *guard = None;
    }
}

/// Recent conversation text (last few turns) handed to the translator so it can
/// keep terminology, names and pronouns consistent across utterances. Empty on
/// any DB error or for a fresh conversation.
async fn recent_context(db: &Db, conv_id: &str) -> String {
    const MAX_TURNS: usize = 6;
    let msgs = match db.list_messages(conv_id).await {
        Ok(m) => m,
        Err(_) => return String::new(),
    };
    let start = msgs.len().saturating_sub(MAX_TURNS);
    msgs[start..]
        .iter()
        .map(|m| {
            if m.translated_text.is_empty() {
                m.original_text.clone()
            } else {
                format!("{} | {}", m.original_text, m.translated_text)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whisper emits non-speech placeholders such as `[BLANK_AUDIO]`, `[Music]`,
/// `[Japanese]` or `(speaking foreign language)` for segments it can't actually
/// transcribe (silence, noise, or speech in an unexpected language). These are
/// markers, not utterances — routing one into the transcript shows garbage on
/// the wrong side, so we drop any segment whose text is entirely one bracketed
/// or parenthesised token.
fn is_noise(text: &str) -> bool {
    let t = text.trim();
    if t.len() < 2 {
        return false;
    }
    let bracketed = (t.starts_with('[') && t.ends_with(']'))
        || (t.starts_with('(') && t.ends_with(')'))
        || (t.starts_with('*') && t.ends_with('*'));
    // Only a marker if there's a single token (no inner closer then more text),
    // e.g. "[Music]" but not "[John] said hi".
    bracketed && !t[1..t.len() - 1].contains(|c| matches!(c, '[' | ']' | '(' | ')'))
}

/// Push a non-fatal error to the UI (best-effort; failures to emit are ignored).
fn emit_error(app: &AppHandle, message: String) {
    let _ = app.emit(RECORDING_ERROR_EVENT, message);
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Monotonic, collision-free message id without pulling in a uuid crate.
fn next_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}{n:x}")
}
