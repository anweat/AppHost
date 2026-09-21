use crate::{
    Error, Result,
    discovery::{Candidate, Draft, INPUTS},
    manifest::{FILENAME, Manifest, Service, valid_text},
    storage,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub installation_id: Uuid,
    pub project_id: Uuid,
    pub path: PathBuf,
    pub human_label: String,
    // Legacy labels have no reliable provenance; preserve them as explicit labels.
    #[serde(default = "legacy_custom_label")]
    pub label_is_custom: bool,
    pub detached: bool,
    #[serde(default)]
    pub retention_policy: RetentionPolicy,
    #[serde(default)]
    pub created_manifest: Option<CreatedManifest>,
    #[serde(default)]
    pub confirmed_launch_sha256: Option<String>,
    // This diagnostic snapshot includes literal env values and may contain secrets.
    // Future exports must redact it; data cleanup must account for this AppHost-owned copy.
    // It is never a runtime source of commands or environment variables.
    #[serde(default)]
    pub confirmed_services: Option<Vec<Service>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_manifest: Option<Manifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_root: Option<PathBuf>,
}

fn legacy_custom_label() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatedManifest {
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatedFile {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct ImportReceipt {
    #[serde(flatten)]
    pub installation: Installation,
    pub created_files: Vec<CreatedFile>,
}

#[derive(Debug, Serialize)]
pub struct RetainedFile {
    pub path: PathBuf,
    pub created_sha256: String,
    pub current_sha256: Option<String>,
    pub matches_created_hash: bool,
    pub error: Option<Error>,
}

impl Installation {
    fn confirmation_matches(&self, manifest: &Manifest) -> bool {
        self.confirmed_launch_sha256.as_deref() == Some(manifest.launch_sha256().as_str())
            && self
                .confirmed_root
                .as_ref()
                .is_none_or(|root| root == &self.path)
    }

    /// A diagnostic receipt, not permission to delete. Missing or unreadable files never match.
    pub fn retained_files(&self) -> Vec<RetainedFile> {
        self.created_manifest
            .iter()
            .map(|created| {
                let path = self.path.join(FILENAME);
                let (current_sha256, error) = match storage::fingerprint(&path) {
                    Ok(digest) => (digest, None),
                    Err(error) => (None, Some(error)),
                };
                RetainedFile {
                    path,
                    matches_created_hash: current_sha256.as_deref()
                        == Some(created.sha256.as_str()),
                    current_sha256,
                    created_sha256: created.sha256.clone(),
                    error,
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    #[default]
    Active,
    Dormant,
    Archived,
}

#[derive(Debug, Serialize)]
pub struct Listing {
    #[serde(flatten)]
    pub installation: Installation,
    pub registration_state: &'static str,
    pub manifest_error: Option<Error>,
    pub launch_warnings: Vec<crate::manifest::Issue>,
    pub manifest: Option<Manifest>,
    pub current_branch: Option<String>,
    pub display_label: String,
    pub current_launch_sha256: Option<String>,
    pub launch_changed: Option<bool>,
    pub requires_launch_confirmation: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_version: u32,
    installations: Vec<Installation>,
}

fn load(data_dir: &Path) -> Result<Registry> {
    let mut remaining = u64::MAX;
    load_budgeted(data_dir, &mut remaining)
}

fn load_budgeted(data_dir: &Path, remaining: &mut u64) -> Result<Registry> {
    let path = data_dir.join("registry.json");
    let bytes = match storage::read_budgeted(&path, remaining) {
        Ok(bytes) => bytes,
        Err(e) if e.code == "NOT_FOUND" => return Ok(Registry::default()),
        Err(e) => return Err(e),
    };
    let registry: Registry = serde_json::from_slice(&bytes)
        .map_err(|e| Error::new("REGISTRY_INVALID", e.to_string()))?;
    let mut ids = HashSet::new();
    let mut paths = HashSet::new();
    if registry.schema_version > 1
        || registry.installations.iter().any(|i| {
            i.installation_id.is_nil()
                || i.project_id.is_nil()
                || !i.path.is_absolute()
                || !valid_text(&i.human_label)
                || i.confirmed_root.as_ref().is_some_and(|p| !p.is_absolute())
                || (registry.schema_version == 0
                    && (i.confirmed_manifest.is_some() || i.confirmed_root.is_some()))
                || i.confirmed_launch_sha256
                    .as_ref()
                    .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
                || if let Some(saved) = &i.confirmed_manifest {
                    saved.schema_version != 1
                        || saved.project_id != i.project_id
                        || i.confirmed_root.is_none()
                        || i.confirmed_services.as_ref() != Some(&saved.services)
                        || i.confirmed_launch_sha256.as_deref()
                            != Some(saved.launch_sha256().as_str())
                } else {
                    i.confirmed_services.as_ref().is_some_and(|services| {
                        // Legacy service-only snapshots retain the exact v0 digest,
                        // even when their registry also contains v1 confirmations.
                        let saved = Manifest {
                            open: None,
                            schema_version: 0,
                            project_id: i.project_id,
                            name: String::new(),
                            description: String::new(),
                            services: services.clone(),
                        };
                        services
                            .iter()
                            .any(|s| s.startup_timeout_ms.is_some() || s.stop_timeout_ms.is_some())
                            || i.confirmed_launch_sha256.as_deref()
                                != Some(saved.launch_sha256().as_str())
                    })
                }
                || i.created_manifest.as_ref().is_some_and(|m| {
                    m.sha256.len() != 64 || !m.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                })
                || !ids.insert(i.installation_id)
                || !paths.insert(&i.path)
        })
    {
        return Err(Error::new(
            "REGISTRY_INVALID",
            "Unsupported version or invalid/duplicate installation records",
        ));
    }
    Ok(registry)
}

fn save(data_dir: &Path, registry: &Registry) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(registry)
        .map_err(|e| Error::new("REGISTRY_INVALID", e.to_string()))?;
    if bytes.len() > storage::MAX_BYTES as usize {
        return Err(Error::new("REGISTRY_INVALID", "Registry exceeds 1 MiB"));
    }
    storage::publish(&data_dir.join("registry.json"), &bytes, true)
}

// Called under registry.lock after batch preflight. The original bytes are the
// rollback artifact, not a reconstructed registry with newly defaulted fields.
fn upgrade(data_dir: &Path, registry: &mut Registry) -> Result<()> {
    if registry.schema_version == 1 {
        return Ok(());
    }
    let path = data_dir.join("registry.json");
    if path.exists() {
        let bytes = storage::read(&path)?;
        let backup = data_dir.join("registry.v0.backup.json");
        match storage::read(&backup) {
            Ok(previous) if previous == bytes => {}
            // A failed import can leave a provisional backup while the live registry
            // remains v0. Refresh that v0 snapshot on retry; a published v1 never
            // reaches this branch, so its migration backup remains immutable.
            Ok(previous)
                if serde_json::from_slice::<Registry>(&previous)
                    .is_ok_and(|old| old.schema_version == 0) =>
            {
                storage::publish(&backup, &bytes, true)?;
            }
            Ok(_) => {
                return Err(Error::new(
                    "CONFLICT",
                    "Registry backup is not a v0 registry; preserve or move that file before retrying",
                ));
            }
            Err(error) if error.code == "NOT_FOUND" => storage::publish(&backup, &bytes, false)?,
            Err(error) => return Err(error),
        }
    }
    registry.schema_version = 1;
    Ok(())
}

struct RegistryLock(File);

impl Drop for RegistryLock {
    fn drop(&mut self) {
        // Explicit unlock also releases a briefly inherited descriptor between fork and exec.
        // Closing this process's handle alone can otherwise leave a transient lock behind.
        let _ = FileExt::unlock(&self.0);
    }
}

fn lock(data_dir: &Path) -> Result<RegistryLock> {
    storage::private_directory(data_dir)?;
    let path = data_dir.join("registry.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| storage::io_error(&path, e))?;
    FileExt::try_lock_exclusive(&file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            Error::new("BUSY", "Another registration command is running")
        } else {
            storage::io_error(&path, e)
        }
    })?;
    Ok(RegistryLock(file))
}

fn check(candidate: &Candidate, root: &Path, registry: &Registry) -> Result<Manifest> {
    if !candidate.path.is_absolute() {
        return Err(Error::new(
            "INVALID_INPUT",
            "Candidate path must be absolute",
        ));
    }
    let actual = storage::directory(&candidate.path)?;
    if actual != candidate.path || !actual.starts_with(root) {
        return Err(Error::new(
            "STALE_DRAFT",
            "Candidate path changed or is outside scan root",
        ));
    }
    if !valid_text(&candidate.human_label) {
        return Err(Error::new(
            "VALIDATION_ERROR",
            "human_label must be nonempty and contain no control characters",
        ));
    }
    let manifest = candidate
        .manifest
        .as_ref()
        .ok_or_else(|| Error::new("VALIDATION_ERROR", "Candidate needs a manifest"))?;
    let issues = manifest.validate(&actual);
    if !issues.is_empty() {
        return Err(
            Error::new("VALIDATION_ERROR", "Invalid candidate manifest").details(json!(issues))
        );
    }
    let text = manifest.to_toml()?;
    if text.len() > storage::MAX_BYTES as usize {
        return Err(Error::new("INVALID_INPUT", "Manifest exceeds 1 MiB"));
    }
    let mut inputs = HashSet::new();
    let extras: HashSet<_> = candidate.rule_inputs.iter().map(String::as_str).collect();
    if extras.len() != candidate.rule_inputs.len()
        || extras.len() + INPUTS.len() > 32
        || extras
            .iter()
            .any(|name| !crate::rules::valid_input_name(name) || INPUTS.contains(name))
    {
        return Err(Error::new(
            "INVALID_INPUT",
            "Invalid additional rule inputs",
        ));
    }
    let sources: HashSet<_> = candidate.source_dirs.iter().collect();
    if sources.len() != candidate.source_dirs.len()
        || sources.len() > 32
        || sources.iter().any(|s| !crate::rules::valid_input_name(s))
    {
        return Err(Error::new(
            "INVALID_INPUT",
            "Invalid grouped source directories",
        ));
    }
    let base: Vec<_> = INPUTS
        .iter()
        .copied()
        .chain(extras.iter().copied())
        .collect();
    let mut expected: HashSet<String> = base.iter().map(|s| s.to_string()).collect();
    for source in &candidate.source_dirs {
        crate::manifest::relative_directory(&actual, source)?;
        expected.extend(base.iter().map(|name| format!("{source}/{name}")));
    }
    for evidence in &candidate.evidence {
        if !expected.contains(&evidence.path) || !inputs.insert(evidence.path.as_str()) {
            return Err(Error::new(
                "INVALID_INPUT",
                "Unknown or duplicate evidence path",
            ));
        }
        let now = storage::fingerprint(&actual.join(&evidence.path))?;
        if now != evidence.sha256 {
            // An earlier attempt may have published the manifest before registry persistence failed.
            // Resume only if the exact declared meaning matches; never delete or overwrite it.
            if evidence.path == FILENAME
                && evidence.sha256.is_none()
                && now.is_some()
                && Manifest::read(&actual.join(FILENAME))? == *manifest
            {
                continue;
            }
            return Err(Error::new(
                "STALE_DRAFT",
                format!(
                    "{} changed; scan again",
                    actual.join(&evidence.path).display()
                ),
            ));
        }
    }
    let from_manifest = candidate.source_dirs.is_empty()
        && candidate.evidence.len() == 1
        && candidate.evidence[0].path == FILENAME
        && candidate.evidence[0].sha256.is_some();
    if !from_manifest
        && (inputs.len() != expected.len() || !expected.iter().all(|p| inputs.contains(p.as_str())))
    {
        return Err(Error::new(
            "INVALID_INPUT",
            "Draft is missing discovery evidence; scan again",
        ));
    }
    if storage::fingerprint(&actual.join(FILENAME))?.is_some()
        && Manifest::read(&actual.join(FILENAME))? != *manifest
    {
        return Err(Error::new(
            "CONFLICT",
            "Existing apphost.toml differs from draft; edit it explicitly and scan again",
        ));
    }
    if registry
        .installations
        .iter()
        .any(|i| i.path == actual && i.project_id != manifest.project_id)
    {
        return Err(Error::new(
            "CONFLICT",
            "This path is already associated with another project_id",
        ));
    }
    if let Some(existing) = registry.installations.iter().find(|i| i.path == actual) {
        let current = manifest.launch_sha256();
        if !existing.confirmation_matches(manifest)
            && candidate.reviewed_launch_sha256.as_deref() != Some(current.as_str())
        {
            return Err(Error::new("REVIEW_REQUIRED", "Launch declaration changed or lacks prior confirmation; explicitly confirm it with review before import")
                .details(json!({"path":actual,"previous_sha256":existing.confirmed_launch_sha256,"current_sha256":current})));
        }
    }
    Ok(manifest.clone())
}

pub fn import(data_dir: &Path, draft: &Draft) -> Result<Vec<Installation>> {
    Ok(import_with_receipts(data_dir, draft)?
        .into_iter()
        .map(|r| r.installation)
        .collect())
}

pub fn import_with_receipts(data_dir: &Path, draft: &Draft) -> Result<Vec<ImportReceipt>> {
    if draft.schema_version > 1 {
        return Err(Error::new(
            "INVALID_INPUT",
            "Only draft versions 0 and 1 are supported",
        ));
    }
    if !draft.root.is_absolute() || storage::directory(&draft.root)? != draft.root {
        return Err(Error::new(
            "STALE_DRAFT",
            "Scan root changed or is not absolute",
        ));
    }
    let selected: Vec<_> = draft.candidates.iter().filter(|c| c.selected).collect();
    if selected.is_empty() {
        return Ok(vec![]);
    }
    let _lock = lock(data_dir)?;
    let mut registry = load(data_dir)?;
    let mut paths = HashSet::new();
    for candidate in &selected {
        if draft.schema_version == 0
            && candidate
                .manifest
                .as_ref()
                .is_some_and(|m| m.schema_version != 0)
        {
            return Err(Error::new(
                "INVALID_INPUT",
                "Manifest v1 requires draft version 1",
            ));
        }
        if !paths.insert(&candidate.path) {
            return Err(Error::new(
                "INVALID_INPUT",
                "Duplicate selected candidate path",
            ));
        }
        check(candidate, &draft.root, &registry)?;
    }
    for parent in &selected {
        if selected
            .iter()
            .any(|child| child.path != parent.path && child.path.starts_with(&parent.path))
        {
            return Err(Error::new(
                "CONFLICT",
                "Selected App roots overlap; confirm grouping before import",
            ));
        }
    }
    if selected
        .iter()
        .any(|c| c.manifest.as_ref().is_some_and(|m| m.schema_version == 1))
    {
        upgrade(data_dir, &mut registry)?;
    }
    let mut completed = Vec::new();
    for candidate in selected {
        let mut created_files = Vec::new();
        let result = (|| {
            // Recheck immediately before writing as well as before any item in the batch is written.
            let manifest = check(candidate, &draft.root, &registry)?;
            let path = candidate.path.join(FILENAME);
            if storage::fingerprint(&path)?.is_none() {
                let text = manifest.to_toml()?;
                if text.len() > storage::MAX_BYTES as usize {
                    return Err(Error::new("INVALID_INPUT", "Manifest exceeds 1 MiB"));
                }
                storage::publish(&path, text.as_bytes(), false)?;
                // Record the bytes we actually published, never the content of a pre-existing file.
                created_files.push(CreatedFile {
                    path: path.clone(),
                    sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
                });
            }
            if let Some(existing) = registry
                .installations
                .iter()
                .find(|i| i.path == candidate.path && i.detached)
            {
                let id = existing.installation_id;
                let label =
                    unique_label(&registry, &existing.human_label, &existing.path, Some(id));
                registry
                    .installations
                    .iter_mut()
                    .find(|i| i.installation_id == id)
                    .unwrap()
                    .human_label = label;
            }
            let installation = if let Some(existing) = registry
                .installations
                .iter_mut()
                .find(|i| i.path == candidate.path)
            {
                existing.detached = false;
                // Explicit import is confirmation for both generated and project-owned manifests.
                existing.confirmed_launch_sha256 = Some(manifest.launch_sha256());
                existing.confirmed_services = Some(manifest.services.clone());
                existing.confirmed_manifest =
                    (manifest.schema_version == 1).then(|| manifest.clone());
                existing.confirmed_root =
                    (registry.schema_version == 1).then(|| existing.path.clone());
                if let Some(file) = created_files.first() {
                    existing.created_manifest = Some(CreatedManifest {
                        sha256: file.sha256.clone(),
                    });
                }
                // Re-import preserves the user's existing label. label is the explicit edit operation.
                existing.clone()
            } else {
                let id = Uuid::new_v4();
                let human_label =
                    unique_label(&registry, &candidate.human_label, &candidate.path, None);
                let installation = Installation {
                    installation_id: id,
                    project_id: manifest.project_id,
                    path: candidate.path.clone(),
                    human_label,
                    label_is_custom: candidate.human_label
                        != candidate
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                    detached: false,
                    retention_policy: RetentionPolicy::Active,
                    created_manifest: created_files.first().map(|f| CreatedManifest {
                        sha256: f.sha256.clone(),
                    }),
                    confirmed_launch_sha256: Some(manifest.launch_sha256()),
                    confirmed_services: Some(manifest.services.clone()),
                    confirmed_manifest: (manifest.schema_version == 1).then(|| manifest.clone()),
                    confirmed_root: (registry.schema_version == 1).then(|| candidate.path.clone()),
                };
                registry.installations.push(installation.clone());
                installation
            };
            save(data_dir, &registry)?;
            Ok(installation)
        })();
        match result {
            Ok(installation) => completed.push(ImportReceipt {
                installation,
                created_files,
            }),
            Err(mut e) => {
                e.details = json!({"completed": completed, "failed_path": candidate.path, "created_files":created_files, "cause": e.details,
                    "recovery": "Any published apphost.toml is retained. Resolve the error, then retry; scan again if stale."});
                return Err(e);
            }
        }
    }
    Ok(completed)
}

pub fn list(data_dir: &Path, include_detached: bool) -> Result<Vec<Listing>> {
    let mut remaining = crate::discovery::MAX_READ_BYTES;
    load(data_dir)?
        .installations
        .into_iter()
        .filter(|i| include_detached || !i.detached)
        .map(|installation| {
            let mut manifest_error = None;
            let mut launch_warnings = Vec::new();
            let mut manifest = None;
            let registration_state = if installation.detached {
                "DETACHED"
            } else {
                match fs::metadata(&installation.path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => "SOURCE_MISSING",
                    Err(e) => return Err(storage::io_error(&installation.path, e)),
                    Ok(meta) if !meta.is_dir() => "SOURCE_MISSING",
                    Ok(_) => {
                        match crate::validate(&installation.path) {
                            Ok(m) if m.project_id != installation.project_id => {
                                manifest_error = Some(Error::new(
                                    "CONFLICT",
                                    "Project identity differs from registration",
                                ))
                            }
                            Ok(m) => {
                                launch_warnings = m.launch_warnings(&installation.path);
                                manifest = Some(m);
                            }
                            Err(e) => manifest_error = Some(e),
                        }
                        "REGISTERED"
                    }
                }
            };
            let current_launch_sha256 = manifest.as_ref().map(Manifest::launch_sha256);
            let launch_changed = installation.confirmed_launch_sha256.as_ref()
                .zip(current_launch_sha256.as_ref()).map(|(confirmed, current)| confirmed != current);
            let requires_launch_confirmation = manifest.as_ref().is_none_or(|m| !installation.confirmation_matches(m));
            if requires_launch_confirmation {
                launch_warnings.push(crate::manifest::Issue::new("launch_confirmation", match launch_changed {
                    Some(true) => "Launch declaration changed after confirmation; review and import again before launch",
                    _ => "No matching confirmed launch declaration is available; explicit confirmation is required",
                }));
            }
            let current_branch = match crate::git_metadata::current_branch(&installation.path, &mut remaining) {
                Ok(branch) => branch,
                Err(e) => {
                    launch_warnings.push(crate::manifest::Issue::new("git", e.to_string()));
                    None
                }
            };
            let display_label = match (&current_branch, installation.label_is_custom) {
                (Some(branch), false) => format!("{} · {branch}", installation.human_label),
                _ => installation.human_label.clone(),
            };
            Ok(Listing {
                installation,
                registration_state,
                manifest_error,
                launch_warnings,
                manifest,
                current_branch,
                display_label,
                current_launch_sha256,
                launch_changed,
                requires_launch_confirmation,
            })
        })
        .collect()
}

pub fn detach(data_dir: &Path, id: Uuid) -> Result<Installation> {
    update(data_dir, id, |installation| {
        installation.detached = true;
    })
}

pub fn label(data_dir: &Path, id: Uuid, value: &str) -> Result<Installation> {
    if !valid_text(value) {
        return Err(Error::new(
            "VALIDATION_ERROR",
            "Label must be nonempty and contain no control characters",
        ));
    }
    let _lock = lock(data_dir)?;
    let mut registry = load(data_dir)?;
    if registry
        .installations
        .iter()
        .any(|i| i.installation_id != id && !i.detached && i.human_label == value)
    {
        return Err(Error::new(
            "CONFLICT",
            "Another managed installation already uses this label",
        ));
    }
    let installation = registry
        .installations
        .iter_mut()
        .find(|i| i.installation_id == id)
        .ok_or_else(|| Error::new("NOT_FOUND", "Unknown installation_id"))?;
    installation.human_label = value.into();
    installation.label_is_custom = true;
    let result = installation.clone();
    save(data_dir, &registry)?;
    Ok(result)
}

fn unique_label(registry: &Registry, base: &str, path: &Path, except: Option<Uuid>) -> String {
    let used = |label: &str| {
        registry
            .installations
            .iter()
            .any(|i| Some(i.installation_id) != except && !i.detached && i.human_label == label)
    };
    if !used(base) {
        return base.into();
    }
    // Human labels never expose a full absolute path or a machine identity as the disambiguator.
    let directory = path.file_name().unwrap_or_default().to_string_lossy();
    let stem = if directory == base {
        base.into()
    } else {
        format!("{base} · {directory}")
    };
    if !used(&stem) {
        return stem;
    }
    for n in 2.. {
        let value = format!("{stem} · {n}");
        if !used(&value) {
            return value;
        }
    }
    unreachable!()
}

pub fn retention(data_dir: &Path, id: Uuid, policy: RetentionPolicy) -> Result<Installation> {
    update(data_dir, id, |i| i.retention_policy = policy)
}

pub fn detach_many(data_dir: &Path, ids: &[Uuid]) -> Result<Vec<Installation>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let _lock = lock(data_dir)?;
    let mut registry = load(data_dir)?;
    for id in ids {
        if !registry
            .installations
            .iter()
            .any(|i| i.installation_id == *id)
        {
            return Err(Error::new(
                "NOT_FOUND",
                "Unknown installation_id; no registrations changed",
            ));
        }
    }
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            continue;
        }
        let installation = registry
            .installations
            .iter_mut()
            .find(|i| i.installation_id == *id)
            .unwrap();
        installation.detached = true;
        results.push(installation.clone());
    }
    save(data_dir, &registry)?;
    Ok(results)
}

pub fn relink(data_dir: &Path, id: Uuid, path: &Path) -> Result<Installation> {
    let target = storage::directory(path)?;
    let manifest = crate::validate(&target)?;
    let _lock = lock(data_dir)?;
    let mut registry = load(data_dir)?;
    let index = registry
        .installations
        .iter()
        .position(|i| i.installation_id == id)
        .ok_or_else(|| Error::new("NOT_FOUND", "Unknown installation_id"))?;
    let old = &registry.installations[index];
    if old.project_id != manifest.project_id {
        return Err(Error::new(
            "CONFLICT",
            "Target manifest has a different project_id",
        ));
    }
    if old.path == target {
        return Ok(old.clone());
    }
    if registry.installations.iter().any(|i| i.path == target) {
        return Err(Error::new("CONFLICT", "Target path is already registered"));
    }
    match fs::metadata(&old.path) {
        Ok(m) if m.is_dir() => {
            return Err(Error::new(
                "CONFLICT",
                "Source still exists; a copy is a separate installation",
            ));
        }
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(storage::io_error(&old.path, e));
        }
        _ => {}
    }
    upgrade(data_dir, &mut registry)?;
    // Keep the old snapshot for review, but never transfer its execution root.
    if registry.installations[index].confirmed_root.is_none() {
        registry.installations[index].confirmed_root =
            Some(registry.installations[index].path.clone());
    }
    registry.installations[index].path = target;
    let result = registry.installations[index].clone();
    save(data_dir, &registry)?;
    Ok(result)
}

