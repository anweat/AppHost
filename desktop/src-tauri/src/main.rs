#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod runtime;
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
        .manage(runtime::Bridge::default())
        .invoke_handler(tauri::generate_handler![
            desktop_snapshot,
            runtime::desktop_connect,
            runtime::desktop_open,
            runtime::desktop_action,
            runtime::desktop_watch,
            runtime::desktop_unwatch,
            desktop_preferences,
            desktop_theme
        ])
        .run(tauri::generate_context!())
        .expect("Unable to run AppHost desktop");
}
