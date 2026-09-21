//! Shared runtime PATH. Never infer it from the invoking terminal or scan a shell profile.
use crate::{Error, Result, storage};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "macos")]
mod capture;
#[cfg(target_os = "macos")]
mod macos;

pub const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
const FILE_NAME: &str = "environment.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub schema_version: u32,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRecord {
    pub shell: PathBuf,
    pub captured_at_ms: u64,
}

fn supported() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(Error::new(
            "UNSUPPORTED_PLATFORM",
            "Runtime environment support requires macOS",
        ));
    }
    Ok(())
}

// Read ordered system path declarations only; never execute path_helper or profiles.
fn system_path(paths: &Path, paths_d: &Path, homebrew: &Path) -> Result<String> {
    let mut entries = Vec::<String>::new();
    let mut files = vec![paths.to_owned()];
    match fs::read_dir(paths_d) {
        Ok(directory) => {
            let mut extra = directory
                .take(257)
                .map(|entry| entry.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| storage::io_error(paths_d, e))?;
            extra.sort();
            files.extend(extra);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(storage::io_error(paths_d, error)),
    }
    if files.len() > 257 {
        return Err(Error::new(
            "ENVIRONMENT_INVALID",
            "Too many system PATH files",
        ));
    }
    let mut remaining = 64 * 1024;
    for file in files {
        let bytes = match storage::read_budgeted(&file, &mut remaining) {
            Ok(bytes) => bytes,
            Err(error) if error.code == "NOT_FOUND" => continue,
            Err(error) => return Err(error),
        };
        let content = std::str::from_utf8(&bytes)
            .map_err(|_| Error::new("ENVIRONMENT_INVALID", "System PATH file must be UTF-8"))?;
        for entry in content.lines().filter(|entry| !entry.is_empty()) {
            if entry.contains(':')
                || entry.chars().any(char::is_control)
                || !Path::new(entry).is_absolute()
            {
                return Err(Error::new(
                    "ENVIRONMENT_INVALID",
                    "System PATH file requires one absolute directory per line",
                ));
            }
            if !entries.iter().any(|previous| previous == entry) {
                entries.push(entry.into());
            }
        }
    }
    // Keep essential system commands available when optional system files are absent.
    for entry in SYSTEM_PATH.split(':') {
        if !entries.iter().any(|previous| previous == entry) {
            entries.push(entry.into());
        }
    }
    if homebrew.is_dir() {
        let entry = homebrew
            .to_str()
            .ok_or_else(|| Error::new("ENVIRONMENT_INVALID", "Invalid Homebrew path"))?;
        if !entries.iter().any(|previous| previous == entry) {
            entries.push(entry.into());
        }
    }
    Ok(entries.join(":"))
}

impl Baseline {
    pub fn system() -> Result<Self> {
        supported()?;
        let baseline = Self {
            schema_version: 1,
            path: system_path(
                Path::new("/etc/paths"),
                Path::new("/etc/paths.d"),
                Path::new("/opt/homebrew/bin"),
            )?,
            capture: None,
        };
        baseline.validate()?;
        Ok(baseline)
    }

    pub fn validate(&self) -> Result<()> {
        supported()?;
        if self.schema_version != 1
            || self.capture.as_ref().is_some_and(|record| {
                !record.shell.is_absolute()
                    || record
                        .shell
                        .to_str()
                        .is_none_or(|path| path.len() > 4096 || path.chars().any(char::is_control))
                    || record.captured_at_ms == 0
            })
            || self.path.is_empty()
            || self.path.len() > 16 * 1024
            || self.path.chars().any(char::is_control)
            || self.path.split(':').count() > 256
            || self
                .path
                .split(':')
                .any(|entry| !Path::new(entry).is_absolute())
        {
            return Err(Error::new(
                "ENVIRONMENT_INVALID",
                "Expected version 1 and a nonempty absolute PATH (at most 256 entries / 16 KiB)",
            ));
        }
        Ok(())
    }
}

pub fn load(data_dir: &Path) -> Result<Baseline> {
    supported()?;
    match storage::read(&data_dir.join(FILE_NAME)) {
        Ok(bytes) => {
            let baseline: Baseline = serde_json::from_slice(&bytes)
                .map_err(|e| Error::new("ENVIRONMENT_INVALID", e.to_string()))?;
            baseline.validate()?;
            Ok(baseline)
        }
        Err(error) if error.code == "NOT_FOUND" => Baseline::system(),
        Err(error) => Err(error),
    }
}

struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

pub fn save_path(data_dir: &Path, path: String) -> Result<Baseline> {
    let baseline = Baseline {
        schema_version: 1,
        path,
        capture: None,
    };
    save(data_dir, baseline, None)
}

// Some(None) means the caller observed no file; None skips optimistic comparison
// for an explicit manual set. The long shell operation never holds this lock.
fn save(data_dir: &Path, baseline: Baseline, expected: Option<Option<String>>) -> Result<Baseline> {
    baseline.validate()?;
    storage::private_directory(data_dir)?;
    let lock_path = data_dir.join("environment.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| storage::io_error(&lock_path, e))?;
    FileExt::try_lock_exclusive(&file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            Error::new("BUSY", "Another environment update is running")
        } else {
            storage::io_error(&lock_path, e)
        }
    })?;
    let _lock = Lock(file);
    // Do not overwrite an unknown/newer format or hide corrupted user settings.
    load(data_dir)?;
    if let Some(expected) = expected
        && storage::fingerprint(&data_dir.join(FILE_NAME))? != expected
    {
        return Err(Error::new(
            "CONFLICT",
            "Environment changed during capture; current settings were preserved",
        ));
    }
    let bytes = serde_json::to_vec_pretty(&baseline)
        .map_err(|e| Error::new("ENVIRONMENT_INVALID", e.to_string()))?;
    storage::publish(&data_dir.join(FILE_NAME), &bytes, true)?;
    Ok(baseline)
}