pub fn associate(data_dir: &Path, draft: &mut Draft, path: &Path, id: Uuid) -> Result<()> {
    let registry = load(data_dir)?;
    let installation = registry
        .installations
        .iter()
        .find(|i| i.installation_id == id)
        .ok_or_else(|| Error::new("NOT_FOUND", "Unknown installation_id"))?;
    let path = storage::directory(path)?;
    let candidate = draft
        .candidates
        .iter_mut()
        .find(|c| c.path == path)
        .ok_or_else(|| Error::new("NOT_FOUND", "Path is not a draft candidate"))?;
    if storage::fingerprint(&path.join(FILENAME))?.is_some()
        && Manifest::read(&path.join(FILENAME))?.project_id != installation.project_id
    {
        return Err(Error::new(
            "CONFLICT",
            "Existing manifest identity cannot be replaced by association",
        ));
    }
    let manifest = candidate
        .manifest
        .as_mut()
        .ok_or_else(|| Error::new("VALIDATION_ERROR", "Candidate needs a manifest"))?;
    manifest.project_id = installation.project_id;
    Ok(())
}

fn update(
    data_dir: &Path,
    id: Uuid,
    mutate: impl FnOnce(&mut Installation),
) -> Result<Installation> {
    let _lock = lock(data_dir)?;
    let mut registry = load(data_dir)?;
    let installation = registry
        .installations
        .iter_mut()
        .find(|i| i.installation_id == id)
        .ok_or_else(|| Error::new("NOT_FOUND", "Unknown installation_id"))?;
    mutate(installation);
    let result = installation.clone();
    save(data_dir, &registry)?;
    Ok(result)
}

