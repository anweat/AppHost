//! Local management transport. Lifecycle operations are advertised only once implemented.
use crate::{Error, Result};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

#[cfg(target_os = "macos")]
mod macos;
pub mod protocol;

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