pub fn capture_path(data_dir: &Path, shell: &Path) -> Result<Baseline> {
    supported()?;
    #[cfg(target_os = "macos")]
    {
        capture::capture_path(data_dir, shell)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data_dir, shell);
        Err(Error::new(
            "UNSUPPORTED_PLATFORM",
            "PATH capture requires macOS",
        ))
    }
}

#[derive(Serialize)]
pub struct PreparedService {
    pub name: String,
    pub cwd: PathBuf,
    pub executable: PathBuf,
    #[serde(skip_serializing)]
    command: Command,
}

impl PreparedService {
    /// The runtime must still recheck registration and confirmation before spawn.
    pub fn into_command(self) -> Command {
        self.command
    }
}

pub fn prepare(
    manifest: &crate::Manifest,
    root: &Path,
    baseline: &Baseline,
) -> Result<Vec<PreparedService>> {
    baseline.validate()?;
    let root = storage::directory(root)?;
    let issues = manifest.validate(&root);
    if !issues.is_empty() {
        return Err(
            Error::new("VALIDATION_ERROR", "Invalid manifest").details(serde_json::json!(issues))
        );
    }
    let mut base = user_environment()?;
    base.insert("PATH".into(), baseline.path.clone());
    let mut prepared = Vec::new();
    for service in &manifest.services {
        let cwd = storage::directory(&root.join(&service.cwd))?;
        if !cwd.starts_with(&root) {
            return Err(Error::new(
                "VALIDATION_ERROR",
                "cwd changed outside project",
            ));
        }
        let mut environment = base.clone();
        environment.extend(service.env.clone());
        // M0 permits relative/empty PATH components. Anchor them once to this
        // service, so resolver and launched grandchildren see the same PATH.
        let path = std::env::join_paths(
            std::env::split_paths(&environment["PATH"]).map(|entry| cwd.join(entry)),
        )
        .map_err(|_| Error::new("ENVIRONMENT_INVALID", "Cannot represent service PATH"))?;
        let path = path
            .into_string()
            .map_err(|_| Error::new("ENVIRONMENT_INVALID", "PATH must be UTF-8"))?;
        let executable = resolve(&service.command[0], &cwd, &path)
            .map_err(|error| error.details(serde_json::json!({"service": service.name})))?;
        environment.insert("PATH".into(), path);
        let mut command = Command::new(&executable);
        command
            .args(&service.command[1..])
            .current_dir(&cwd)
            .env_clear()
            .envs(environment);
        prepared.push(PreparedService {
            name: service.name.clone(),
            cwd,
            executable,
            command,
        });
    }
    Ok(prepared)
}

fn resolve(name: &str, cwd: &Path, path: &str) -> Result<PathBuf> {
    let candidates: Vec<PathBuf> = if name.contains('/') {
        vec![cwd.join(name)]
    } else {
        std::env::split_paths(path)
            .map(|entry| entry.join(name))
            .collect()
    };
    for candidate in candidates {
        if !fs::metadata(&candidate).is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        #[cfg(target_os = "macos")]
        if macos::can_execute(&candidate) {
            return Ok(candidate);
        }
    }
    Err(Error::new(
        "INVALID_COMMAND",
        format!(
            "Executable {name:?} is unavailable in the configured service cwd/PATH; configure PATH with apphost environment set-path or explicitly run apphost environment capture --shell /bin/zsh --yes"
        ),
    ))
}

fn user_environment() -> Result<BTreeMap<String, String>> {
    #[cfg(target_os = "macos")]
    {
        macos::user_environment()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(Error::new(
            "UNSUPPORTED_PLATFORM",
            "Runtime environment support requires macOS",
        ))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn ordered_system_path_files_are_bounded_and_homebrew_is_explicit() {
        let temp = tempfile::tempdir().unwrap();
        let paths = temp.path().join("paths");
        let extra = temp.path().join("paths.d");
        let brew = temp.path().join("homebrew/bin");
        fs::create_dir(&extra).unwrap();
        fs::write(&paths, "/usr/local/bin\n/usr/bin\n/bin\n/usr/sbin\n/sbin\n").unwrap();
        fs::write(extra.join("20-second"), "/second\n/usr/bin\n").unwrap();
        fs::write(extra.join("10-first"), "/first\n").unwrap();
        let expected = "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/first:/second";
        assert_eq!(system_path(&paths, &extra, &brew).unwrap(), expected);
        fs::create_dir_all(&brew).unwrap();
        assert_eq!(
            system_path(&paths, &extra, &brew).unwrap(),
            format!("{expected}:{}", brew.display())
        );
        fs::write(&paths, "relative\n").unwrap();
        assert_eq!(
            system_path(&paths, &extra, &brew).unwrap_err().code,
            "ENVIRONMENT_INVALID"
        );
        fs::write(&paths, vec![b'x'; 65537]).unwrap();
        assert!(system_path(&paths, &extra, &brew).is_err());
        fs::remove_file(&paths).unwrap();
        fs::remove_dir_all(&extra).unwrap();
        fs::remove_dir_all(temp.path().join("homebrew")).unwrap();
        assert_eq!(system_path(&paths, &extra, &brew).unwrap(), SYSTEM_PATH);
    }
}
