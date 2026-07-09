//! Live recording orchestrator.
//!
//! Bridges the Tauri-independent audio pipeline ([`crate::audio`]) to the
//! providers and database: microphone -> VAD segments -> STT -> A/B-routed
//! translation -> persisted [`Message`] -> `transcript-message` event for the
//! UI to append in realtime.
//!
//! cpal's `Stream` is `!Send`, so the capture object lives on a dedicated
//! controller thread; segments cross thread boundaries as plain PCM.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::unbounded_channel;

use crate::audio::{AudioCapture, AudioSource, VadConfig, VadEvent, SAMPLE_RATE};
use crate::db::{Conversation, Db, Message};
use crate::provider;

/// Event name for non-fatal recording failures surfaced to the UI as a toast.
pub const RECORDING_ERROR_EVENT: &str = "recording-error";

/// Emitted the instant speech pauses, so the UI can draw a live "silence
/// countdown" bar that fills over `hangoverMs`; if the speaker resumes the bar
/// is aborted (`segment-cancelled`), otherwise it flows into the processing
/// phase below (docs/PROJECT.md §10.8).
const SEGMENT_SILENCE_EVENT: &str = "segment-silence";
/// Emitted when the segment finalizes and STT/translation begins, reusing the
/// silence placeholder's id so the UI swaps its countdown bar for a processing
/// shimmer.
const SEGMENT_PENDING_EVENT: &str = "segment-pending";
/// Emitted when a pending segment is dropped — the pause aborted (speech
/// resumed / too short) or STT yielded no usable text — so the UI removes its
/// placeholder.
const SEGMENT_CANCELLED_EVENT: &str = "segment-cancelled";

/// Pause began: the UI raises a silence-countdown bar that fills over
/// `hangover_ms`.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SegmentSilence {
    pending_id: u64,
    conversation_id: String,
    hangover_ms: u32,
}

/// Segment finalized; STT/translation now running for this placeholder.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SegmentPending {
    pending_id: u64,
    conversation_id: String,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SegmentCancelled {
    pending_id: u64,
}

/// `transcript-message` payload: the persisted message plus the `pending_id` it
/// resolves, flattened so the UI sees ordinary message fields next to it.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TranscriptMessage<'a> {
    #[serde(flatten)]
    message: &'a Message,
    pending_id: u64,
}

/// Session parameters parsed from the settings blob + conversation — extracted
/// from `Recorder::start` so the parsing is pure and unit-testable. Path
/// resolution for `saveAudio` stays in `start()` (it needs the `AppHandle`).
struct SessionConfig {
    source: AudioSource,
    device: Option<String>,
    vad_cfg: VadConfig,
    detect_foreign: bool,
    langs: Vec<String>,
    save_audio: bool,
}

/// A conversation's ordered language list (§10.7): the `langs` array, falling
/// back to `[lang_a, lang_b]` for rows that predate multi-language. DB-sourced
/// rows are already normalized by `row_to_conversation`; the fallback also
/// covers hand-built Conversations (tests) and keeps this module independent
/// of that invariant.
fn conv_langs(conv: &Conversation) -> Vec<String> {
    if conv.langs.is_empty() {
        vec![conv.lang_a.clone(), conv.lang_b.clone()]
    } else {
        conv.langs.clone()
    }
}

