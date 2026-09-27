use apphost::{Error, Result, runtime::State};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Serialize)]
pub struct App {
    installation_id: Uuid,
    project_id: Uuid,
    label: String,
    path: String,
    group: String,
    group_label: String,
    registration_state: String,
    requires_confirmation: bool,
    has_entry: bool,
    runtime: Option<Instance>,
}

// Deliberately project fields instead of forwarding list's confirmed env/commands
// or runtime launch_context to the webview. Details can request their own data later.
#[derive(Deserialize, Serialize)]
pub struct Instance {
    installation_id: Uuid,
    instance_id: Uuid,
    #[serde(default)]
    operation_id: Option<Uuid>,
    #[serde(default)]
    operation_done: bool,
    #[serde(default)]
    operation_error: Option<Failure>,
    status: Status,
    updated_at_ms: u64,
    #[serde(default)]
    created_at_ms: u64,
    #[serde(default)]
    started_at_ms: Option<u64>,
    #[serde(default)]
    management_uncertain: bool,
    services: Vec<Service>,
}
#[derive(Deserialize, Serialize)]
struct Failure {
    code: String,
    #[serde(default)]
    message: String,
}
#[derive(Deserialize, Serialize)]
struct Status {
    state: State,
    failure: Option<String>,
}
#[derive(Deserialize, Serialize)]
struct Service {
    name: String,
    port: Option<u16>,
    alive: bool,
    ready: bool,
    #[serde(default)]
    startup_timeout_ms: u64,
}
#[derive(Deserialize)]
struct Observation {
    live: bool,
    #[serde(default)]
    automatic_wake_paused: bool,
    instances: Vec<Instance>,
}
#[derive(Serialize)]
pub struct Snapshot {
    pub apps: Vec<App>,
    pub live: bool,
    pub automatic_wake_paused: bool,
    pub read_at_ms: u64,
    pub runtime_error: Option<Error>,
}

pub fn read(data: &Path) -> Result<Snapshot> {
    let listings = apphost::list(data, false)?;
    let (observation, runtime_error) = match apphost::daemon::status(data, None) {
        Ok(value) => (Some(value), None),
        Err(e) => {
            // Unsupported platforms still display registration metadata.
            // A transport/authentication error never becomes a fresh STOPPED result.
            (None, Some(e))
        }
    };
    let (observation, runtime_error) = match observation {
        Some(value) => match serde_json::from_value::<Observation>(value) {
            Ok(value) => (Some(value), runtime_error),
            Err(_) => (
                None,
                Some(Error::new("PROTOCOL_ERROR", "Invalid runtime snapshot")),
            ),
        },
        None => (None, runtime_error),
    };
    let automatic_wake_paused = observation
        .as_ref()
        .is_some_and(|o| o.automatic_wake_paused);
    let live = observation.as_ref().is_some_and(|o| o.live);
    let mut instances: std::collections::HashMap<_, _> = observation
        .into_iter()
        .flat_map(|o| o.instances)
        .map(|r| (r.installation_id, r))
        .collect();
    let apps = listings
        .into_iter()
        .map(|item| App {
            installation_id: item.installation.installation_id,
            project_id: item.installation.project_id,
            label: item.display_label,
            group: item
                .installation
                .path
                .parent()
                .unwrap_or(Path::new(""))
                .to_string_lossy()
                .into_owned(),
            group_label: item
                .installation
                .path
                .parent()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "应用".into()),
            path: item.installation.path.to_string_lossy().into_owned(),
            registration_state: item.registration_state.into(),
            requires_confirmation: item.requires_launch_confirmation,
            has_entry: item
                .manifest
                .as_ref()
                .and_then(|m| m.open_target().ok().flatten())
                .is_some(),
            runtime: instances.remove(&item.installation.installation_id),
        })
        .collect();
    Ok(Snapshot {
        apps,
        live,
        automatic_wake_paused,
        runtime_error,
        read_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_read_creates_no_data_or_daemon() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("absent");
        let snapshot = read(&data).unwrap();
        assert!(snapshot.apps.is_empty());
        assert!(!snapshot.live);
        assert!(!data.exists());
    }
    #[test]
    fn registered_apps_survive_bad_runtime_without_exposing_confirmed_env() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let manifest: apphost::Manifest = serde_json::from_value(serde_json::json!({
            "schema_version":1,"project_id":Uuid::new_v4(),"name":"Example",
            "services":[{"name":"web","command":["unused"],"env":{"TOKEN":"secret-must-not-cross"}}]
        }))
        .unwrap();
        std::fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id =
            apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        std::fs::create_dir(data.join("runtime")).unwrap();
        std::fs::write(data.join("runtime").join(format!("{id}.json")), "{}").unwrap();
        let snapshot = read(&data).unwrap();
        assert_eq!(snapshot.apps.len(), 1);
        assert_eq!(snapshot.apps[0].installation_id, id);
        assert!(snapshot.apps[0].runtime.is_none());
        assert!(snapshot.runtime_error.is_some());
        assert!(!snapshot.live);
        let text = serde_json::to_string(&snapshot).unwrap();
        assert!(!text.contains("secret-must-not-cross"));
        assert!(!text.contains("confirmed_services"));
    }
    #[test]
    fn runtime_projection_drops_launch_secrets() {
        let observation: Observation = serde_json::from_value(serde_json::json!({
            "live":false, "instances":[{
                "installation_id":Uuid::nil(),"instance_id":Uuid::nil(),"updated_at_ms":42,"started_at_ms":20,"operation_error":{"code":"STOP_FAILED"},
                "status":{"state":"FAILED","failure":"SERVICE_EXITED"},
                "services":[{"name":"web","port":3000,"alive":true,"ready":false}],
                "launch_context":{"env":{"TOKEN":"secret-must-not-cross"}}
            }]
        }))
        .unwrap();
        let value = serde_json::to_string(&observation.instances).unwrap();
        assert!(!value.contains("secret-must-not-cross"));
        assert!(value.contains("FAILED"));
        assert!(value.contains("\"started_at_ms\":20"));
        assert!(value.contains("STOP_FAILED"));
    }
}
