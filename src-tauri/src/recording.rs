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
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};
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

        // Worker -> async processor: one PCM segment per utterance.
        let (seg_tx, mut seg_rx) = unbounded_channel::<Vec<f32>>();

        // STT + translation pipeline runs on the async runtime.
        let stt = provider::stt_from_settings(&settings);
        let translator = provider::translation_from_settings(&settings);
        let hint = vec![lang_a.clone(), lang_b.clone()];
        let err_app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(pcm) = seg_rx.recv().await {
                let transcript = match stt.transcribe(&pcm, SAMPLE_RATE, &hint).await {
                    Ok(t) => t,
                    Err(e) => {
                        log::error!("STT failed: {e}");
                        emit_error(&err_app, format!("Speech recognition failed: {e}"));
                        continue;
                    }
                };
                if transcript.text.is_empty() {
                    continue;
                }
                // Route by detected language: translate into the *other* side.
                let (from, to) = if transcript.lang == lang_b {
                    (lang_b.clone(), lang_a.clone())
                } else {
                    (lang_a.clone(), lang_b.clone())
                };
                let translated = match translator.translate(&transcript.text, &from, &to).await {
                    Ok(t) => t,
                    Err(e) => {
                        log::error!("translation failed: {e}");
                        emit_error(&err_app, format!("Translation failed: {e}"));
                        String::new()
                    }
                };
                let now = now_ms();
                let msg = Message {
                    id: next_id(),
                    conversation_id: conv_id.clone(),
                    source: "audio".into(),
                    detected_lang: transcript.lang,
                    speaker: None,
                    original_text: transcript.text,
                    translated_text: translated,
                    start_ms: 0,
                    end_ms: 0,
                    created_at: now,
                };
                if let Err(e) = db.add_message(&msg).await {
                    log::error!("persist message failed: {e}");
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
            let capture =
                AudioCapture::start(source, device.as_deref(), VadConfig::default(), move |seg| {
                    let _ = seg_tx.send(seg);
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
