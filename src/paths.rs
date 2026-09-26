//! Shared CLI/desktop data location; resolving it never creates a directory.
use crate::{Error, Result};
use std::{env, path::PathBuf};

pub fn data_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit.or_else(|| env::var_os("APPHOST_DATA_DIR").map(PathBuf::from)) {
        if path.as_os_str().is_empty() {
            return Err(Error::new("INVALID_INPUT", "data-dir must not be empty"));
        }
        return Ok(path);
    }
    if cfg!(target_os = "windows") {
        return env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("AppHost"))
            .ok_or_else(|| Error::new("INVALID_INPUT", "Set --data-dir or LOCALAPPDATA"));
    }
    if !cfg!(target_os = "macos")
        && let Some(path) = env::var_os("XDG_DATA_HOME")
    {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Ok(path.join("apphost"));
        }
    }
    let home =
        env::var_os("HOME").ok_or_else(|| Error::new("INVALID_INPUT", "Set --data-dir or HOME"))?;
    Ok(PathBuf::from(home).join(if cfg!(target_os = "macos") {
        "Library/Application Support/AppHost"
    } else {
        ".local/share/apphost"
    }))
}
