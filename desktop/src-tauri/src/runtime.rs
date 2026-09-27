use apphost::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tauri::{State, ipc::Channel};
use uuid::Uuid;

#[derive(Default)]
pub struct Bridge {
    pub(crate) connecting: Arc<Mutex<()>>,
    generation: Arc<AtomicU64>,
    condition: Arc<Mutex<Option<Uuid>>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Start,
    Stop,
    Restart,
    StopAll,
    ResumeWake,
    StopOwnerAndStart,
}
#[derive(Clone, Serialize)]
pub struct Notice {
    kind: &'static str,
}
pub(crate) fn cli_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| Error::new("BACKEND_MISSING", e.to_string()))?;
    let path = exe.with_file_name(if cfg!(windows) {
        "apphost.exe"
    } else {
        "apphost"
    });
    if !path.is_file() {
        return Err(Error::new(
            "BACKEND_MISSING",
            "Bundled apphost executable is missing",
        ));
    }
    Ok(path)
}
#[tauri::command]
pub async fn desktop_connect(state: State<'_, Bridge>) -> Result<Value> {
    let lock = state.connecting.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let warnings =
            apphost::daemon::ensure_running(&apphost::paths::data_dir(None)?, &cli_path()?)?;
        Ok(json!({"warnings":warnings}))
    })
    .await
    .map_err(join_error)?
}
// Use the bundled CLI, never the GUI executable, for explicit daemon bootstrap.
// A mutation is sent once: a timeout does not authorize replaying it.
pub(crate) fn explicit_call(
    data: &std::path::Path,
    method: &str,
    params: Value,
    lock: &Mutex<()>,
) -> Result<Value> {
    let warnings = {
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        apphost::daemon::ensure_running(data, &cli_path()?)?
    };
    let mut result = apphost::daemon::call(data, method, params, false)?;
    for warning in warnings {
        apphost::journal::add_warning(&mut result, warning);
    }
    Ok(result)
}
fn owner_from_report(id: Uuid, failure_id: Option<Uuid>, report: &Value) -> Result<Value> {
    if failure_id.is_none() || report["failure"]["failure_id"] != json!(failure_id) {
        return Err(Error::new(
            "CONDITION_EXPIRED",
            "失败现场已变化，请刷新后重试",
        ));
    }
    // The daemon rechecks both owner identities and the port before stopping it.
    let action = report["advice"]["actions"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["id"] == "stop_owner_and_start"))
        .ok_or_else(|| Error::new("CONDITION_EXPIRED", "当前现场没有可停止的受管占用者"))?;
    let args = action["argv"]
        .as_array()
        .ok_or_else(|| Error::new("PROTOCOL_ERROR", "Missing owner evidence"))?;
    let arg = |flag: &str| -> Result<Uuid> {
        args.windows(2)
            .find(|a| a[0] == flag)
            .and_then(|a| a[1].as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| Error::new("PROTOCOL_ERROR", "Invalid owner evidence"))
    };
    Ok(
        json!({"installation_id":id,"owner_installation_id":arg("--owner-installation-id")?,"owner_instance_id":arg("--owner-instance-id")?}),
    )
}
#[tauri::command]
pub async fn desktop_action(
    state: State<'_, Bridge>,
    action: Action,
    installation_id: Option<Uuid>,
    failure_id: Option<Uuid>,
) -> Result<Value> {
    let lock = state.connecting.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = apphost::paths::data_dir(None)?;
        let (method, params) = match action {
            Action::StopAll => ("stop-all", json!({})),
            Action::ResumeWake => ("resume-wake", json!({})),
            action => {
                let id = installation_id
                    .ok_or_else(|| Error::new("INVALID_PARAMS", "Installation is required"))?;
                match action {
                    Action::StopOwnerAndStart => (
                        "stop-owner-and-start",
                        owner_from_report(
                            id,
                            failure_id,
                            &apphost::failure::read(&data, id, None)?,
                        )?,
                    ),
                    Action::Start => ("start", json!({"installation_id":id})),
                    Action::Stop => ("stop", json!({"installation_id":id})),
                    _ => ("restart", json!({"installation_id":id})),
                }
            }
        };
        explicit_call(&data, method, params, &lock)
    })
    .await
    .map_err(join_error)?
}
#[tauri::command]
pub async fn desktop_watch(state: State<'_, Bridge>, channel: Channel<Notice>) -> Result<()> {
    let generation = state.generation.clone();
    let current = generation.fetch_add(1, Ordering::AcqRel) + 1;
    tauri::async_runtime::spawn_blocking(move || {
        let result = apphost::daemon::watch_cancellable(
            &apphost::paths::data_dir(None)?,
            json!({}),
            |event| {
                let kind = match event["kind"].as_str() {
                    Some("heartbeat") => return Ok(true),
                    Some("snapshot" | "subscribed") => "connected",
                    _ => "changed",
                };
                // Never forward raw event records containing launch context to WebView.
                channel
                    .send(Notice { kind })
                    .map_err(|_| Error::new("SUBSCRIPTION_CLOSED", "Desktop receiver closed"))?;
                Ok(true)
            },
            || generation.load(Ordering::Acquire) == current,
        );
        match result {
            Err(e) if e.code == "SUBSCRIPTION_CLOSED" => Ok(()),
            other => other,
        }
    })
    .await
    .map_err(join_error)?
}
#[tauri::command]
pub fn desktop_unwatch(state: State<'_, Bridge>) {
    state.generation.fetch_add(1, Ordering::AcqRel);
}
fn join_error(error: tauri::Error) -> Error {
    Error::new("DESKTOP_RUNTIME_FAILED", error.to_string())
}

