use crate::{Error, Result, storage};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Component, Path},
};
use uuid::Uuid;

pub const FILENAME: &str = "apphost.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub project_id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub name: String,
    #[serde(default = "default_cwd")]
    pub cwd: String,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl Service {
    /// Human-readable argv, not a shell command to execute. Quote ambiguous arguments.
    pub fn launch_summary(&self) -> String {
        let command = self
            .command
            .iter()
            .map(|arg| {
                if !arg.is_empty()
                    && arg
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._/-:@".contains(&b))
                {
                    arg.clone()
                } else {
                    serde_json::to_string(arg).unwrap()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let mut result = format!(
            "cwd={:?}  {}",
            self.cwd,
            if command.is_empty() {
                "（待填写命令）"
            } else {
                &command
            }
        );
        if let Some(port) = self.port {
            result.push_str(&format!("  port={port}"));
        }
        if !self.env.is_empty() {
            result.push_str(&format!("  env={:?}", self.env));
        }
        result
    }
}

fn default_cwd() -> String {
    ".".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    pub field: String,
    pub message: String,
}
impl Issue {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

pub fn valid_text(text: &str) -> bool {
    !text.trim().is_empty() && !text.chars().any(char::is_control)
}

pub fn relative_directory(root: &Path, cwd: &str) -> Result<()> {
    let path = Path::new(cwd);
    // Check Windows separators even on Unix, so a portable declaration cannot change meaning on Windows.
    if cwd.is_empty()
        || cwd.contains('\\')
        || cwd.contains(':')
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::new(
            "VALIDATION_ERROR",
            "cwd must be a relative directory without parent traversal",
        ));
    }
    let root = storage::directory(root)?;
    let actual = storage::directory(&root.join(path))?;
    if !actual.starts_with(&root) {
        return Err(Error::new(
            "VALIDATION_ERROR",
            "cwd resolves outside project",
        ));
    }
    Ok(())
}

impl Manifest {
    /// Versioned launch declaration digest. Cosmetic edits do not revoke confirmation.
    /// M1 must compare this on the validated snapshot it will execute, immediately before launch.
    /// This does not attest to executable/script contents, installed dependencies or PATH changes.
    pub fn launch_sha256(&self) -> String {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(&(
            "apphost.launch.v1",
            self.schema_version,
            self.project_id,
            &self.services,
        ))
        .expect("Launch declarations contain only JSON-serializable values");
        format!("{:x}", Sha256::digest(bytes))
    }

    /// One renderer for size preflight, atomic publication and creation receipts.
    pub fn to_toml(&self) -> Result<String> {
        let text = toml::to_string(self).map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
        if let Some(services) = document
            .get_mut("services")
            .and_then(toml_edit::Item::as_array_of_tables_mut)
        {
            for (table, service) in services.iter_mut().zip(&self.services) {
                // JSON string escapes are also TOML basic-string escapes; DEL needs explicit
                // escaping in TOML. Parse the array as a value, never splice document text.
                let args = service
                    .command
                    .iter()
                    .map(|arg| {
                        serde_json::to_string(arg)
                            .unwrap()
                            .replace('\u{7f}', "\\u007F")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let value = format!("[{args}]")
                    .parse::<toml_edit::Value>()
                    .map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
                table["command"] = toml_edit::Item::Value(value);
            }
        }
        Ok(document.to_string())
    }

    pub fn read(path: &Path) -> Result<Self> {
        let bytes = storage::read(path)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| Error::new("VALIDATION_ERROR", e.to_string()))?;
        toml::from_str(text).map_err(|e| Error::new("VALIDATION_ERROR", e.to_string()))
    }

    pub fn validate(&self, root: &Path) -> Vec<Issue> {
        let mut issues = Vec::new();
        if self.schema_version != 0 {
            issues.push(Issue::new("schema_version", "Only version 0 is supported"));
        }
        if self.project_id.is_nil() {
            issues.push(Issue::new("project_id", "UUID must not be nil"));
        }
        if !valid_text(&self.name) {
            issues.push(Issue::new(
                "name",
                "Name must be nonempty and contain no control characters",
            ));
        }
        if self.description.contains('\0') {
            issues.push(Issue::new("description", "NUL is not allowed"));
        }
        if self.services.is_empty() {
            issues.push(Issue::new("services", "At least one service is required"));
        }
        let mut names = HashSet::new();
        for (i, service) in self.services.iter().enumerate() {
            let field = |name: &str| format!("services[{i}].{name}");
            if !valid_text(&service.name) || !names.insert(&service.name) {
                issues.push(Issue::new(
                    field("name"),
                    "Service names must be nonempty, unique and contain no control characters",
                ));
            }
            if service.command.first().is_none_or(|c| !valid_text(c))
                || service.command.iter().any(|c| c.contains('\0'))
            {
                issues.push(Issue::new(
                    field("command"),
                    "Expected nonempty executable and NUL-free arguments",
                ));
            }
            if let Err(e) = relative_directory(root, &service.cwd) {
                issues.push(Issue::new(field("cwd"), e.message));
            }
            if service.port == Some(0) {
                issues.push(Issue::new(
                    field("port"),
                    "Port must be between 1 and 65535",
                ));
            }
            if service
                .env
                .iter()
                .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
            {
                issues.push(Issue::new(
                    field("env"),
                    "Invalid environment key or NUL in value",
                ));
            }
        }
        issues
    }
}
