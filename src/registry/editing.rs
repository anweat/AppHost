//! Text buffers belong to the caller. Only a reviewed save mutates the source.
use super::*;
use crate::journal::Actor;
use serde_json::Value;

pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn read_buffer(path: &Path) -> Result<String> {
    String::from_utf8(storage::read(path)?)
        .map_err(|_| Error::new("VALIDATION_ERROR", "Edit buffer must be UTF-8"))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRequest {
    pub revision: u64,
    pub base_sha256: String,
    pub confirmed_launch_sha256: String,
    // None confirms an external edit without writing its bytes again.
    pub text: Option<String>,
}
fn source(data: &Path, installation: &Installation) -> Result<(PathBuf, String)> {
    if installation.detached {
        return Err(Error::new(
            "DETACHED",
            "Re-register the installation before editing",
        ));
    }
    let root = storage::directory(&installation.path)?;
    if root != installation.path {
        return Err(Error::new(
            "EDIT_CONFLICT",
            "Registered project root changed",
        ));
    }
    let path = sources::path(data, installation);
    let bytes = storage::read(&path)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::new("VALIDATION_ERROR", "Manifest text must be UTF-8"))?;
    Ok((path, text))
}
pub fn read(data: &Path, id: Uuid) -> Result<Value> {
    let installation = registered(data, id)?;
    let (path, text) = source(data, &installation)?;
    Ok(
        json!({"installation_id":id,"revision":installation.revision,
        "project_root":installation.path,"manifest_source":installation.manifest_source,"manifest_path":path,
        "base_sha256":digest(text.as_bytes()),"text":text}),
    )
}
fn parse(installation: &Installation, text: &str) -> Result<Manifest> {
    if text.len() > storage::MAX_BYTES as usize {
        return Err(Error::new("INVALID_INPUT", "Manifest exceeds 1 MiB"));
    }
    let manifest: Manifest =
        toml::from_str(text).map_err(|e| Error::new("VALIDATION_ERROR", e.to_string()))?;
    if manifest.project_id != installation.project_id {
        return Err(Error::new(
            "CONFLICT",
            "Editing cannot replace project identity",
        ));
    }
    let issues = manifest.validate(&installation.path);
    if !issues.is_empty() {
        return Err(Error::new("VALIDATION_ERROR", "Invalid manifest").details(json!(issues)));
    }
    Ok(manifest)
}
pub fn review(data: &Path, id: Uuid, text: Option<&str>) -> Result<Value> {
    let installation = registered(data, id)?;
    let (path, current) = source(data, &installation)?;
    let proposed = text.unwrap_or(&current);
    let manifest = parse(&installation, proposed)?;
    Ok(
        json!({"installation_id":id,"revision":installation.revision,
        "project_root":installation.path,"manifest_source":installation.manifest_source,"manifest_path":path,
        "base_sha256":digest(current.as_bytes()),"before_text":current,"after_text":proposed,
        "previous_confirmation":{"manifest":installation.confirmed_manifest,"services":installation.confirmed_services,
            "launch_sha256":installation.confirmed_launch_sha256,"actor":installation.confirmed_actor,"at_ms":installation.confirmed_at_ms},
        "manifest":manifest,"launch_sha256":manifest.launch_sha256(),
        "warnings":manifest.launch_warnings(&installation.path)}),
    )
}
fn checked(
    data: &Path,
    installation: &Installation,
    request: &SaveRequest,
) -> Result<(PathBuf, String, Manifest)> {
    let (path, current) = source(data, installation)?;
    if installation.revision != request.revision
        || digest(current.as_bytes()) != request.base_sha256
    {
        return Err(Error::new(
            "EDIT_CONFLICT",
            "Manifest or registration changed; reload or merge the buffer",
        )
        .details(
            json!({"revision":installation.revision,"current_sha256":digest(current.as_bytes())}),
        ));
    }
    let proposed = request.text.as_deref().unwrap_or(&current);
    let manifest = parse(installation, proposed)?;
    if manifest.launch_sha256() != request.confirmed_launch_sha256 {
        return Err(Error::new(
            "REVIEW_REQUIRED",
            "Confirmation does not match the proposed declaration",
        ));
    }
    Ok((path, proposed.to_owned(), manifest))
}
/// Reject stale or invalid input before asking a live installation to stop.
pub fn preflight(data: &Path, id: Uuid, request: &SaveRequest) -> Result<()> {
    checked(data, &registered(data, id)?, request).map(|_| ())
}
/// L0 commit; callers coordinate active instances through the daemon first.
pub fn save(data: &Path, id: Uuid, request: &SaveRequest, actor: Actor) -> Result<Value> {
    let _lock = lock(data)?;
    let mut registry = load(data)?;
    let index = registry
        .installations
        .iter()
        .position(|i| i.installation_id == id)
        .ok_or_else(|| Error::new("NOT_FOUND", "Unknown installation_id"))?;
    let (path, text, manifest) = checked(data, &registry.installations[index], request)?;
    // Keep the migration backup of the original registry before publishing any
    // project bytes. A failed confirmation never recomputes the historical hash.
    upgrade(data, &mut registry)?;
    if request.text.is_some() {
        storage::publish(&path, text.as_bytes(), true)?;
    }
    let installation = &mut registry.installations[index];
    installation.confirmed_launch_sha256 = Some(manifest.launch_sha256());
    installation.confirmed_services = Some(manifest.services.clone());
    installation.confirmed_manifest = (manifest.schema_version == 1).then_some(manifest);
    installation.confirmed_root = Some(installation.path.clone());
    installation.confirmed_actor = Some(actor);
    installation.confirmed_at_ms = Some(now());
    if let Err(error) = super::save(data, &mut registry) {
        return Err(Error::new("MANIFEST_SAVED_UNCONFIRMED", "Manifest is saved but its new confirmation could not be published")
            .details(json!({"manifest_path":path,"sha256":digest(text.as_bytes()),"file_written":request.text.is_some(),"cause":error})));
    }
    Ok(
        json!({"installation":registry.installations[index],"manifest_path":path,
        "sha256":digest(text.as_bytes()),"file_written":request.text.is_some(),"confirmed":true}),
    )
}

/// Shared CLI/desktop boundary: direct L0 only when no instance needs recovery.
pub fn save_coordinated(data: &Path, id: Uuid, request: SaveRequest) -> Result<Value> {
    let actor = Actor::from_environment()?;
    preflight(data, id, &request)?;
    match crate::daemon::coordinate(
        data,
        id,
        "manifest-save",
        json!({"installation_id":id,"edit":request}),
    )? {
        Some(result) => Ok(result),
        None => crate::journal::record_result(
            data,
            "manifest-save",
            actor,
            Some(id),
            save(data, id, &request, actor),
        ),
    }
}