/// Refresh trust diagnostics for CLI review, including drafts created without registry context.
pub fn annotate_launch_changes(data_dir: &Path, draft: &mut Draft) -> Result<()> {
    mark_launch_changes(&load(data_dir)?, draft);
    Ok(())
}

fn mark_launch_changes(registry: &Registry, draft: &mut Draft) {
    use crate::manifest::Issue;
    for candidate in &mut draft.candidates {
        candidate.notes.retain(|n| n.field != "launch_change");
        let Some(previous) = registry
            .installations
            .iter()
            .find(|i| i.path == candidate.path)
        else {
            continue;
        };
        let Some(manifest) = &candidate.manifest else {
            continue;
        };
        let current = manifest.launch_sha256();
        if previous.confirmation_matches(manifest) {
            continue;
        }
        if candidate.reviewed_launch_sha256.as_deref() != Some(current.as_str()) {
            candidate.selected = false;
        }
        candidate.notes.push(Issue::new(
            "launch_change",
            "启动声明已变更或缺少历史确认；必须通过 review 明确确认",
        ));
        if previous
            .confirmed_root
            .as_ref()
            .is_some_and(|p| p != &previous.path)
        {
            candidate.notes.push(Issue::new(
                "launch_change",
                "执行根目录已改变；旧确认不适用于新目录",
            ));
        }
        if manifest.schema_version == 1 || previous.confirmed_manifest.is_some() {
            candidate.notes.push(Issue::new(
                "launch_change",
                format!(
                    "入口：旧 {}；新 {}",
                    previous
                        .confirmed_manifest
                        .as_ref()
                        .map(|m| m.open_summary())
                        .unwrap_or_else(|| "入口未保存，无法还原旧声明".into()),
                    manifest.open_summary()
                ),
            ));
        }
        if let Some(services) = &previous.confirmed_services {
            for service in services {
                candidate.notes.push(Issue::new(
                    "launch_change",
                    format!("旧 {}  {}", service.name, service.launch_summary()),
                ));
            }
        } else {
            // A hash cannot recover old commands. Never infer them from the current manifest.
            candidate
                .notes
                .push(Issue::new("launch_change", "旧命令未保存，无法还原旧声明"));
        }
        for service in &manifest.services {
            candidate.notes.push(Issue::new(
                "launch_change",
                format!("新 {}  {}", service.name, service.launch_summary()),
            ));
        }
    }
}

