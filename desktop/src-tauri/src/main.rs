#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod details;
mod registration;
mod runtime;
mod settings;
mod snapshot;

// Web content cannot choose a data directory or submit arbitrary daemon methods.
#[tauri::command]
async fn desktop_snapshot() -> apphost::Result<snapshot::Snapshot> {
    tauri::async_runtime::spawn_blocking(|| snapshot::read(&apphost::paths::data_dir(None)?))
        .await
        .map_err(|e| apphost::Error::new("DESKTOP_READ_FAILED", e.to_string()))?
}

#[tauri::command]
async fn desktop_preferences() -> apphost::Result<apphost::ui::Preferences> {
    apphost::ui::load(&apphost::paths::data_dir(None)?)
}
#[tauri::command]
async fn desktop_theme(theme: apphost::ui::Theme) -> apphost::Result<apphost::ui::Preferences> {
    tauri::async_runtime::spawn_blocking(move || {
        apphost::ui::save_theme(&apphost::paths::data_dir(None)?, theme)
    })
    .await
    .map_err(|e| apphost::Error::new("DESKTOP_WRITE_FAILED", e.to_string()))?
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(runtime::Bridge::default())
        .manage(registration::DraftBuffer::default())
        .invoke_handler(tauri::generate_handler![
            desktop_snapshot,
            runtime::desktop_connect,
            runtime::desktop_open,
            runtime::desktop_action,
            runtime::desktop_watch,
            runtime::desktop_unwatch,
            desktop_preferences,
            desktop_theme,
            details::desktop_logs,
            details::desktop_failure,
            details::desktop_diagnostics,
            details::desktop_terminal,
            details::desktop_copy,
            registration::desktop_scan,
            registration::desktop_group,
            registration::desktop_import,
            registration::desktop_manifest_read,
            registration::desktop_manifest_review,
            registration::desktop_manifest_save,
            settings::desktop_environment_read,
            settings::desktop_environment_save,
            settings::desktop_environment_capture,
            settings::desktop_environment_check,
            settings::desktop_retention_read,
            settings::desktop_retention_save
        ])
        .run(tauri::generate_context!())
        .expect("Unable to run AppHost desktop");
}
