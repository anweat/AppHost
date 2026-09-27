use apphost::{Error, Result};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use uuid::Uuid;

fn join_error(e: tauri::Error) -> Error {
    Error::new("DESKTOP_READ_FAILED", e.to_string())
}

#[tauri::command]
pub async fn desktop_logs(installation_id: Uuid, instance_id: Uuid) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let to = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
        let from = to.saturating_sub(5 * 60 * 1000);
        let mut value = apphost::daemon::call(&apphost::paths::data_dir(None)?, "logs",
            json!({"installation_id":installation_id,"instance_id":instance_id,"from_ms":from,"to_ms":to,"limit":65536}), false)?;
        value["read_at_ms"] = json!(to);
        value["from_ms"] = json!(from);
        value["to_ms"] = json!(to);
        Ok(value)
    }).await.map_err(join_error)?
}

#[tauri::command]
pub async fn desktop_failure(installation_id: Uuid, failure_id: Option<Uuid>) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        apphost::failure::read(
            &apphost::paths::data_dir(None)?,
            installation_id,
            failure_id,
        )
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn desktop_diagnostics(installation_id: Uuid, failure_id: Uuid) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        apphost::failure::export(
            &apphost::paths::data_dir(None)?,
            installation_id,
            Some(failure_id),
        )
    })
    .await
    .map_err(join_error)?
}

fn project_directory(data: &Path, id: Uuid) -> Result<PathBuf> {
    // Resolve only a currently registered directory, never a WebView-supplied path
    // or a shell snippet. The terminal chooses its own normal interactive startup.
    let root = apphost::registry::registered(data, id)?.path;
    let directory = root
        .canonicalize()
        .map_err(|_| Error::new("SOURCE_MISSING", "项目目录不可用"))?;
    if !directory.is_dir() {
        return Err(Error::new("SOURCE_MISSING", "项目路径不是目录"));
    }
    Ok(directory)
}

#[tauri::command]
pub async fn desktop_terminal(installation_id: Uuid) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let directory = project_directory(&apphost::paths::data_dir(None)?, installation_id)?;
        #[cfg(target_os = "macos")]
        {
            let result = std::process::Command::new("/usr/bin/open")
                .args(["-b", "com.apple.Terminal"])
                .arg(directory)
                .status()
                .map_err(|e| Error::new("TERMINAL_OPEN_FAILED", e.to_string()))?;
            if !result.success() {
                return Err(Error::new("TERMINAL_OPEN_FAILED", "无法打开系统终端"));
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = directory;
            Err(Error::new(
                "UNSUPPORTED_PLATFORM",
                "该平台的系统终端入口尚未接入",
            ))
        }
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub fn desktop_copy(app: tauri::AppHandle, text: String) -> Result<()> {
    if text.len() > 1024 * 1024 {
        return Err(Error::new("COPY_LIMIT", "复制内容超过 1 MiB"));
    }
    // Write only on a user's copy action; no clipboard read permission is exposed.
    app.clipboard()
        .write_text(text)
        .map_err(|e| Error::new("COPY_FAILED", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_directory_comes_from_registration_and_requires_existing_directory() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let root = t.path().join("project with 'quotes' ; $(never-run)");
        std::fs::create_dir(&root).unwrap();
        let manifest: apphost::Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture","services":[{"name":"web","command":["unused"]}]})).unwrap();
        std::fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id =
            apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        assert_eq!(
            project_directory(&data, id).unwrap(),
            root.canonicalize().unwrap()
        );
        assert!(project_directory(&data, Uuid::new_v4()).is_err());
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            project_directory(&data, id).unwrap_err().code,
            "SOURCE_MISSING"
        );
    }
}

#[tauri::command]
pub async fn desktop_icon(installation_id: Uuid) -> Result<Option<Value>> {
    tauri::async_runtime::spawn_blocking(move || {
        let data = apphost::paths::data_dir(None)?;
        let Some(icon) = apphost::icon::discover(&data, installation_id)? else {
            return Ok(None);
        };
        let bytes = apphost::icon::read(&data, installation_id, &icon.path, &icon.sha256)?;
        Ok(Some(json!({"mime":icon.mime,"bytes":bytes})))
    })
    .await
    .map_err(join_error)?
}
