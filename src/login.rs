//! Explicit next-login registration. Setting this never starts or stops an App.
use crate::{Error, Result};
use serde_json::Value;
use std::path::Path;

pub fn show(data: &Path) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        macos::show(data, &macos::home()?)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data;
        Err(Error::new(
            "UNSUPPORTED_PLATFORM",
            "Login registration requires macOS",
        ))
    }
}
pub fn set(data: &Path, enabled: bool) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        let actor = crate::journal::Actor::from_environment()?;
        let result = macos::set(
            data,
            &macos::home()?,
            &std::env::current_exe().map_err(|e| Error::new("IO_ERROR", e.to_string()))?,
            enabled,
        );
        crate::journal::record_result(
            data,
            if enabled {
                "login-enable"
            } else {
                "login-disable"
            },
            actor,
            None,
            result,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (data, enabled);
        Err(Error::new(
            "UNSUPPORTED_PLATFORM",
            "Login registration requires macOS",
        ))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::storage;
    use fs2::FileExt;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{fs, os::unix::fs::OpenOptionsExt, path::PathBuf};
    const MARKER: &str = "<!-- AppHost login configuration v1 -->";
    pub(super) fn home() -> Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| Error::new("INVALID_INPUT", "HOME is unavailable"))?;
        storage::directory(Path::new(&home))
    }
    fn target(data: &Path, home: &Path) -> Result<(String, PathBuf)> {
        let data = storage::directory(data)?;
        let label = format!(
            "io.apphost.daemon.{:x}",
            Sha256::digest(data.as_os_str().as_encoded_bytes())
        );
        Ok((
            label.clone(),
            home.join("Library/LaunchAgents")
                .join(format!("{label}.plist")),
        ))
    }
    fn existing(path: &Path) -> Result<bool> {
        match storage::read(path) {
            Ok(bytes) if std::str::from_utf8(&bytes).is_ok_and(|s| s.contains(MARKER)) => Ok(true),
            Ok(_) => Err(Error::new(
                "LOGIN_CONFIG_CONFLICT",
                "Existing login file is not an AppHost configuration",
            )),
            Err(e) if e.code == "NOT_FOUND" => Ok(false),
            Err(e) => Err(e),
        }
    }
    pub(super) fn show(data: &Path, home: &Path) -> Result<Value> {
        if !data.try_exists().map_err(|e| storage::io_error(data, e))? {
            return Ok(json!({"enabled":false,"activation":"next_login"}));
        }
        let (label, path) = target(data, home)?;
        Ok(json!({"enabled":existing(&path)?,"label":label,"path":path,"activation":"next_login"}))
    }
    fn escape(value: &str) -> Result<String> {
        if value.chars().any(char::is_control) {
            return Err(Error::new(
                "INVALID_INPUT",
                "Login paths cannot contain control characters",
            ));
        }
        Ok(value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;"))
    }
    fn plist(label: &str, data: &Path, executable: &Path) -> Result<String> {
        let text = |p: &Path| {
            p.to_str()
                .ok_or_else(|| Error::new("INVALID_INPUT", "Login paths must be UTF-8"))
                .and_then(escape)
        };
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{MARKER}\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--data-dir</string><string>{}</string><string>daemon</string><string>serve</string></array>\n<key>RunAtLoad</key><true/>\n<key>ProcessType</key><string>Background</string>\n</dict></plist>\n",
            escape(label)?,
            text(executable)?,
            text(data)?
        ))
    }
    pub(super) fn set(data: &Path, home: &Path, executable: &Path, enabled: bool) -> Result<Value> {
        storage::private_directory(data)?;
        let data = storage::directory(data)?;
        let lock_path = data.join("login.lock");
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&lock_path)
            .map_err(|e| storage::io_error(&lock_path, e))?;
        if !lock
            .metadata()
            .map_err(|e| storage::io_error(&lock_path, e))?
            .is_file()
        {
            return Err(Error::new("LOGIN_CONFIG_CONFLICT", "Invalid login lock"));
        }
        lock.try_lock_exclusive()
            .map_err(|_| Error::new("BUSY", "Login settings are being changed"))?;
        let (label, path) = target(&data, home)?;
        let exists = existing(&path)?;
        if enabled {
            let executable =
                fs::canonicalize(executable).map_err(|e| storage::io_error(executable, e))?;
            if !executable.is_file() {
                return Err(Error::new(
                    "INVALID_INPUT",
                    "Login executable must be a file",
                ));
            }
            let text = plist(&label, &data, &executable)?;
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| storage::io_error(&path, e))?;
            storage::publish(&path, text.as_bytes(), exists)?;
        } else if exists {
            fs::remove_file(&path).map_err(|e| storage::io_error(&path, e))?;
            fs::File::open(path.parent().unwrap())
                .and_then(|f| f.sync_all())
                .map_err(|e| storage::io_error(&path, e))?;
        }
        // No bootstrap/bootout: changes apply at the next login. In particular,
        // disabling the setting must leave a currently running daemon and Apps alone.
        show(&data, home)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn opt_in_is_scoped_read_only_and_preserves_unrelated_agents() {
            let t = tempfile::tempdir().unwrap();
            let data = t.path().join("data & one");
            let home = t.path().join("home");
            assert_eq!(show(&data, &home).unwrap()["enabled"], false);
            assert!(!data.exists());
            assert!(!home.exists());
            let exe = std::env::current_exe().unwrap();
            let value = set(&data, &home, &exe, true).unwrap();
            let path = PathBuf::from(value["path"].as_str().unwrap());
            let output = std::process::Command::new("/usr/bin/plutil")
                .args(["-convert", "json", "-o", "-", "--"])
                .arg(&path)
                .output()
                .unwrap();
            assert!(output.status.success());
            let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                parsed["ProgramArguments"][2],
                json!(storage::directory(&data).unwrap())
            );
            assert_eq!(parsed["ProgramArguments"][3], "daemon");
            assert_eq!(parsed["RunAtLoad"], true);
            assert!(parsed["KeepAlive"].is_null());
            let other = set(&t.path().join("other"), &home, &exe, true).unwrap();
            assert_ne!(other["label"], value["label"]);
            set(&data, &home, &exe, false).unwrap();
            assert!(!path.exists());
            assert!(Path::new(other["path"].as_str().unwrap()).exists());
            fs::write(&path, b"unrelated user file").unwrap();
            assert_eq!(
                set(&data, &home, &exe, false).unwrap_err().code,
                "LOGIN_CONFIG_CONFLICT"
            );
            assert_eq!(fs::read(&path).unwrap(), b"unrelated user file");
        }
    }
}
