//! Local management transport. Lifecycle operations are advertised only once implemented.
use crate::{Error, Result};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

#[cfg(target_os = "macos")]
mod macos;
pub mod protocol;
pub mod session;

pub fn serve(data_dir: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::serve(data_dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data_dir;
        Err(unsupported())
    }
}

pub fn ping(data_dir: &Path) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        macos::ping(data_dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data_dir;
        Err(unsupported())
    }
}

pub fn check_launch(data_dir: &Path, installation_id: Uuid) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        macos::check_launch(data_dir, installation_id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data_dir, installation_id);
        Err(unsupported())
    }
}

#[cfg(not(target_os = "macos"))]
fn unsupported() -> Error {
    Error::new("UNSUPPORTED_PLATFORM", "Daemon transport requires macOS")
}

#[cfg(target_os = "macos")]
fn unavailable() -> Error {
    Error::new(
        "DAEMON_UNAVAILABLE",
        "Daemon is unavailable; no process was started automatically",
    )
}

/// Explicit runtime commands may start the management process; read-only calls
/// always return an offline result/error without creating a daemon.
pub fn call(data_dir: &Path, method: &str, params: Value, autostart: bool) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        macos::call(data_dir, method, params, autostart)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data_dir, method, params, autostart);
        Err(unsupported())
    }
}
/// Explicit desktop connection bootstraps the bundled CLI, not the GUI executable.
pub fn ensure_running(data_dir: &Path, executable: &Path) -> Result<Vec<Value>> {
    #[cfg(target_os = "macos")]
    {
        macos::ensure_running(data_dir, executable)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data_dir, executable);
        Err(unsupported())
    }
}

pub fn status(data_dir: &Path, installation_id: Option<Uuid>) -> Result<Value> {
    match call(
        data_dir,
        "status",
        serde_json::json!({"installation_id":installation_id}),
        false,
    ) {
        #[cfg(target_os = "macos")]
        Err(e) if e.code == "DAEMON_UNAVAILABLE" => {
            crate::runtime::lifecycle::history(data_dir, installation_id)
        }
        result => result,
    }
}

/// Read-only streaming observations. Returning false closes this subscription.
pub fn watch(data: &Path, params: Value, receive: impl FnMut(Value) -> Result<bool>) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::watch(data, params, receive)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data, params, receive);
        Err(unsupported())
    }
}

pub fn watch_cancellable(
    data: &Path,
    params: Value,
    receive: impl FnMut(Value) -> Result<bool>,
    current: impl Fn() -> bool,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::stream_events_cancellable(data, "subscribe", params, receive, current)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data, params, receive, current);
        Err(unsupported())
    }
}

pub fn watch_condition(
    data: &Path,
    params: Value,
    receive: impl FnMut(Value) -> Result<bool>,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::stream_events(data, "condition-watch", params, receive)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data, params, receive);
        Err(unsupported())
    }
}

pub fn follow_logs(
    data: &Path,
    params: Value,
    receive: impl FnMut(Value) -> Result<bool>,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::stream_events(data, "logs-follow", params, receive)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data, params, receive);
        Err(unsupported())
    }
}

/// Registration writes need runtime coordination only when a daemon is live or
/// a durable instance exists. M0-only offline work remains a short L0 operation.
pub fn coordinate(data: &Path, id: Uuid, method: &str, params: Value) -> Result<Option<Value>> {
    #[cfg(target_os = "macos")]
    {
        match ping(data) {
            Ok(_) => return call(data, method, params, false).map(Some),
            Err(e) if e.code == "DAEMON_UNAVAILABLE" => {}
            Err(e) => return Err(e),
        }
        if data
            .join("runtime")
            .join(format!("{id}.json"))
            .try_exists()
            .map_err(|e| crate::storage::io_error(data, e))?
        {
            return call(data, method, params, true).map(Some);
        }
    }
    let _ = (data, id, method, params);
    Ok(None)
}
