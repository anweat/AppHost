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
    connecting: Arc<Mutex<()>>,
    generation: Arc<AtomicU64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Start,
    Stop,
    Restart,
    StopAll,
    ResumeWake,
}
#[derive(Clone, Serialize)]
pub struct Notice {
    kind: &'static str,
}
fn cli_path() -> Result<PathBuf> {
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
#[tauri::command]
pub async fn desktop_action(action: Action, installation_id: Option<Uuid>) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let (method, params) = match action {
            Action::StopAll => ("stop-all", json!({})),
            Action::ResumeWake => ("resume-wake", json!({})),
            action => {
                let id = installation_id
                    .ok_or_else(|| Error::new("INVALID_PARAMS", "Installation is required"))?;
                (
                    match action {
                        Action::Start => "start",
                        Action::Stop => "stop",
                        _ => "restart",
                    },
                    json!({"installation_id":id}),
                )
            }
        };
        // No retry of a mutation: a timeout is an uncertain receipt, not permission
        // to repeat a start. Only the explicit connection action bootstraps a daemon.
        apphost::daemon::call(&apphost::paths::data_dir(None)?, method, params, false)
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
