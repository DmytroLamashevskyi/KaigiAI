mod audio;
mod commands;
mod db;
mod keychain;
mod provider;
mod recording;
mod sidecar;

use tauri::Manager;

use db::Db;
use recording::Recorder;
use sidecar::Sidecars;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Log to a file (and stdout) in release too, so users can send a log
            // when something misbehaves. File lives in the app log dir.
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Info)
                    .targets([
                        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                            file_name: Some("kaigi".into()),
                        }),
                    ])
                    .build(),
            )?;

            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("kaigi.db");

            let db = tauri::async_runtime::block_on(Db::connect(&db_path))
                .expect("failed to open database");
            app.manage(db);
            app.manage(Recorder::default());
            app.manage(Sidecars::default());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::list_messages,
            commands::create_conversation,
            commands::rename_conversation,
            commands::set_conversation_langs,
            commands::set_languages,
            commands::set_speaker_names,
            commands::set_message_speaker,
            commands::delete_conversation,
            commands::add_message,
            commands::save_settings,
            commands::translate_text,
            commands::summarize_conversation,
            commands::generate_title,
            commands::export_zip,
            commands::start_recording,
            commands::stop_recording,
            commands::warmup_servers,
            commands::is_recording,
            commands::list_audio_devices,
            commands::open_url,
            commands::open_present_window,
            commands::check_setup,
            commands::path_exists,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let label = window.label();
                if label.starts_with("present-") {
                    // Present windows are pre-created and reused — hide instead
                    // of destroy so the ⤢ button can show them again.
                    api.prevent_close();
                    let _ = window.hide();
                } else if label == "main" {
                    // Stop any recording cleanly (drops the capture stream and
                    // flushes), then quit — the hidden present windows would
                    // otherwise keep the app alive after the main window closes.
                    let app = window.app_handle();
                    if let Some(rec) = app.try_state::<Recorder>() {
                        rec.stop();
                    }
                    app.exit(0);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(sidecars) = app_handle.try_state::<Sidecars>() {
                    sidecars.shutdown();
                }
            }
        });
}