/// Optional copy hints read current manifest names, not cached identity guesses or Git URLs.
/// Consume the scan's remaining budget; unavailable hint sources never look like a complete match search.
pub(crate) fn annotate_copies(data_dir: &Path, draft: &mut Draft, remaining: &mut u64) {
    use crate::manifest::Issue;
    let mut budget = (*remaining).min(8 * 1024 * 1024);
    let initial = budget;
    let result = (|| -> Result<()> {
        let registry = load_budgeted(data_dir, &mut budget)?;
        // This comparison needs only the registry and existing candidate snapshots. Run it
        // before optional copy-name reads, so an unavailable sibling cannot hide a trust change.
        mark_launch_changes(&registry, draft);
        let mut names: std::collections::BTreeMap<String, Vec<&Installation>> =
            std::collections::BTreeMap::new();
        let mut unavailable = false;
        for installation in registry.installations.iter().filter(|i| !i.detached) {
            let bytes = match storage::read_budgeted(&installation.path.join(FILENAME), &mut budget)
            {
                Ok(bytes) => bytes,
                Err(e) if e.code == "SCAN_LIMIT" => return Err(e),
                Err(_) => {
                    unavailable = true;
                    continue;
                }
            };
            let manifest = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|s| toml::from_str::<Manifest>(s).ok());
            match manifest {
                Some(m)
                    if m.project_id == installation.project_id
                        && m.schema_version <= 1
                        && valid_text(&m.name) =>
                {
                    names.entry(m.name).or_default().push(installation);
                }
                _ => unavailable = true,
            }
        }
        for c in draft
            .candidates
            .iter_mut()
            .chain(draft.groups.iter_mut().map(|g| &mut g.candidate))
        {
            c.notes.retain(|n| n.field != "possible_copy");
            let Some(manifest) = &c.manifest else {
                continue;
            };
            if let Some(matches) = names.get(&manifest.name) {
                let matches: Vec<_> = matches.iter().filter(|i| i.path != c.path).collect();
                for i in matches.iter().take(8) {
                    c.notes.push(Issue::new("possible_copy", format!("Same project name as {:?} at {} (installation_id={}); possible copy only, use associate explicitly if these are the same project", i.human_label, i.path.display(), i.installation_id)));
                }
                if matches.len() > 8 {
                    c.notes.push(Issue::new(
                        "possible_copy",
                        "More matching installations omitted; inspect list before associating",
                    ));
                }
            }
        }
        if unavailable {
            draft.issues.push(Issue::new("copy_hints", "Some registered manifests were unavailable or inconsistent; copy hints are incomplete"));
        }
        Ok(())
    })();
    *remaining -= initial - budget;
    if let Err(e) = result {
        draft.issues.push(Issue::new(
            "copy_hints",
            format!("Copy hints unavailable or incomplete: {e}"),
        ));
    }
}