impl SessionConfig {
    fn from_settings(settings: &serde_json::Value, conv: &Conversation) -> Self {
        // Conversation languages (§10.7). For a 2-language chat this is just
        // [lang_a, lang_b]; with 3+ it drives N-way translation and the grid UI.
        let langs = conv_langs(conv);
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
        // Silence timing (§10.8). A fixed ~1.5 s grace passes silently (no bar)
        // so mid-sentence pauses don't flicker; then the countdown bar fills over
        // the user's "silence" setting (0.5–3 s) before the utterance finalizes.
        // Total hangover = grace + bar; the trailing silence is trimmed before
        // STT so the long wait never reaches whisper.
        const GRACE_MS: u64 = 1500;
        let frame_ms = crate::audio::vad::FRAME_MS as u64;
        let bar_ms = settings
            .get("silenceMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(3000)
            .clamp(500, 3000);
        let reveal_frames = (GRACE_MS / frame_ms) as usize;
        let vad_cfg = VadConfig {
            reveal_frames,
            end_frames: reveal_frames + (bar_ms / frame_ms) as usize,
            ..VadConfig::default()
        };
        // Language policy (§10.7). When off (default), every utterance is forced
        // onto one of the conversation languages — whisper's occasional misfire
        // (e.g. Russian heard as Turkish) no longer spawns a spurious "foreign"
        // row. When on, genuine third languages are kept (variant A).
        let detect_foreign = settings
            .get("detectForeignLanguages")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // When `saveAudio` is on, each utterance's PCM is written as a WAV under
        // <app_data>/audio and linked to its message via the audio_clip table.
        let save_audio = settings
            .get("saveAudio")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Self {
            source,
            device,
            vad_cfg,
            detect_foreign,
            langs,
            save_audio,
        }
    }
}

/// Everything the per-segment pipeline needs, bundled so the segment loop is a
/// plain function call instead of a 115-line closure body inside `start()`.
struct PipelineCtx {
    app: AppHandle,
    db: Db,
    conv_id: String,
    /// Conversation languages at session start — the fallback when the per-
    /// segment refresh can't reach the DB. `process_segment` re-reads the live
    /// list each utterance so a language added mid-recording (§10.7) starts
    /// being translated immediately, without restarting the session.
    langs: Vec<String>,
    detect_foreign: bool,
    stt: Box<dyn provider::SttProvider>,
    translator: Box<dyn provider::TranslationProvider>,
    audio_dir: Option<std::path::PathBuf>,
}

impl PipelineCtx {
    /// The conversation's CURRENT languages: re-read from the DB so mid-session
    /// changes (adding/removing a language in the UI) reach the pipeline.
    /// `None` means the conversation was deleted mid-recording — the segment
    /// must be dropped (persisting it would fail the FK anyway). A transient DB
    /// error falls back to the session-start snapshot.
    async fn live_langs(&self) -> Option<Vec<String>> {
        match self.db.get_conversation(&self.conv_id).await {
            Ok(Some(conv)) => Some(conv_langs(&conv)),
            Ok(None) => None,
            Err(e) => {
                log::error!("live langs re-read failed, using session snapshot: {e}");
                Some(self.langs.clone())
            }
        }
    }
}

/// Tauri-managed recording state. Holds the stop handle of the active session,
/// or `None` when idle.
#[derive(Default)]
pub struct Recorder {
    stop: Mutex<Option<Sender<()>>>,
}

impl Recorder {
    /// Lock the stop-handle, recovering from a poisoned lock instead of
    /// panicking (mirrors `Sidecars::lock` — a panic during `start()`'s long
    /// guard-held setup must not brick recording until app restart; the guarded
    /// `Option<Sender>` has no invariants poisoning protects).
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Sender<()>>> {
        self.stop.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_recording(&self) -> bool {
        self.lock().is_some()
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
        let mut guard = self.lock();
        if guard.is_some() {
            return Err("already recording".into());
        }

        let cfg = SessionConfig::from_settings(&settings, &conv);
        let conv_id = conv.id.clone();
        // saveAudio's target dir needs the AppHandle, so it resolves here rather
        // than in the pure SessionConfig parser.
        let audio_dir = if cfg.save_audio {
            app.path().app_data_dir().ok().map(|d| d.join("audio"))
        } else {
            None
        };
        if let Some(dir) = &audio_dir {
            let _ = std::fs::create_dir_all(dir);
        }

        // Worker -> async processor: one PCM segment per utterance, tagged with
        // its end offset (ms since session start) so transcript rows carry a real
        // timeline instead of zeros, plus a monotonic `pending_id` and the
        // `Instant` it left the VAD so the processor can drive the §10.8 latency
        // placeholder and stamp `processing_ms`.
        let (seg_tx, mut seg_rx) = unbounded_channel::<(Vec<f32>, i64, u64, Instant)>();

        // STT + translation pipeline runs on the async runtime.
        let ctx = PipelineCtx {
            app: app.clone(),
            db,
            conv_id: conv_id.clone(),
            langs: cfg.langs.clone(),
            detect_foreign: cfg.detect_foreign,
            stt: provider::stt_from_settings(&settings),
            translator: provider::translation_from_settings(&settings),
            audio_dir,
        };
        // Per-session diarizer: labels are stable within this conversation only.
        let mut diarizer = provider::diarizer_from_settings(&settings);
        // Optional speaker-turn segmenter (§10.15): cuts a VAD segment where the
        // speaker changes without a pause, so each turn is processed separately.
        let mut segmenter = provider::segmenter_from_settings(&settings);
        // Cloned for the controller thread (it emits silence/abort events itself —
        // they originate from the VAD on the worker thread, not the async pipeline).
        let ctl_app = app;
        let ctl_conv_id = conv_id;
        tauri::async_runtime::spawn(async move {
            while let Some((pcm, end_ms, pending_id, emitted_at)) = seg_rx.recv().await {
                process_segment(
                    &ctx,
                    &mut diarizer,
                    &mut segmenter,
                    pcm,
                    end_ms,
                    pending_id,
                    emitted_at,
                )
                .await;
            }
        });

        // Controller thread owns the !Send cpal stream and parks until stopped.
        // It also emits the silence/abort events directly (they originate from
        // the VAD on the worker thread, not the async STT pipeline), so it needs
        // its own AppHandle + conversation id.
        let (stop_tx, stop_rx) = channel::<()>();
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        std::thread::spawn(move || {
            // Marks t=0 of the recording; each segment is stamped with the wall
            // clock elapsed when VAD hands it over (its end), so rows get a real
            // timeline even though VAD drops the silence between utterances.
            let session_start = Instant::now();
            // Monotonic id per placeholder so the UI can match each silence bar to
            // the row that finalizes it (or the abort that drops it) (§10.8).
            let mut next_pending: u64 = 0;
            // The id of the silence bar currently on screen, allocated on the first
            // silent frame and reused by the segment that finalizes it.
            let mut current_silence_id: Option<u64> = None;
            let capture = AudioCapture::start(
                cfg.source,
                cfg.device.as_deref(),
                cfg.vad_cfg,
                move |event| match event {
                    VadEvent::SilenceStarted { hangover_ms } => {
                        next_pending += 1;
                        current_silence_id = Some(next_pending);
                        let _ = ctl_app.emit(
                            SEGMENT_SILENCE_EVENT,
                            SegmentSilence {
                                pending_id: next_pending,
                                conversation_id: ctl_conv_id.clone(),
                                hangover_ms,
                            },
                        );
                    }
                    VadEvent::PendingAborted => {
                        // Speech resumed, or a too-short segment was discarded:
                        // drop the on-screen silence bar.
                        if let Some(id) = current_silence_id.take() {
                            let _ = ctl_app
                                .emit(SEGMENT_CANCELLED_EVENT, SegmentCancelled { pending_id: id });
                        }
                    }
                    VadEvent::Segment(seg) => {
                        // Reuse the silence bar's id if this close followed a pause;
                        // a cap/flush close has none, so allocate a fresh one.
                        let pending_id = current_silence_id.take().unwrap_or_else(|| {
                            next_pending += 1;
                            next_pending
                        });
                        let end_ms = session_start.elapsed().as_millis() as i64;
                        let _ = seg_tx.send((seg, end_ms, pending_id, Instant::now()));
                    }
                },
            );
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
        let mut guard = self.lock();
        *guard = None;
    }
}

/// One VAD segment through the pipeline. Emits the processing placeholder,
/// re-reads the live language list, optionally cuts the segment at speaker
/// changes (§10.15), and runs each part as its own utterance. The placeholder
/// is resolved by the first part's `transcript-message`; if NO part produced a
/// message (all noise / STT failed / conversation deleted), it is cancelled.
async fn process_segment(
    ctx: &PipelineCtx,
    diarizer: &mut Box<dyn provider::diarize::Diarizer>,
    segmenter: &mut Option<provider::segment::OnnxSegmenter>,
    pcm: Vec<f32>,
    end_ms: i64,
    pending_id: u64,
    emitted_at: Instant,
) {
    let dur_ms = (pcm.len() as f64 / SAMPLE_RATE as f64 * 1000.0) as i64;
    let seg_start_ms = (end_ms - dur_ms).max(0);
    // The segment finalized: swap the UI's silence-countdown bar for a
    // processing shimmer while STT + translation run (§10.8).
    let _ = ctx.app.emit(
        SEGMENT_PENDING_EVENT,
        SegmentPending {
            pending_id,
            conversation_id: ctx.conv_id.clone(),
        },
    );
    // Re-read the conversation's languages so a language added mid-recording
    // is picked up from this utterance on (§10.7) — no session restart needed.
    // `None` = the conversation was deleted mid-recording: drop the segment
    // (its placeholder included) instead of emitting a message the DB would
    // reject on the conversation FK.
    let Some(langs) = ctx.live_langs().await else {
        let _ = ctx
            .app
            .emit(SEGMENT_CANCELLED_EVENT, SegmentCancelled { pending_id });
        return;
    };
    // Speaker-turn cuts (§10.15): [0, cuts.., len] → part ranges. Without a
    // segmenter the segment is processed whole, exactly as before. The ONNX
    // inference is CPU-bound (~0.1–1 s for a long segment), so it hops to a
    // blocking thread instead of stalling the shared async runtime.
    let (cuts, pcm) = match segmenter.take() {
        Some(mut s) => {
            match tauri::async_runtime::spawn_blocking(move || {
                let cuts = s.change_points(&pcm);
                (cuts, pcm, s)
            })
            .await
            {
                Ok((cuts, pcm, s)) => {
                    *segmenter = Some(s);
                    (cuts, pcm)
                }
                Err(e) => {
                    // The task panicked and took pcm with it — drop the segment
                    // (and its placeholder); the segmenter stays disabled.
                    log::error!("speaker segmentation task failed: {e}");
                    let _ = ctx
                        .app
                        .emit(SEGMENT_CANCELLED_EVENT, SegmentCancelled { pending_id });
                    return;
                }
            }
        }
        None => (Vec::new(), pcm),
    };
    if !cuts.is_empty() {
        // Diagnosability: when phrases look truncated in the transcript, the log
        // shows whether (and where) the segmenter cut them.
        let at_ms: Vec<i64> = cuts
            .iter()
            .map(|&c| (c as f64 / SAMPLE_RATE as f64 * 1000.0) as i64)
            .collect();
        log::info!(
            "speaker segmentation: {} cut(s) in a {} ms segment at {:?} ms",
            cuts.len(),
            dur_ms,
            at_ms
        );
    }
    let mut bounds = Vec::with_capacity(cuts.len() + 2);
    bounds.push(0);
    bounds.extend(cuts);
    bounds.push(pcm.len());
    let ms_of = |sample: usize| {
        seg_start_ms + (sample as f64 / SAMPLE_RATE as f64 * 1000.0) as i64
    };
    // Per-part latency baseline: part 1 measures from VAD release (the classic
    // §10.8 meaning); later parts from the previous part's finish, so each row
    // reports its own incremental wait rather than a cumulative sum.
    let mut part_started = emitted_at;
    let last = bounds.len().saturating_sub(2);
    for (i, pair) in bounds.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        if a >= b {
            continue;
        }
        let emitted = process_utterance(
            ctx,
            diarizer,
            &langs,
            &pcm[a..b],
            ms_of(a),
            ms_of(b),
            pending_id,
            part_started,
        )
        .await;
        part_started = Instant::now();
        // The part's message resolved the placeholder; re-raise the processing
        // shimmer while the remaining parts run so the UI still shows activity.
        if emitted && i < last {
            let _ = ctx.app.emit(
                SEGMENT_PENDING_EVENT,
                SegmentPending {
                    pending_id,
                    conversation_id: ctx.conv_id.clone(),
                },
            );
        }
    }
    // Unconditional: clears the original placeholder when NO part produced a
    // message, or a re-raised shimmer whose trailing part turned out to be
    // noise. A no-op when the last message already resolved it.
    let _ = ctx
        .app
        .emit(SEGMENT_CANCELLED_EVENT, SegmentCancelled { pending_id });
}

/// One utterance (a whole VAD segment, or one speaker turn of it) through the
/// pipeline: STT, noise filter, language resolution, N-way translation,
/// diarization, persist, optional WAV clip, and the `transcript-message` event.
/// Returns whether a message was emitted (resolving the §10.8 placeholder).
#[allow(clippy::too_many_arguments)]
async fn process_utterance(
    ctx: &PipelineCtx,
    diarizer: &mut Box<dyn provider::diarize::Diarizer>,
    langs: &[String],
    pcm: &[f32],
    start_ms: i64,
    end_ms: i64,
    pending_id: u64,
    emitted_at: Instant,
) -> bool {
    let dur_ms = end_ms - start_ms;
    // The conversation languages double as whisper's decoding hint.
    let transcript = match ctx.stt.transcribe(pcm, SAMPLE_RATE, langs).await {
        Ok(t) => t,
        Err(e) => {
            log::error!("STT failed: {e}");
            emit_error(&ctx.app, format!("Speech recognition failed: {e}"));
            return false;
        }
    };
    if transcript.text.is_empty() || is_noise(&transcript.text) {
        return false;
    }
    let context = recent_context(&ctx.db, &ctx.conv_id).await;
    // Helper: translate, logging+toasting on failure (empty on error).
    let do_translate = |from: String, to: String| {
        let translator = &ctx.translator;
        let text = &transcript.text;
        let context = &context;
        let err_app = &ctx.app;
        async move {
            match translator.translate(text, &from, &to, context).await {
                Ok(t) => t,
                Err(e) => {
                    log::error!("translation failed: {e}");
                    emit_error(err_app, format!("Translation failed: {e}"));
                    String::new()
                }
            }
        }
    };
    // Resolve whisper's detected language against the conversation
    // languages, snapping obvious misfires by script and (unless
    // foreign detection is on) forcing the result onto one of them.
    let detected = resolve_lang_n(
        &transcript.lang,
        &transcript.text,
        langs,
        ctx.detect_foreign,
    );
    // Translate the utterance into every *other* conversation language
    // (docs/PROJECT.md §10.7). For 2 languages this is a single
    // translation; with 3+ it fans out, one row per target language
    // stored in `message_translation`.
    let mut translations: HashMap<String, String> = HashMap::new();
    for target in langs {
        if target == &detected {
            continue;
        }
        let t = do_translate(detected.clone(), target.clone()).await;
        if !t.is_empty() {
            translations.insert(target.clone(), t);
        }
    }
    // Back-compat scalar fields for the 2-column view / export:
    //  - pair utterance → `translated_text` = the other language.
    //  - foreign utterance (outside the pair) → `translated_text` =
    //    lang_a translation, `translated_text_b` = lang_b translation.
    //  - 3+ languages → `translated_text` = first other language; the
    //    grid UI reads `translations` instead.
    let (translated, translated_b) = back_compat_translations(&detected, langs, &translations);
    let speaker = diarizer.label(pcm, SAMPLE_RATE);
    let now = now_ms();
    // Whole-pipeline latency: from the moment VAD released the
    // segment to now (STT + translation done, about to persist).
    let processing_ms = emitted_at.elapsed().as_millis() as i64;
    let msg = Message {
        id: next_id(),
        conversation_id: ctx.conv_id.clone(),
        source: "audio".into(),
        detected_lang: detected,
        speaker,
        original_text: transcript.text,
        translated_text: translated,
        translated_text_b: translated_b,
        start_ms,
        end_ms,
        created_at: now,
        processing_ms: Some(processing_ms),
        translations,
    };
    if let Err(e) = ctx.db.add_message(&msg).await {
        log::error!("persist message failed: {e}");
    }
    if let Some(dir) = &ctx.audio_dir {
        let path = dir.join(format!("{}.wav", msg.id));
        let wav = crate::audio::encode_wav_pcm16(pcm, SAMPLE_RATE);
        match std::fs::write(&path, wav) {
            Ok(()) => {
                if let Err(e) = ctx
                    .db
                    .add_audio_clip(&msg.id, &path.to_string_lossy(), dur_ms)
                    .await
                {
                    log::error!("persist audio clip failed: {e}");
                }
            }
            Err(e) => log::error!("write audio clip failed: {e}"),
        }
    }
    // Carry `pending_id` alongside the message so the UI replaces the
    // exact placeholder it raised on `segment-pending` (§10.8).
    if let Err(e) = ctx.app.emit(
        "transcript-message",
        TranscriptMessage {
            message: &msg,
            pending_id,
        },
    ) {
        log::error!("emit failed: {e}");
    }
    true
}

/// Recent conversation text (last few turns) handed to the translator so it can
/// keep terminology, names and pronouns consistent across utterances. Empty on
/// any DB error or for a fresh conversation. Runs once per finalized segment, so
/// it uses the LIMIT-ed query instead of scanning the whole conversation.
async fn recent_context(db: &Db, conv_id: &str) -> String {
    const MAX_TURNS: i64 = 6;
    let rows = match db.recent_message_texts(conv_id, MAX_TURNS).await {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    rows.iter()
        .map(|(original, translated)| {
            if translated.is_empty() {
                original.clone()
            } else {
                format!("{original} | {translated}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whisper emits non-speech placeholders such as `[BLANK_AUDIO]`, `[Music]`,
/// `[Japanese]` or `(speaking foreign language)` for segments it can't actually
/// transcribe (silence, noise, or speech in an unexpected language). It also
/// transcribes throat-clears and hesitations literally ("ahem", "cough", "um").
/// These are markers, not utterances — routing one into the transcript shows
/// garbage on the wrong side, so we drop a segment that is *only* such filler.
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
    if bracketed && !t[1..t.len() - 1].contains(|c| matches!(c, '[' | ']' | '(' | ')')) {
        return true;
    }
    // Non-lexical interjections: drop a short segment made up only of fillers
    // (a cough, throat-clear, "um"…) which would otherwise clutter the transcript
    // and get pointlessly "translated". Punctuation is stripped per word.
    let words: Vec<String> = t
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    !words.is_empty() && words.len() <= 3 && words.iter().all(|w| is_filler(w))
}

/// A single non-lexical filler / hesitation token (whole-segment matches only,
/// so real words inside a sentence are never dropped). Kept conservative — no
/// meaningful short words like "да"/"yes"/"ok".
fn is_filler(w: &str) -> bool {
    matches!(
        w,
        "ahem"
            | "cough"
            | "coughs"
            | "coughing"
            | "um"
            | "umm"
            | "uh"
            | "uhh"
            | "uhm"
            | "erm"
            | "er"
            | "hmm"
            | "hm"
            | "hmmm"
            | "mm"
            | "mmm"
            | "mhm"
            | "huh"
            | "кхм"
            | "кх"
            | "эм"
            | "ээ"
            | "эээ"
            | "мхм"
            | "кашель"
            // Japanese hesitations (whole-segment only; word-like ones such as
            // あの/その are deliberately excluded to avoid dropping real speech).
            | "えっと"
            | "ええと"
            | "えーと"
            | "えー"
            | "あー"
            | "うー"
            | "うーん"
            | "んー"
    )
}

/// The writing system a piece of text is mostly in. Used to sanity-check (and
/// override) whisper's claimed language: a short or noisy segment is sometimes
/// mislabelled (Russian heard as Turkish), but the *script* of the decoded text
/// is a strong, cheap signal of the real language family.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    Cyrillic,
    Japanese, // kana (uniquely JP) or kanji
    Hangul,
    Arabic,
    Latin,
    Other,
}

/// Dominant script of `text`, by counting letters in each Unicode range.
fn dominant_script(text: &str) -> Script {
    let (mut cyr, mut jp, mut han, mut ar, mut lat) = (0u32, 0u32, 0u32, 0u32, 0u32);
    for c in text.chars() {
        match c {
            '\u{0400}'..='\u{04FF}' => cyr += 1,
            // Hiragana / Katakana — unique to Japanese.
            '\u{3040}'..='\u{30FF}' => jp += 2,
            // CJK ideographs — kanji (also Chinese); weighted lighter than kana.
            '\u{4E00}'..='\u{9FFF}' => jp += 1,
            '\u{AC00}'..='\u{D7AF}' => han += 1,
            '\u{0600}'..='\u{06FF}' => ar += 1,
            'A'..='Z' | 'a'..='z' => lat += 1,
            _ => {}
        }
    }
    let max = cyr.max(jp).max(han).max(ar).max(lat);
    if max == 0 {
        return Script::Other;
    }
    if max == cyr {
        Script::Cyrillic
    } else if max == jp {
        Script::Japanese
    } else if max == han {
        Script::Hangul
    } else if max == ar {
        Script::Arabic
    } else {
        Script::Latin
    }
}

/// Whether a script is plausible for an ISO language code. Only the codes the
/// app routinely sees are special-cased; everything else is treated as Latin.
fn script_fits_lang(script: Script, lang: &str) -> bool {
    let expected = match lang {
        "ru" | "uk" | "be" | "bg" | "sr" | "mk" => Script::Cyrillic,
        "ja" => Script::Japanese,
        "zh" => Script::Japanese, // shares kanji/CJK range
        "ko" => Script::Hangul,
        "ar" | "fa" | "ur" => Script::Arabic,
        _ => Script::Latin, // en, es, fr, de, it, pt, pl, tr, …
    };
    script == expected
}

/// Decide the real language of an utterance from whisper's claim plus the script
/// of the decoded text and the conversation pair.
///
/// - If `detect_foreign` is off (default), the result is always `lang_a`/`lang_b`:
///   pick the pair language whose script matches the text; if neither matches,
///   keep whisper's guess when it is already in the pair, else fall back to
///   `lang_a`. This stops misfires (Russian → "tr") from spawning fake foreign rows.
/// - If on, a genuine third language is kept — but a claim that contradicts the
///   text script is still corrected toward a matching pair language (e.g. Cyrillic
///   text labelled "tr" becomes `ru`).
#[cfg(test)]
fn resolve_lang(detected: &str, text: &str, lang_a: &str, lang_b: &str, detect_foreign: bool) -> String {
    resolve_lang_n(
        detected,
        text,
        &[lang_a.to_string(), lang_b.to_string()],
        detect_foreign,
    )
}

/// N-language generalization of [`resolve_lang`] (§10.7). Picks the conversation
/// language for an utterance from whisper's claim plus the decoded text's script.
/// With `detect_foreign` off the result is always one of `langs`; with it on a
/// genuine outside language is kept unless the script unambiguously points at a
/// single conversation language. For a 2-element `langs` this is identical to the
/// original pair logic (the `resolve_lang` tests still exercise that path).
fn resolve_lang_n(detected: &str, text: &str, langs: &[String], detect_foreign: bool) -> String {
    let script = dominant_script(text);
    // Conversation languages whose writing system matches the decoded text.
    let fits: Vec<&String> = langs
        .iter()
        .filter(|l| script_fits_lang(script, l))
        .collect();
    let in_set = langs.iter().any(|l| l == detected);

    if detect_foreign {
        // Trust whisper, but correct a clear contradiction: text whose script
        // matches exactly one conversation language overrides a label outside
        // the set (e.g. Cyrillic tagged "tr" → ru).
        if !in_set && fits.len() == 1 {
            return fits[0].clone();
        }
        return detected.to_string();
    }

    // Locked to the set: snap to the single matching language if unambiguous,
    // else keep whisper's guess when it is already in the set, else the first
    // matching language, else the first conversation language.
    if fits.len() == 1 {
        return fits[0].clone();
    }
    if in_set {
        return detected.to_string();
    }
    if let Some(f) = fits.first() {
        return (*f).clone();
    }
    langs
        .first()
        .cloned()
        .unwrap_or_else(|| detected.to_string())
}

/// Derive the legacy scalar translation columns from the per-language map so the
/// 2-column transcript view, the foreign-row layout, and Markdown export keep
/// working unchanged while the N-language grid reads `translations` (§10.7).
fn back_compat_translations(
    detected: &str,
    langs: &[String],
    translations: &HashMap<String, String>,
) -> (String, Option<String>) {
    let get = |l: &str| translations.get(l).cloned().unwrap_or_default();
    if langs.len() > 2 {
        // N-language: the first other language seeds the legacy column; the grid
        // UI reads the full map instead.
        let first_other = langs.iter().find(|l| l.as_str() != detected);
        return (first_other.map(|l| get(l)).unwrap_or_default(), None);
    }
    let a = langs.first().map(String::as_str).unwrap_or("");
    let b = langs.get(1).map(String::as_str);
    let in_pair = detected == a || b == Some(detected);
    if in_pair {
        // Pair utterance: translated into the other pair language.
        let other = if detected == a { b } else { Some(a) };
        (other.map(get).unwrap_or_default(), None)
    } else {
        // Foreign utterance: lang_a translation in the primary column, lang_b in
        // the secondary (variant A layout).
        (get(a), b.map(get))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filler_only_segments_are_noise() {
        assert!(is_noise("ahem"));
        assert!(is_noise("cough"));
        assert!(is_noise("(coughs)"));
        assert!(is_noise("Um, uh"));
        assert!(is_noise("кхм"));
        assert!(is_noise("[Music]"));
        // Real utterances must survive, including ones that contain a filler word.
        assert!(!is_noise("um, let's go to Tokyo"));
        assert!(!is_noise("да"));
        assert!(!is_noise("привет как дела"));
        assert!(!is_noise("okay"));
    }

    #[test]
    fn script_detection_basic() {
        assert_eq!(dominant_script("привет как дела"), Script::Cyrillic);
        assert_eq!(dominant_script("こんにちは"), Script::Japanese);
        assert_eq!(dominant_script("注文は完了"), Script::Japanese); // kanji
        assert_eq!(dominant_script("hello world"), Script::Latin);
        assert_eq!(dominant_script("123 …"), Script::Other);
    }

    #[test]
    fn locked_snaps_misdetected_russian_to_pair() {
        // The real bug: Russian audio decoded as Turkish text would have spawned
        // a fake "tr" foreign row. But once whisper mislabels Russian as Turkish
        // the *text* is Latin gibberish, so script can't save that case — the
        // pair-lock fallback must still keep it off a foreign row.
        // Cyrillic text mislabelled "tr" → snapped to ru by script.
        assert_eq!(resolve_lang("tr", "в чумон вакуума", "ru", "ja", false), "ru");
        // Latin gibberish mislabelled "tr", locked: not in pair, no script match
        // for ja, Latin doesn't fit ru/ja → falls back to lang_a (ru).
        assert_eq!(resolve_lang("tr", "Sumun kivrita", "ru", "ja", false), "ru");
    }

    #[test]
    fn locked_keeps_correct_pair_languages() {
        assert_eq!(resolve_lang("ru", "привет", "ru", "ja", false), "ru");
        assert_eq!(resolve_lang("ja", "こんにちは", "ru", "ja", false), "ja");
    }

    #[test]
    fn foreign_on_keeps_genuine_third_language_but_fixes_script() {
        // Genuine third language kept when foreign detection is on.
        assert_eq!(resolve_lang("fr", "bonjour le monde", "ru", "ja", true), "fr");
        // …but Cyrillic text wrongly tagged "tr" is still corrected to ru.
        assert_eq!(resolve_lang("tr", "это русский", "ru", "ja", true), "ru");
    }

    fn test_conv(langs: Vec<String>) -> Conversation {
        Conversation {
            id: "c".into(),
            title: "t".into(),
            lang_a: "ru".into(),
            lang_b: "en".into(),
            langs,
            speaker_names: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn session_config_defaults_and_lang_fallback() {
        let cfg = SessionConfig::from_settings(&serde_json::json!({}), &test_conv(vec![]));
        // Empty langs falls back to the [lang_a, lang_b] pair.
        assert_eq!(cfg.langs, vec!["ru", "en"]);
        assert!(!cfg.detect_foreign); // off by default (§10.7)
        assert!(!cfg.save_audio);
        assert_eq!(cfg.source, AudioSource::Mic);
        assert_eq!(cfg.device, None);
    }

    #[test]
    fn session_config_clamps_silence_ms() {
        let frame_ms = crate::audio::vad::FRAME_MS as u64;
        let grace_frames = (1500 / frame_ms) as usize;
        // Below the floor → clamped to 500 ms of bar time.
        let lo = SessionConfig::from_settings(
            &serde_json::json!({ "silenceMs": 100 }),
            &test_conv(vec![]),
        );
        assert_eq!(lo.vad_cfg.end_frames, grace_frames + (500 / frame_ms) as usize);
        // Above the ceiling → clamped to 3000 ms.
        let hi = SessionConfig::from_settings(
            &serde_json::json!({ "silenceMs": 60000 }),
            &test_conv(vec![]),
        );
        assert_eq!(hi.vad_cfg.end_frames, grace_frames + (3000 / frame_ms) as usize);
    }
}
