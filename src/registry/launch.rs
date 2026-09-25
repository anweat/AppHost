use super::{Installation, Registry, load};
use crate::{Error, Manifest, Result, storage};
use std::path::{Path, PathBuf};
use uuid::Uuid;

// Private fields prevent substituting a different declaration after verification.
// This is an in-process value, never a serialized authorization token.
pub struct LaunchSnapshot {
    data_dir: PathBuf,
    pub(super) registry_version: u32,
    pub(super) installation: Installation,
    pub(super) manifest: Manifest,
}

impl LaunchSnapshot {
    /// The callback is a short synchronous dispatch point, not a readiness wait.
    /// Keep registry writers excluded through spawn, then release the guard.
    pub fn with_current<T>(
        &self,
        data_dir: &Path,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        if storage::directory(data_dir)? != self.data_dir {
            return Err(Error::new(
                "CONFLICT",
                "Launch snapshot belongs to a different data directory",
            ));
        }
        // Runtime dispatch tolerates a short external writer, then revalidates
        // the snapshot under the acquired lock. Other registry commands stay nonblocking.
        let _lock = super::lock_wait(data_dir, std::time::Duration::from_secs(2))?;
        let current = from_registry(load(data_dir)?, self.installation.installation_id, data_dir)?;
        if current.registry_version != self.registry_version
            || current.installation.revision != self.installation.revision
            || current.installation.path != self.installation.path
            || current.manifest.launch_sha256() != self.manifest.launch_sha256()
        {
            return Err(Error::new(
                "STALE_LAUNCH",
                "Installation changed after preparation; prepare the current declaration again",
            ));
        }
        action()
    }

    pub fn installation(&self) -> &Installation {
        &self.installation
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn registry_version(&self) -> u32 {
        self.registry_version
    }
}

pub fn launch_snapshot(data_dir: &Path, installation_id: Uuid) -> Result<LaunchSnapshot> {
    let registry = load(data_dir)?;
    from_registry(registry, installation_id, data_dir)
}

fn from_registry(
    registry: Registry,
    installation_id: Uuid,
    data_dir: &Path,
) -> Result<LaunchSnapshot> {
    let installation = registry
        .installations
        .into_iter()
        .find(|installation| installation.installation_id == installation_id)
        .ok_or_else(|| Error::new("NOT_FOUND", "Installation is not registered"))?;
    if installation.detached {
        return Err(Error::new(
            "DETACHED",
            "Installation was detached; register and confirm it before launch",
        ));
    }
    let root = storage::directory(&installation.path).map_err(|error| {
        if error.code == "NOT_FOUND" || !installation.path.is_dir() {
            Error::new(
                "SOURCE_MISSING",
                "Registered project directory is unavailable",
            )
        } else {
            error
        }
    })?;
    if root != installation.path {
        return Err(Error::new(
            "REVIEW_REQUIRED",
            "Registered project root resolves to a different directory",
        ));
    }
    let manifest = super::sources::resolve(data_dir, &installation)?;
    if manifest.project_id != installation.project_id {
        return Err(Error::new(
            "CONFLICT",
            "Project identity differs from registration",
        ));
    }
    if !installation.confirmation_matches(&manifest) {
        return Err(Error::new(
            "REVIEW_REQUIRED",
            "Review and import the current launch declaration before launch",
        ));
    }
    Ok(LaunchSnapshot {
        data_dir: storage::directory(data_dir)?,
        registry_version: registry.schema_version,
        installation,
        manifest,
    })
}
