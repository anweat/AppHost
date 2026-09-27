//! Registration mutations use the same stop-and-write coordinator as the CLI.
use apphost::{Error, Result, journal::Actor};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

fn join_error(e: tauri::Error) -> Error {
    Error::new("DESKTOP_WRITE_FAILED", e.to_string())
}
fn mutate(data: &Path, id: Uuid, path: Option<&Path>, lock: &Mutex<()>) -> Result<Value> {
    apphost::registry::registered(data, id)?;
    let method = if path.is_some() { "relink" } else { "detach" };
    let params = if let Some(path) = path {
        json!({"installation_id":id,"path":path})
    } else {
        json!({"installation_id":id})
    };
    #[cfg(target_os = "macos")]
    {
        let live = match apphost::daemon::ping(data) {
            Ok(_) => true,
            Err(e) if e.code == "DAEMON_UNAVAILABLE" => false,
            Err(e) => return Err(e),
        };
        let durable = data
            .join("runtime")
            .join(format!("{id}.json"))
            .try_exists()
            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
        if live || durable {
            return crate::runtime::explicit_call(data, method, params, lock)
                .map(|v| receipt(id, &v));
        }
    }
    let _ = (params, lock);
    let result = if let Some(path) = path {
        apphost::registry::relink(data, id, path)
    } else {
        apphost::registry::detach(data, id)
    };
    // Do not expose confirmed services/environment from the installation record.
    apphost::journal::record_result(
        data,
        method,
        Actor::Human,
        Some(id),
        result.map(|_| json!({"installation_id":id,"completed":true})),
    )
}
// Direct registration results contain confirmed env; only project the receipt.
pub(crate) fn receipt(id: Uuid, value: &Value) -> Value {
    json!({"installation_id":id,"completed":!value["operation_id"].is_string(),
        "operation_id":value["operation_id"],"operation_done":value["operation_done"],"warnings":value["warnings"]})
}
#[tauri::command]
pub async fn desktop_detach(
    state: tauri::State<'_, crate::runtime::Bridge>,
    installation_id: Uuid,
) -> Result<Value> {
    let lock = state.connecting.clone();
    tauri::async_runtime::spawn_blocking(move || {
        mutate(
            &apphost::paths::data_dir(None)?,
            installation_id,
            None,
            &lock,
        )
    })
    .await
    .map_err(join_error)?
}
#[tauri::command]
pub async fn desktop_relink(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::runtime::Bridge>,
    installation_id: Uuid,
) -> Result<Option<Value>> {
    let lock = state.connecting.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(folder) = app
            .dialog()
            .file()
            .set_title("选择移动后的项目目录并重新关联")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let path = folder
            .into_path()
            .map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
        mutate(
            &apphost::paths::data_dir(None)?,
            installation_id,
            Some(&path),
            &lock,
        )
        .map(Some)
    })
    .await
    .map_err(join_error)?
}
#[tauri::command]
pub async fn desktop_operation(installation_id: Uuid, operation_id: Uuid) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let status =
            apphost::daemon::status(&apphost::paths::data_dir(None)?, Some(installation_id))?;
        if status["live"] != true {
            return Err(Error::new(
                "DAEMON_UNAVAILABLE",
                "后台未连接，操作结果尚未确认",
            ));
        }
        let record = status["instances"]
            .as_array()
            .and_then(|a| a.first())
            .filter(|r| r["operation_id"] == json!(operation_id))
            .ok_or_else(|| Error::new("OPERATION_EXPIRED", "操作已被替换，请刷新状态核对"))?;
        Ok(json!({"completed":record["operation_done"],"error":record["operation_error"].as_object().map(|e|json!({"code":e.get("code"),"message":e.get("message").and_then(Value::as_str).unwrap_or("操作未完成，请核对应用状态")}))}))
    })
    .await
    .map_err(join_error)?
}
#[tauri::command]
pub async fn desktop_journal(cursor: Option<String>) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        apphost::journal::read(&apphost::paths::data_dir(None)?, cursor.as_deref(), 50)
    })
    .await
    .map_err(join_error)?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_and_coordinated_receipts_exclude_registration_secrets() {
        let id = Uuid::new_v4();
        let direct = receipt(
            id,
            &json!({"installation_id":id,"confirmed_services":[{"env":{"TOKEN":"private"}}]}),
        );
        assert_eq!(direct["completed"], true);
        assert!(!direct.to_string().contains("private"));
        let queued = receipt(
            id,
            &json!({"operation_id":Uuid::new_v4(),"operation_done":false}),
        );
        assert_eq!(queued["completed"], false);
    }
    #[test]
    fn offline_relink_and_detach_preserve_identity_without_returning_env() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let root = t.path().join("app");
        std::fs::create_dir(&root).unwrap();
        let manifest: apphost::Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture","services":[{"name":"web","command":["unused"],"env":{"TOKEN":"private-value"}}]})).unwrap();
        std::fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id =
            apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        let lock = Mutex::new(());
        let moved = t.path().join("moved");
        std::fs::rename(root, &moved).unwrap();
        let result = mutate(&data, id, Some(&moved), &lock).unwrap();
        assert_eq!(result["completed"], true);
        assert!(!result.to_string().contains("private-value"));
        assert_eq!(
            apphost::registry::registered(&data, id).unwrap().path,
            moved.canonicalize().unwrap()
        );
        assert!(apphost::list(&data, false).unwrap()[0].requires_launch_confirmation);
        mutate(&data, id, None, &lock).unwrap();
        assert!(moved.join("apphost.toml").exists());
        assert!(apphost::list(&data, false).unwrap().is_empty());
    }
}
