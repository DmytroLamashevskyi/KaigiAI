//! Tauri command wrappers — deliberately thin pass-throughs over the core
//! `Db` service so the same persistence logic can later be exposed over HTTP/WS
//! by an Axum server without rewriting anything.

use tauri::{AppHandle, Manager, State};

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

/// Replace a conversation's full ordered language list (§10.7 N-language mode).
/// The first two entries are mirrored into `lang_a`/`lang_b` for back-compat.
#[tauri::command]
pub async fn set_languages(db: State<'_, Db>, id: String, langs: Vec<String>, updated_at: i64) -> CmdResult<()> {
    db.set_languages(&id, &langs, updated_at).await.map_err(err)
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
    if !api_key.is_empty()
        && crate::keychain::get_api_key().as_deref() != Some(api_key.as_str())
    {
        // Only touch the OS keychain when the key actually changed — otherwise
        // every unrelated settings edit re-writes the secret needlessly.
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
    let mut transcript = messages
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
    // Cap a very long meeting so it doesn't blow the LLM context / time out; keep
    // the most recent text (truncate from the front on a char boundary).
    const MAX_SUMMARY_CHARS: usize = 24_000;
    if transcript.chars().count() > MAX_SUMMARY_CHARS {
        let start = transcript
            .char_indices()
            .nth(transcript.chars().count() - MAX_SUMMARY_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(0);
        transcript = format!("…\n{}", &transcript[start..]);
    }
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

/// Generate a short conversation title with the translation LLM, from the
/// transcript so far. Returns the title; the frontend persists it via rename.
#[tauri::command]
pub async fn generate_title(
    db: State<'_, Db>,
    sidecars: State<'_, Sidecars>,
    conversation_id: String,
    lang: String,
) -> CmdResult<String> {
    let messages = db.list_messages(&conversation_id).await.map_err(err)?;
    if messages.is_empty() {
        return Err("nothing to title yet".into());
    }
    // A compact transcript (cap length so a long meeting doesn't blow the prompt).
    let mut transcript = String::new();
    for m in &messages {
        transcript.push_str(&m.original_text);
        transcript.push('\n');
        if transcript.len() > 4000 {
            break;
        }
    }
    let mut settings = db.get_app_settings().await.map_err(err)?;
    inject_api_key(&mut settings);
    if provider::translation_is_local(&settings) {
        let url = sidecars.ensure_llama(&settings)?;
        set_str(&mut settings, "localLlmBaseUrl", url);
    }
    provider::translation_from_settings(&settings)
        .title(&transcript, &lang)
        .await
}

/// Export a conversation as a ZIP containing the markdown transcript plus any
/// saved audio clips. `dest_dir` is the user's export folder (empty → a default
/// under app data). Returns the written ZIP path.
#[tauri::command]
pub async fn export_zip(
    app: AppHandle,
    db: State<'_, Db>,
    conversation_id: String,
    title: String,
    markdown: String,
    dest_dir: String,
) -> CmdResult<String> {
    use std::io::Write;
    let clips = db.list_audio_clips(&conversation_id).await.map_err(err)?;

    let dir = if dest_dir.trim().is_empty() {
        app.path().app_data_dir().map_err(err)?.join("exports")
    } else {
        std::path::PathBuf::from(dest_dir.trim())
    };
    std::fs::create_dir_all(&dir).map_err(err)?;
    let zip_path = dir.join(format!("{}.zip", sanitize_filename(&title)));

    // Defense-in-depth: only bundle audio clips that actually live under the
    // app's audio directory, so a tampered DB path can't leak arbitrary files.
    let audio_dir = app
        .path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("audio"))
        .and_then(|d| std::fs::canonicalize(d).ok());

    let file = std::fs::File::create(&zip_path).map_err(err)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();

    zip.start_file("transcript.md", opts).map_err(err)?;
    zip.write_all(markdown.as_bytes()).map_err(err)?;

    for (message_id, path) in clips {
        let inside = match (&audio_dir, std::fs::canonicalize(&path)) {
            (Some(base), Ok(canon)) => canon.starts_with(base),
            _ => false,
        };
        if !inside {
            log::warn!("skip audio clip outside audio dir: {path}");
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                let name = format!("audio/{message_id}.wav");
                zip.start_file(name, opts).map_err(err)?;
                zip.write_all(&bytes).map_err(err)?;
            }
            // A missing clip file shouldn't abort the whole export.
            Err(e) => log::warn!("skip audio clip {path}: {e}"),
        }
    }
    zip.finish().map_err(err)?;
    Ok(zip_path.to_string_lossy().to_string())
}

/// Replace characters that are invalid in file names with underscores.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() { "conversation".to_string() } else { trimmed.to_string() }
}

