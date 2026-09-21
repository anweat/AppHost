use crate::{
    Result,
    environment::{self, Baseline, PreparedService},
    registry::{self, LaunchSnapshot},
};
use serde::Serialize;
use std::path::Path;
use uuid::Uuid;

pub struct LaunchPlan {
    snapshot: LaunchSnapshot,
    baseline: Baseline,
    services: Vec<PreparedService>,
}

#[derive(Serialize)]
pub struct LaunchSummary<'a> {
    pub installation_id: Uuid,
    pub project_root: &'a Path,
    pub manifest_version: u32,
    pub launch_sha256: String,
    pub services: &'a [PreparedService],
}

impl LaunchPlan {
    pub fn prepare(data_dir: &Path, installation_id: Uuid) -> Result<Self> {
        let snapshot = registry::launch_snapshot(data_dir, installation_id)?;
        let baseline = environment::load(data_dir)?;
        let services = environment::prepare(
            snapshot.manifest(),
            &snapshot.installation().path,
            &baseline,
        )?;
        Ok(Self {
            snapshot,
            baseline,
            services,
        })
    }

    /// Called by the runtime only after queue, instance and port checks. This
    /// callback must synchronously spawn, never wait for readiness or client IO.
    pub fn with_current<T>(
        self,
        data_dir: &Path,
        action: impl FnOnce(Vec<PreparedService>) -> Result<T>,
    ) -> Result<T> {
        let Self {
            snapshot,
            baseline,
            services,
        } = self;
        snapshot.with_current(data_dir, || {
            let current = environment::prepare(
                snapshot.manifest(),
                &snapshot.installation().path,
                &baseline,
            )?;
            if current.iter().zip(&services).any(|(current, saved)| {
                current.cwd != saved.cwd || current.executable != saved.executable
            }) {
                return Err(crate::Error::new(
                    "STALE_LAUNCH",
                    "Service cwd or executable resolution changed after preparation",
                ));
            }
            action(services)
        })
    }

    pub fn summary(&self) -> LaunchSummary<'_> {
        LaunchSummary {
            installation_id: self.snapshot.installation().installation_id,
            project_root: &self.snapshot.installation().path,
            manifest_version: self.snapshot.manifest().schema_version,
            launch_sha256: self.snapshot.manifest().launch_sha256(),
            services: &self.services,
        }
    }
}
