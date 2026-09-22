//! Ordinary log retention only. Failure evidence has a separate lifetime and
//! must never be placed in this tree. No active instance is removed here.
use super::lifecycle::Record;
use crate::{Error, Result, storage};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
const BUDGET: u64 = 100 * 1024 * 1024;
const ACTIVE_RESERVE: u64 = 9 * 1024 * 1024;
const AGE: u128 = 7 * 24 * 60 * 60 * 1000;
fn invalid() -> Error {
    Error::new(
        "RUNTIME_INVALID",
        "Invalid instance log archive; preserve it for inspection",
    )
}
pub(super) fn archive(data: &Path, r: &Record) -> Result<()> {
    let root = data.join("instances").join(r.instance_id.to_string());
    storage::private_directory(&root)?;
    storage::publish(
        &root.join("record.json"),
        &serde_json::to_vec(r).map_err(|_| invalid())?,
        true,
    )
}
fn size(root: &Path, depth: usize, count: &mut usize) -> Result<u64> {
    if depth > 3 {
        return Err(invalid());
    }
    let mut total = 0;
    for entry in fs::read_dir(root).map_err(|e| storage::io_error(root, e))? {
        *count += 1;
        if *count > 15000 {
            return Err(Error::new("RUNTIME_LIMIT", "Too many archive files"));
        }
        let entry = entry.map_err(|e| storage::io_error(root, e))?;
        let kind = entry.file_type().map_err(|e| storage::io_error(root, e))?;
        if kind.is_file() {
            total += entry
                .metadata()
                .map_err(|e| storage::io_error(root, e))?
                .len();
        } else if kind.is_dir() {
            total += size(&entry.path(), depth + 1, count)?;
        } else {
            return Err(invalid());
        }
    }
    Ok(total)
}
pub(super) fn prune(data: &Path, id: Uuid, active: Uuid) -> Result<Vec<serde_json::Value>> {
    let root = data.join("instances");
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(storage::io_error(&root, e)),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut archives: Vec<(u128, u64, PathBuf)> = Vec::new();
    let mut files = 0;
    let mut ignored = 0;
    for (count, e) in entries.enumerate() {
        if count >= 4096 {
            return Err(Error::new("RUNTIME_LIMIT", "Too many instance directories"));
        }
        let e = e.map_err(|e| storage::io_error(&root, e))?;
        let name = e.file_name();
        let instance = name
            .to_str()
            .and_then(|name| Uuid::parse_str(name).ok())
            .filter(|id| name == id.to_string().as_str());
        let Some(instance) = instance else {
            ignored += 1;
            continue;
        };
        if instance == active {
            continue;
        }
        if !e
            .file_type()
            .map_err(|e| storage::io_error(&root, e))?
            .is_dir()
        {
            return Err(invalid());
        }
        let record: Record = match storage::read(&e.path().join("record.json")) {
            Ok(b) => serde_json::from_slice(&b).map_err(|_| invalid())?,
            // A live instance has only its daemon-owned current record.
            Err(e) if e.code == "NOT_FOUND" => continue,
            Err(e) => return Err(e),
        };
        if record.instance_id != instance {
            return Err(invalid());
        }
        if record.installation_id != id {
            continue;
        }
        if record.services.iter().any(|s| s.alive)
            || record.collector_alive
            || record.management_uncertain
        {
            continue;
        }
        archives.push((
            record.created_at_ms,
            size(&e.path(), 0, &mut files)?,
            e.path(),
        ));
    }
    archives.sort_by_key(|entry| entry.0);
    let mut bytes: u64 = archives.iter().map(|a| a.1).sum();
    for (created, size, path) in archives {
        if now.saturating_sub(created) > AGE || bytes > BUDGET - ACTIVE_RESERVE {
            fs::remove_dir_all(&path).map_err(|e| storage::io_error(&path, e))?;
            bytes = bytes.saturating_sub(size);
        }
    }
    Ok(storage::directory_warnings(&root, ignored))
}
pub(super) fn read(data: &Path, id: Uuid, instance: Uuid) -> Result<Record> {
    let path = data
        .join("instances")
        .join(instance.to_string())
        .join("record.json");
    let r: Record = serde_json::from_slice(&storage::read(&path)?).map_err(|_| invalid())?;
    if r.installation_id != id || r.instance_id != instance {
        return Err(invalid());
    }
    Ok(r)
}
