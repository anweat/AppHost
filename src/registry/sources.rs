//! One explicit source per installation; never merge or prefer a newly found file.
use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ManifestSource {
    #[default]
    Project,
    External,
}
impl ManifestSource {
    pub fn is_project(&self) -> bool {
        *self == Self::Project
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::External => "external",
        }
    }
}
pub fn external_path(data: &Path, id: Uuid) -> PathBuf {
    data.join("manifests").join(format!("{id}.toml"))
}
pub fn path(data: &Path, installation: &Installation) -> PathBuf {
    match installation.manifest_source {
        ManifestSource::Project => installation.path.join(FILENAME),
        ManifestSource::External => external_path(data, installation.installation_id),
    }
}
pub fn validate_at(path: &Path, project_root: &Path) -> Result<Manifest> {
    let root = storage::directory(project_root)?;
    let manifest = Manifest::read(path)?;
    let issues = manifest.validate(&root);
    if !issues.is_empty() {
        return Err(Error::new("VALIDATION_ERROR", "Invalid manifest").details(json!(issues)));
    }
    Ok(manifest)
}
pub fn resolve(data: &Path, installation: &Installation) -> Result<Manifest> {
    validate_at(&path(data, installation), &installation.path)
}
impl Installation {
    pub fn retained_files_in(&self, data: &Path) -> Vec<RetainedFile> {
        let mut files = self.retained_files();
        if let Some(created) = &self.external_manifest_sha256 {
            let path = external_path(data, self.installation_id);
            let (current_sha256, error) = match storage::fingerprint(&path) {
                Ok(hash) => (hash, None),
                Err(error) => (None, Some(error)),
            };
            files.push(RetainedFile {
                path,
                matches_created_hash: current_sha256.as_deref() == Some(created),
                current_sha256,
                created_sha256: created.clone(),
                error,
            });
        }
        files
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalTarget {
    // Reserved in the v2 draft so a failed registration can retry the same file.
    pub installation_id: Uuid,
    pub sha256: Option<String>,
}
/// Select external storage only for new inferred candidates. Existing sources require a switch.
pub fn select_external(data: &Path, draft: &mut Draft) -> Result<()> {
    let registry = load(data)?;
    for c in &draft.candidates {
        if c.external.is_none()
            && storage::fingerprint(&c.path.join(FILENAME))?.is_none()
            && registry.installations.iter().any(|i| i.path == c.path)
        {
            return Err(Error::new(
                "SOURCE_CONFLICT",
                "Switch an existing installation's source explicitly",
            ));
        }
    }
    for c in draft
        .candidates
        .iter_mut()
        .chain(draft.groups.iter_mut().map(|g| &mut g.candidate))
    {
        if c.external.is_none() && storage::fingerprint(&c.path.join(FILENAME))?.is_none() {
            c.external = Some(ExternalTarget {
                installation_id: Uuid::new_v4(),
                sha256: None,
            });
        }
    }
    if draft.candidates.iter().any(|c| c.external.is_some()) {
        draft.schema_version = 2;
    }
    Ok(())
}
pub(crate) fn external_installations(
    data: &Path,
    remaining: &mut u64,
) -> Result<Vec<Installation>> {
    Ok(load_budgeted(data, remaining)?
        .installations
        .into_iter()
        .filter(|i| i.manifest_source == ManifestSource::External)
        .collect())
}
pub(crate) fn candidate(data: &Path, i: &Installation, remaining: &mut u64) -> Result<Candidate> {
    let project_hash = storage::fingerprint_budgeted(&i.path.join(FILENAME), remaining)?;
    let external_path = external_path(data, i.installation_id);
    let mut issues = Vec::new();
    let mut hash = None;
    let manifest = match storage::read_budgeted(&external_path, remaining) {
        Ok(bytes) => {
            hash = Some(format!("{:x}", Sha256::digest(&bytes)));
            std::str::from_utf8(&bytes)
                .map_err(|e| Error::new("VALIDATION_ERROR", e.to_string()))
                .and_then(|text| {
                    toml::from_str::<Manifest>(text)
                        .map_err(|e| Error::new("VALIDATION_ERROR", e.to_string()))
                })
        }
        Err(error) if error.code == "SCAN_LIMIT" => return Err(error),
        Err(error) => Err(error),
    };
    let manifest = match manifest {
        Ok(m) => {
            issues.extend(m.validate(&i.path));
            if m.project_id != i.project_id {
                issues.push(crate::manifest::Issue::new(
                    "project_id",
                    "External declaration has a different project identity",
                ));
            }
            Some(m)
        }
        Err(e) => {
            issues.push(crate::manifest::Issue::new("manifest", e.to_string()));
            None
        }
    };
    let mut notes = Vec::new();
    if project_hash.is_some() {
        notes.push(crate::manifest::Issue::new("manifest_source", "Project apphost.toml also exists; keep the external source explicitly in review or switch sources"));
    }
    Ok(Candidate {
        path: i.path.clone(),
        selected: issues.is_empty() && project_hash.is_none(),
        human_label: i.human_label.clone(),
        manifest,
        evidence: vec![crate::discovery::Evidence {
            path: FILENAME.into(),
            sha256: project_hash,
        }],
        external: Some(ExternalTarget {
            installation_id: i.installation_id,
            sha256: hash,
        }),
        issues,
        notes,
        nested_projects: vec![],
        hints_truncated: false,
        rule_inputs: vec![],
        suggestions: Default::default(),
        source_dirs: vec![],
        reviewed_launch_sha256: None,
    })
}

pub(super) fn check_target(
    data: &Path,
    c: &Candidate,
    registry: &Registry,
    manifest: &Manifest,
) -> Result<()> {
    let previous = registry.installations.iter().find(|i| i.path == c.path);
    let Some(target) = &c.external else {
        if previous.is_some_and(|i| i.manifest_source != ManifestSource::Project) {
            return Err(Error::new(
                "SOURCE_CONFLICT",
                "Draft source differs from the registered source; scan again",
            ));
        }
        return Ok(());
    };
    if target.installation_id.is_nil()
        || registry
            .installations
            .iter()
            .any(|i| i.installation_id == target.installation_id && i.path != c.path)
        || previous.is_some_and(|i| {
            i.installation_id != target.installation_id
                || i.manifest_source != ManifestSource::External
        })
    {
        return Err(Error::new(
            "SOURCE_CONFLICT",
            "External draft installation or source changed",
        ));
    }
    let path = external_path(data, target.installation_id);
    let current = storage::fingerprint(&path)?;
    if current != target.sha256 {
        // A failed first import may leave exactly this declaration. Never overwrite it.
        if target.sha256.is_some() || current.is_none() || Manifest::read(&path)? != *manifest {
            return Err(Error::new(
                "STALE_DRAFT",
                "External manifest changed; scan again",
            ));
        }
    }
    if current.is_some() && Manifest::read(&path)? != *manifest {
        return Err(Error::new(
            "CONFLICT",
            "Edit the external declaration explicitly before importing",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchRequest {
    pub revision: u64,
    pub base_sha256: String,
    pub target_source: ManifestSource,
    pub target_sha256: Option<String>,
    pub confirmed_launch_sha256: String,
}
fn switch_target(
    data: &Path,
    i: &Installation,
    target: ManifestSource,
) -> Result<(PathBuf, Option<String>, String, Manifest)> {
    if i.detached {
        return Err(Error::new(
            "DETACHED",
            "Re-register before switching source",
        ));
    }
    if storage::directory(&i.path)? != i.path {
        return Err(Error::new("EDIT_CONFLICT", "Project root changed"));
    }
    if i.manifest_source == target {
        return Err(Error::new("SOURCE_CONFLICT", "Source is already active"));
    }
    let mut next = i.clone();
    next.manifest_source = target;
    let path = path(data, &next);
    let hash = storage::fingerprint(&path)?;
    let current_path = super::sources::path(data, i);
    let text = super::editing::read_buffer(if hash.is_some() { &path } else { &current_path })?;
    let manifest: Manifest =
        toml::from_str(&text).map_err(|e| Error::new("VALIDATION_ERROR", e.to_string()))?;
    if manifest.project_id != i.project_id {
        return Err(Error::new(
            "CONFLICT",
            "Target has a different project identity",
        ));
    }
    let issues = manifest.validate(&i.path);
    if !issues.is_empty() {
        return Err(Error::new("VALIDATION_ERROR", "Invalid switch target").details(json!(issues)));
    }
    Ok((path, hash, text, manifest))
}
pub fn review_switch(data: &Path, id: Uuid, target: ManifestSource) -> Result<serde_json::Value> {
    let i = registered(data, id)?;
    let before = super::editing::read(data, id)?;
    let (path, hash, text, manifest) = switch_target(data, &i, target)?;
    Ok(
        json!({"installation_id":id,"revision":i.revision,"from_source":i.manifest_source,
        "from_path":before["manifest_path"],"base_sha256":before["base_sha256"],"before_text":before["text"],
        "target_source":target,"target_path":path,"target_sha256":hash,"after_text":text,
        "manifest":manifest,"launch_sha256":manifest.launch_sha256(),"warnings":manifest.launch_warnings(&i.path),"original_retained":true}),
    )
}
fn checked_switch(
    data: &Path,
    i: &Installation,
    request: &SwitchRequest,
) -> Result<(PathBuf, Option<String>, String, Manifest)> {
    let before = storage::fingerprint(&path(data, i))?;
    if i.revision != request.revision || before.as_deref() != Some(&request.base_sha256) {
        return Err(Error::new(
            "EDIT_CONFLICT",
            "Registration or active source changed; review again",
        ));
    }
    let target = switch_target(data, i, request.target_source)?;
    if target.1 != request.target_sha256 {
        return Err(Error::new(
            "EDIT_CONFLICT",
            "Switch target changed; review again",
        ));
    }
    if target.3.launch_sha256() != request.confirmed_launch_sha256 {
        return Err(Error::new(
            "REVIEW_REQUIRED",
            "Target declaration does not match confirmation",
        ));
    }
    Ok(target)
}
pub fn preflight_switch(data: &Path, id: Uuid, request: &SwitchRequest) -> Result<()> {
    checked_switch(data, &registered(data, id)?, request).map(|_| ())
}
pub fn switch(
    data: &Path,
    id: Uuid,
    request: &SwitchRequest,
    actor: crate::journal::Actor,
) -> Result<serde_json::Value> {
    let _lock = lock(data)?;
    let mut registry = load(data)?;
    let index = registry
        .installations
        .iter()
        .position(|i| i.installation_id == id)
        .ok_or_else(|| Error::new("NOT_FOUND", "Unknown installation"))?;
    let (path, hash, text, manifest) =
        checked_switch(data, &registry.installations[index], request)?;
    upgrade(data, &mut registry)?;
    let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    if hash.is_none() {
        if request.target_source == ManifestSource::External {
            storage::private_directory(&data.join("manifests"))?;
        }
        storage::publish(&path, text.as_bytes(), false)?;
    }
    let i = &mut registry.installations[index];
    i.manifest_source = request.target_source;
    if hash.is_none() {
        match request.target_source {
            ManifestSource::External => i.external_manifest_sha256 = Some(digest.clone()),
            ManifestSource::Project => {
                i.created_manifest = Some(CreatedManifest {
                    sha256: digest.clone(),
                })
            }
        }
    }
    i.confirmed_launch_sha256 = Some(manifest.launch_sha256());
    i.confirmed_services = Some(manifest.services.clone());
    i.confirmed_manifest = (manifest.schema_version == 1).then_some(manifest);
    i.confirmed_root = Some(i.path.clone());
    i.confirmed_actor = Some(actor);
    i.confirmed_at_ms = Some(editing::now());
    if let Err(error) = save(data, &mut registry) {
        return Err(Error::new("MANIFEST_SAVED_UNCONFIRMED", "Target retained; source switch was not published")
            .details(json!({"manifest_path":path,"sha256":digest,"file_written":hash.is_none(),"cause":error})));
    }
    Ok(
        json!({"installation":registry.installations[index],"manifest_path":path,"sha256":digest,"file_written":hash.is_none(),"original_retained":true}),
    )
}
pub fn switch_coordinated(
    data: &Path,
    id: Uuid,
    request: SwitchRequest,
) -> Result<serde_json::Value> {
    let actor = crate::journal::Actor::from_environment()?;
    preflight_switch(data, id, &request)?;
    match crate::daemon::coordinate(
        data,
        id,
        "manifest-switch",
        json!({"installation_id":id,"switch":request}),
    )? {
        Some(result) => Ok(result),
        None => crate::journal::record_result(
            data,
            "manifest-switch",
            actor,
            Some(id),
            switch(data, id, &request, actor),
        ),
    }
}
