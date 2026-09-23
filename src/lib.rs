pub mod daemon;
pub mod discovery;
pub mod environment;
pub mod failure;
mod git_metadata;
mod launch;
pub mod logs;
pub mod manifest;
pub mod process;
pub mod registry;
pub mod review;
pub mod rules;
pub mod runtime;
mod storage;

use serde::Serialize;
use serde_json::{Value, json};
use std::fmt;

pub use discovery::{Draft, scan};
pub use manifest::Manifest;
pub use registry::{associate, detach, detach_many, import, label, list, relink, retention};

#[derive(Debug, Serialize)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Value::Null,
        }
    }
    pub fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

pub fn read_draft(path: &std::path::Path) -> Result<Draft> {
    serde_json::from_slice(&storage::read(path)?)
        .map_err(|e| Error::new("INVALID_INPUT", e.to_string()))
}

pub fn write_draft(path: &std::path::Path, draft: &Draft) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(draft).map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
    if bytes.len() > storage::MAX_BYTES as usize {
        return Err(Error::new(
            "INVALID_INPUT",
            "Draft exceeds 1 MiB; scan a smaller directory",
        ));
    }
    storage::publish(path, &bytes, false)
}

pub fn validate(path: &std::path::Path) -> Result<Manifest> {
    let path = if path.is_dir() {
        path.join(manifest::FILENAME)
    } else {
        path.to_owned()
    };
    let manifest = Manifest::read(&path)?;
    let root = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let issues = manifest.validate(root);
    if !issues.is_empty() {
        return Err(Error::new("VALIDATION_ERROR", "Invalid manifest").details(json!(issues)));
    }
    Ok(manifest)
}
