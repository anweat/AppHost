#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod snapshot;

// Web content cannot choose a data directory or submit arbitrary daemon methods.
#[tauri::command]
async fn desktop_snapshot() -> apphost::Result<snapshot::Snapshot> {
    tauri::async_runtime::spawn_blocking(|| snapshot::read(&apphost::paths::data_dir(None)?))
        .await
        .map_err(|e| apphost::Error::new("DESKTOP_READ_FAILED", e.to_string()))?
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![desktop_snapshot])
        .run(tauri::generate_context!())
        .expect("Unable to run AppHost desktop");
}