/// One thing the user still needs to set up before recording will work.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupIssue {
    /// The settings field this is about (so the UI can deep-link).
    field: String,
    message: String,
}

/// Inspect the saved settings and report what's missing/invalid for the current
/// mode, so the UI can show a "needs setup" banner and the first-run wizard can
/// validate. Empty result = ready to record.
#[tauri::command]
pub async fn check_setup(db: State<'_, Db>) -> CmdResult<Vec<SetupIssue>> {
    let mut settings = db.get_app_settings().await.map_err(err)?;
    inject_api_key(&mut settings);
    let s = |k: &str| settings.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let mut issues = Vec::new();
    let mut add = |field: &str, message: String| issues.push(SetupIssue { field: field.into(), message });

    let stt_local = provider::stt_is_local(&settings);
    let tr_local = provider::translation_is_local(&settings);

    // Any cloud stage needs an endpoint + key.
    if !stt_local || !tr_local {
        if s("apiBaseUrl").is_empty() {
            add("apiBaseUrl", "Не указан адрес API (base URL)".into());
        }
        if s("apiKey").is_empty() {
            add("apiKey", "Не указан API-ключ".into());
        }
    }
    let mut check_file = |path: String, label: &str, field: &str| {
        if path.is_empty() {
            add(field, format!("Не указан путь: {label}"));
        } else if !std::path::Path::new(&path).exists() {
            add(field, format!("Файл не найден: {label}"));
        }
    };
    if stt_local {
        check_file(s("localWhisperServerPath"), "whisper-server (.exe)", "localWhisperServerPath");
        check_file(s("localWhisperPath"), "модель Whisper", "localWhisperPath");
    }
    if tr_local {
        check_file(s("localLlmServerPath"), "llama-server (.exe)", "localLlmServerPath");
        check_file(s("localLlmPath"), "модель LLM", "localLlmPath");
    }
    Ok(issues)
}

/// Whether a file/dir exists — drives the live green-check next to path fields.
#[tauri::command]
pub fn path_exists(path: String) -> bool {
    let p = path.trim();
    !p.is_empty() && std::path::Path::new(p).exists()
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
    let stt_local = provider::stt_is_local(&settings);
    if stt_local {
        let stt = sidecars.ensure_whisper(&settings)?;
        set_str(&mut settings, "localSttBaseUrl", stt);
    }
    if provider::translation_is_local(&settings) {
        let llm = sidecars.ensure_llama(&settings)?;
        set_str(&mut settings, "localLlmBaseUrl", llm);
    }
    let res = recorder.start(app, db.inner().clone(), conv, settings);
    if res.is_err() && stt_local {
        // Recording failed to start (e.g. no audio device) after we spawned the
        // whisper server — kill it so it doesn't sit holding VRAM all session.
        // (llama is left for reuse since text translation may share it.)
        sidecars.kill_whisper();
    }
    res
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

/// Open (or focus) a standalone presentation window showing one side of the
/// transcript large, for a second screen/projector. A browser `window.open`
/// never creates a native window in the Tauri webview, so the ⤢ buttons route
/// here. The `present-a`/`present-b` windows are PRE-DEFINED in tauri.conf.json
/// (hidden, loading present.html) because runtime-created windows didn't load
/// the bundled assets in release — config windows use the same proven mechanism
/// as the main window. We just title + show + focus the right one. `title` is
/// the language name for the caption; state syncs over the Tauri event bus.
#[tauri::command]
pub fn open_present_window(app: AppHandle, side: String, title: String) -> CmdResult<()> {
    if side != "A" && side != "B" {
        return Err("side must be A or B".into());
    }
    let label = format!("present-{}", side.to_lowercase());
    let win = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("present window '{label}' not found"))?;
    let caption = if title.trim().is_empty() { side } else { title };
    let _ = win.set_title(&format!("KaigiAI — {caption}"));
    win.show().map_err(err)?;
    let _ = win.set_focus();
    Ok(())
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
