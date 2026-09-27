//! Settings share CLI policy and auditing; neither reads nor writes start services.
use apphost::{Error, Result, journal::Actor};
use serde_json::{Value, json};
use std::path::Path;
use uuid::Uuid;

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    Zsh,
    Bash,
}
impl Shell {
    fn path(&self) -> &'static Path {
        Path::new(match self {
            Self::Zsh => "/bin/zsh",
            Self::Bash => "/bin/bash",
        })
    }
}
async fn blocking<T: Send + 'static>(
    task: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|e| Error::new("DESKTOP_SETTINGS_FAILED", e.to_string()))?
}
fn audited(data: &Path, action: &str, result: Result<Value>) -> Result<Value> {
    apphost::journal::record_result(data, action, Actor::Human, None, result)
}
#[tauri::command]
pub async fn desktop_environment_read() -> Result<apphost::environment::Baseline> {
    blocking(|| apphost::environment::load(&apphost::paths::data_dir(None)?)).await
}
#[tauri::command]
pub async fn desktop_environment_save(path: String) -> Result<Value> {
    blocking(move || {
        let data = apphost::paths::data_dir(None)?;
        audited(
            &data,
            "environment-set-path",
            apphost::environment::save_path(&data, path).map(|v| json!(v)),
        )
    })
    .await
}
#[tauri::command]
pub async fn desktop_environment_capture(shell: Shell) -> Result<Value> {
    blocking(move || {
        let data = apphost::paths::data_dir(None)?;
        // Only the explicit capture button reaches this execution entry point.
        // Core owns the 5 s / 64 KiB bounds, group cleanup and concurrent-save guard.
        audited(
            &data,
            "environment-capture",
            apphost::environment::capture_path(&data, shell.path()).map(|v| json!(v)),
        )
    })
    .await
}
fn check(data: &Path, id: Uuid) -> Result<Value> {
    let plan = apphost::runtime::LaunchPlan::prepare(data, id)?;
    Ok(json!(plan.summary()))
}
#[tauri::command]
pub async fn desktop_environment_check(installation_id: Uuid) -> Result<Value> {
    blocking(move || check(&apphost::paths::data_dir(None)?, installation_id)).await
}
#[tauri::command]
pub async fn desktop_retention_read() -> Result<apphost::log_retention::Settings> {
    blocking(|| apphost::log_retention::load(&apphost::paths::data_dir(None)?)).await
}
fn save_retention(data: &Path, days: u16, max_mib: u32) -> Result<Value> {
    audited(
        data,
        "log-retention-set",
        apphost::log_retention::save(data, days, max_mib).map(|v| json!(v)),
    )
}
#[tauri::command]
pub async fn desktop_retention_save(days: u16, max_mib: u32) -> Result<Value> {
    blocking(move || save_retention(&apphost::paths::data_dir(None)?, days, max_mib)).await
}

#[tauri::command]
pub async fn desktop_login_read() -> Result<Value> {
    blocking(|| apphost::login::show(&apphost::paths::data_dir(None)?)).await
}
#[tauri::command]
pub async fn desktop_login_save(enabled: bool) -> Result<Value> {
    blocking(move || {
        apphost::login::set_executable(
            &apphost::paths::data_dir(None)?,
            &crate::runtime::cli_path()?,
            enabled,
            Actor::Human,
        )
    })
    .await
}
#[tauri::command]
pub async fn desktop_gateway_read() -> Result<apphost::gateway::Config> {
    blocking(|| apphost::gateway::config(&apphost::paths::data_dir(None)?)).await
}
#[tauri::command]
pub async fn desktop_gateway_save(port: u16) -> Result<Value> {
    blocking(move || apphost::gateway::set_port(&apphost::paths::data_dir(None)?, port)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_save_preserves_logs_and_invalid_updates_preserve_settings() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        assert_eq!(apphost::log_retention::load(&data).unwrap().days, 7);
        assert!(!data.exists());
        save_retention(&data, 14, 200).unwrap();
        let archive = data.join("instances");
        std::fs::create_dir(&archive).unwrap();
        std::fs::write(archive.join("keep"), b"archive").unwrap();
        assert_eq!(
            save_retention(&data, 0, 100).unwrap_err().code,
            "INVALID_LOG_RETENTION"
        );
        assert_eq!(apphost::log_retention::load(&data).unwrap().days, 14);
        save_retention(&data, 1, 10).unwrap();
        assert_eq!(std::fs::read(archive.join("keep")).unwrap(), b"archive");
        assert!(!data.join("runtime").exists());
    }
    #[test]
    fn capture_shell_is_a_fixed_choice_not_a_program_or_script() {
        assert_eq!(
            serde_json::from_str::<Shell>("\"zsh\"").unwrap().path(),
            Path::new("/bin/zsh")
        );
        assert!(serde_json::from_str::<Shell>("\"/tmp/script\"").is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn environment_check_is_read_only_and_does_not_expose_service_environment() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let manifest: apphost::Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"check","services":[{"name":"worker","command":["/usr/bin/true"],"env":{"SECRET":"test-private-value"}}]})).unwrap();
        std::fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id =
            apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        let value = check(&data, id).unwrap();
        assert_eq!(value["services"][0]["executable"], "/usr/bin/true");
        assert!(!value.to_string().contains("test-private-value"));
        assert!(!data.join("runtime").exists());
        assert!(!data.join("environment.json").exists());
    }
}