#[tauri::command]
pub async fn desktop_open(
    installation_id: Uuid,
    instance_id: Uuid,
    operation_id: Uuid,
) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let data = apphost::paths::data_dir(None)?;
        let status = apphost::daemon::status(&data, Some(installation_id))?;
        let record = status["instances"]
            .as_array()
            .and_then(|items| items.first());
        if status["live"] != true
            || !record.is_some_and(|r| {
                r["instance_id"] == json!(instance_id)
                    && r["operation_id"] == json!(operation_id)
                    && r["status"]["state"] == "RUNNING"
                    && r["services"].as_array().is_some_and(|s| {
                        !s.is_empty() && s.iter().all(|s| s["alive"] == true && s["ready"] == true)
                    })
            })
        {
            return Err(Error::new(
                "OPERATION_EXPIRED",
                "The selected instance is no longer ready",
            ));
        }
        let entry = apphost::gateway::url(&data, installation_id)?;
        let url = entry["url"]
            .as_str()
            .ok_or_else(|| Error::new("NO_BROWSER_ENTRY", "This app has no browser entry"))?;
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("/usr/bin/open")
                .arg(url)
                .output()
                .map_err(|e| Error::new("OPEN_FAILED", e.to_string()))?;
            if !output.status.success() {
                return Err(Error::new(
                    "OPEN_FAILED",
                    "System browser could not open the app",
                ));
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = url;
            Err(Error::new(
                "UNSUPPORTED_PLATFORM",
                "Browser runtime requires macOS",
            ))
        }
    })
    .await
    .map_err(join_error)?
}

#[tauri::command]
pub async fn desktop_condition_watch(
    state: State<'_, Bridge>,
    installation_id: Uuid,
    failure_id: Uuid,
    token: Uuid,
    channel: Channel<Value>,
) -> Result<()> {
    let active = state.condition.clone();
    *active.lock().unwrap_or_else(|e| e.into_inner()) = Some(token);
    tauri::async_runtime::spawn_blocking(move || {
        // One observation per second, bounded to 60 seconds by the core. Closing
        // or replacing a panel closes this stream at the next observation.
        apphost::daemon::watch_condition(
            &apphost::paths::data_dir(None)?,
            json!({"installation_id":installation_id,"failure_id":failure_id,"duration_secs":60}),
            |event| {
                if *active.lock().unwrap_or_else(|e| e.into_inner()) != Some(token) {
                    return Ok(false);
                }
                Ok(channel.send(event).is_ok())
            },
        )
    })
    .await
    .map_err(join_error)?
}
#[tauri::command]
pub fn desktop_condition_unwatch(state: State<'_, Bridge>, token: Uuid) {
    let mut active = state.condition.lock().unwrap_or_else(|e| e.into_inner());
    if *active == Some(token) {
        *active = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_action_is_bound_to_latest_failure_and_core_uuids() {
        let id = Uuid::new_v4();
        let failure = Uuid::new_v4();
        let owner = Uuid::new_v4();
        let instance = Uuid::new_v4();
        let report = json!({"failure":{"failure_id":failure},"advice":{"actions":[{"id":"stop_owner_and_start","argv":["apphost","stop-owner-and-start",id,"--owner-installation-id",owner,"--owner-instance-id",instance]}]}});
        let params = owner_from_report(id, Some(failure), &report).unwrap();
        assert_eq!(params["owner_installation_id"], json!(owner));
        assert_eq!(
            owner_from_report(id, Some(Uuid::new_v4()), &report)
                .unwrap_err()
                .code,
            "CONDITION_EXPIRED"
        );
        assert!(owner_from_report(id, None, &report).is_err());
        assert!(
            owner_from_report(
                id,
                Some(failure),
                &json!({"failure":{"failure_id":failure},"advice":{"actions":[]}})
            )
            .is_err()
        );
    }
}
