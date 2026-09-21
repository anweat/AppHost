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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<Open>,
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Open {
    pub service: String,
    #[serde(default = "default_open_path")]
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenTarget {
    pub service: String,
    pub port: u16,
    pub path: String,
}

fn default_open_path() -> String {
    "/".into()
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_timeout_ms: Option<u64>,
}

impl Service {
    pub fn startup_timeout_ms(&self) -> u64 {
        self.startup_timeout_ms.unwrap_or(30_000)
    }

    pub fn stop_timeout_ms(&self) -> u64 {
        self.stop_timeout_ms.unwrap_or(5_000)
    }

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
        if let Some(timeout) = self.startup_timeout_ms {
            result.push_str(&format!("  startup_timeout_ms={timeout}"));
        }
        if let Some(timeout) = self.stop_timeout_ms {
            result.push_str(&format!("  stop_timeout_ms={timeout}"));
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
        // Keep the v0 byte representation exact: historical confirmations predate
        // open/timeout fields. Only v1 normalizes omitted execution defaults.
        let bytes = if self.schema_version == 1 {
            let mut services = self.services.clone();
            for service in &mut services {
                service.startup_timeout_ms = Some(service.startup_timeout_ms());
                service.stop_timeout_ms = Some(service.stop_timeout_ms());
            }
            serde_json::to_vec(&(
                "apphost.launch.v2",
                self.schema_version,
                self.project_id,
                services,
                &self.open,
            ))
        } else {
            serde_json::to_vec(&(
                "apphost.launch.v1",
                self.schema_version,
                self.project_id,
                &self.services,
            ))
        }
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
        if self.schema_version > 1 {
            issues.push(Issue::new(
                "schema_version",
                "Only versions 0 and 1 are supported",
            ));
        }
        if self.schema_version == 0 && self.open.is_some() {
            issues.push(Issue::new("open", "open requires schema_version 1"));
        }
        if self.open.is_some()
            && let Err(error) = self.open_target()
        {
            issues.push(Issue::new("open", error.message));
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
            for (key, value, maximum) in [
                ("startup_timeout_ms", service.startup_timeout_ms, 600_000),
                ("stop_timeout_ms", service.stop_timeout_ms, 60_000),
            ] {
                if let Some(value) = value {
                    if self.schema_version == 0 {
                        issues.push(Issue::new(
                            field(key),
                            "Timeout fields require schema_version 1",
                        ));
                    } else if !(1_000..=maximum).contains(&value) {
                        issues.push(Issue::new(
                            field(key),
                            format!("Expected 1000..={maximum} milliseconds"),
                        ));
                    }
                }
            }
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

    /// Resolve a validated declaration, without probing ports or waking anything.
    pub fn open_target(&self) -> Result<Option<OpenTarget>> {
        let (service, path) = if let Some(open) = &self.open {
            if !open.path.starts_with('/')
                || open.path.starts_with("//")
                || open.path.contains(['\\', '?', '#'])
                || open.path.chars().any(char::is_control)
            {
                return Err(Error::new(
                    "VALIDATION_ERROR",
                    "open.path must be a local path starting with one slash, without query or fragment",
                ));
            }
            let matches: Vec<_> = self
                .services
                .iter()
                .filter(|s| s.name == open.service)
                .collect();
            if matches.len() != 1 || matches[0].port.is_none_or(|p| p == 0) {
                return Err(Error::new(
                    "VALIDATION_ERROR",
                    "open.service must reference exactly one service with a declared port",
                ));
            }
            (matches[0], open.path.clone())
        } else {
            let mut services = self.services.iter().filter(|s| s.port.is_some());
            let Some(service) = services.next() else {
                return Ok(None);
            };
            if services.next().is_some() {
                return Err(Error::new(
                    "OPEN_TARGET_REQUIRED",
                    "Multiple services declare ports; select open.service explicitly",
                ));
            }
            (service, "/".into())
        };
        Ok(Some(OpenTarget {
            service: service.name.clone(),
            port: service.port.unwrap(),
            path,
        }))
    }
}
