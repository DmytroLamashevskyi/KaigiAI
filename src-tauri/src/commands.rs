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

fn set_str(settings: &mut serde_json::Value, key: &str, val: String) {
    if let Some(obj) = settings.as_object_mut() {
        obj.insert(key.to_string(), serde_json::Value::String(val));
    }
}

/// Re-populate `apiKey` from the OS keychain. The key is stripped before the
/// settings blob is persisted (see [`save_settings`]), so any settings read
/// from the DB carries an empty `apiKey`; this restores it just before a
/// provider is built. Without it `provider::is_api` would never select the API
/// provider and the app would silently fall back to the mock.
fn inject_api_key(settings: &mut serde_json::Value) {
    if let Some(key) = crate::keychain::get_api_key() {
        set_str(settings, "apiKey", key);
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
pub async fn set_speaker_names(db: State<'_, Db>, id: String, names_json: String, updated_at: i64) -> CmdResult<()> {
    db.set_speaker_names(&id, &names_json, updated_at).await.map_err(err)
}

#[tauri::command]
pub async fn set_message_speaker(db: State<'_, Db>, message_id: String, label: Option<String>) -> CmdResult<()> {
    db.set_message_speaker(&message_id, label.as_deref()).await.map_err(err)
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
pub async fn save_settings(db: State<'_, Db>, mut settings: serde_json::Value) -> CmdResult<()> {
    // Move the API key into the OS keychain and never persist it in plaintext.
    // An empty/absent key leaves the stored secret untouched (the UI sends an
    // empty field after reload, which must not wipe a previously saved key).
    let api_key = settings
        .get("apiKey")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if !api_key.is_empty() {
        crate::keychain::set_api_key(&api_key);
    }
    set_str(&mut settings, "apiKey", String::new());
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
    inject_api_key(&mut settings);
    if provider::translation_is_local(&settings) {
        let url = sidecars.ensure_llama(&settings)?;
        set_str(&mut settings, "localLlmBaseUrl", url);
    }
    provider::translation_from_settings(&settings)
        .translate(&text, &from, &to, "")
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
    inject_api_key(&mut settings);
    if provider::translation_is_local(&settings) {
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
    inject_api_key(&mut settings);
    // Spawn only the sidecars the chosen modes need: whisper for local STT,
    // llama for local translation. A mixed setup (e.g. local speech + cloud
    // translation) starts just one.
    if provider::stt_is_local(&settings) {
        let stt = sidecars.ensure_whisper(&settings)?;
        set_str(&mut settings, "localSttBaseUrl", stt);
    }
    if provider::translation_is_local(&settings) {
        let llm = sidecars.ensure_llama(&settings)?;
        set_str(&mut settings, "localLlmBaseUrl", llm);
    }
    recorder.start(app, db.inner().clone(), conv, settings)
}

#[tauri::command]
pub fn stop_recording(recorder: State<'_, Recorder>) -> CmdResult<()> {
    recorder.stop();
    Ok(())
}

/// Pre-start the local sidecar servers the current settings need, without
/// recording. Lets the UI "warm up" on launch (or on demand) so the first
/// recording isn't blocked by the multi-second model load. No-op for cloud-only
/// setups. Returns once the needed servers report ready.
#[tauri::command]
pub async fn warmup_servers(db: State<'_, Db>, sidecars: State<'_, Sidecars>) -> CmdResult<()> {
    let mut settings = db.get_app_settings().await.map_err(err)?;
    inject_api_key(&mut settings);
    if provider::stt_is_local(&settings) {
        sidecars.ensure_whisper(&settings)?;
    }
    if provider::translation_is_local(&settings) {
        sidecars.ensure_llama(&settings)?;
    }
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

/// Open an external URL in the user's default browser. A bare `<a target=_blank>`
/// in the Tauri webview doesn't reach the system browser, so the "Get key" /
/// download links route through here. Restricted to http(s) so the command can't
/// be coaxed into launching arbitrary files or programs.
#[tauri::command]
pub fn open_url(url: String) -> CmdResult<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs may be opened".into());
    }
    open_external(&url)
}

#[cfg(target_os = "windows")]
fn open_external(url: &str) -> CmdResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // `cmd /C start "" <url>` — the empty "" is start's title arg so a URL with
    // spaces isn't mistaken for the window title.
    std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(err)
}

#[cfg(target_os = "macos")]
fn open_external(url: &str) -> CmdResult<()> {
    std::process::Command::new("open").arg(url).spawn().map(|_| ()).map_err(err)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_external(url: &str) -> CmdResult<()> {
    std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ()).map_err(err)
}
