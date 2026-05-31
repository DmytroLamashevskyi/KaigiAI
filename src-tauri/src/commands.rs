//! Tauri command wrappers — deliberately thin pass-throughs over the core
//! `Db` service so the same persistence logic can later be exposed over HTTP/WS
//! by an Axum server without rewriting anything.

use tauri::{AppHandle, State};

use crate::db::{Bootstrap, Conversation, Db, Message};
use crate::provider;
use crate::recording::Recorder;
use crate::sidecar::Sidecars;

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn is_local(settings: &serde_json::Value) -> bool {
    settings.get("providerMode").and_then(|v| v.as_str()) == Some("local")
}

fn set_str(settings: &mut serde_json::Value, key: &str, val: String) {
    if let Some(obj) = settings.as_object_mut() {
        obj.insert(key.to_string(), serde_json::Value::String(val));
    }
}

#[tauri::command]
pub async fn bootstrap(db: State<'_, Db>) -> CmdResult<Bootstrap> {
    db.bootstrap().await.map_err(err)
}

#[tauri::command]
pub async fn list_messages(db: State<'_, Db>, conversation_id: String) -> CmdResult<Vec<Message>> {
    db.list_messages(&conversation_id).await.map_err(err)
}

#[tauri::command]
pub async fn create_conversation(db: State<'_, Db>, conversation: Conversation) -> CmdResult<()> {
    db.create_conversation(&conversation).await.map_err(err)
}

#[tauri::command]
pub async fn rename_conversation(db: State<'_, Db>, id: String, title: String, updated_at: i64) -> CmdResult<()> {
    db.rename_conversation(&id, &title, updated_at).await.map_err(err)
}

#[tauri::command]
pub async fn set_conversation_langs(db: State<'_, Db>, id: String, lang_a: String, lang_b: String, updated_at: i64) -> CmdResult<()> {
    db.set_conversation_langs(&id, &lang_a, &lang_b, updated_at).await.map_err(err)
}

#[tauri::command]
pub async fn delete_conversation(db: State<'_, Db>, id: String) -> CmdResult<()> {
    db.delete_conversation(&id).await.map_err(err)
}

#[tauri::command]
pub async fn add_message(db: State<'_, Db>, message: Message) -> CmdResult<()> {
    db.add_message(&message).await.map_err(err)
}

#[tauri::command]
pub async fn save_settings(db: State<'_, Db>, settings: serde_json::Value) -> CmdResult<()> {
    db.save_settings(&settings).await.map_err(err)
}

#[tauri::command]
pub async fn translate_text(
    db: State<'_, Db>,
    sidecars: State<'_, Sidecars>,
    text: String,
    from: String,
    to: String,
) -> CmdResult<String> {
    let mut settings = db.get_app_settings().await.map_err(err)?;
    if is_local(&settings) {
        let url = sidecars.ensure_llama(&settings)?;
        set_str(&mut settings, "localLlmBaseUrl", url);
    }
    provider::translation_from_settings(&settings)
        .translate(&text, &from, &to)
        .await
}

#[tauri::command]
pub async fn summarize_conversation(
    db: State<'_, Db>,
    sidecars: State<'_, Sidecars>,
    conversation_id: String,
    lang: String,
) -> CmdResult<String> {
    let messages = db.list_messages(&conversation_id).await.map_err(err)?;
    let transcript = messages
        .iter()
        .map(|m| {
            if m.translated_text.is_empty() {
                m.original_text.clone()
            } else {
                format!("{}\n{}", m.original_text, m.translated_text)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut settings = db.get_app_settings().await.map_err(err)?;
    if is_local(&settings) {
        let url = sidecars.ensure_llama(&settings)?;
        set_str(&mut settings, "localLlmBaseUrl", url);
    }
    provider::translation_from_settings(&settings)
        .summarize(&transcript, &lang)
        .await
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    db: State<'_, Db>,
    recorder: State<'_, Recorder>,
    sidecars: State<'_, Sidecars>,
    conversation_id: String,
) -> CmdResult<()> {
    let conv = db
        .get_conversation(&conversation_id)
        .await
        .map_err(err)?
        .ok_or_else(|| "conversation not found".to_string())?;
    let mut settings = db.get_app_settings().await.map_err(err)?;
    if is_local(&settings) {
        let stt = sidecars.ensure_whisper(&settings)?;
        let llm = sidecars.ensure_llama(&settings)?;
        set_str(&mut settings, "localSttBaseUrl", stt);
        set_str(&mut settings, "localLlmBaseUrl", llm);
    }
    recorder.start(app, db.inner().clone(), conv, settings)
}

#[tauri::command]
pub fn stop_recording(recorder: State<'_, Recorder>) -> CmdResult<()> {
    recorder.stop();
    Ok(())
}

#[tauri::command]
pub fn is_recording(recorder: State<'_, Recorder>) -> bool {
    recorder.is_recording()
}

#[tauri::command]
pub fn list_audio_devices() -> Vec<String> {
    crate::audio::list_input_devices()
}
