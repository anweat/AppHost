use crate::{
    manifest::{Issue, Manifest},
    storage,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn found(command: &str, cwd: &Path, path: Option<std::ffi::OsString>) -> bool {
    let name = Path::new(command);
    let options: Vec<PathBuf> = if name.is_absolute() || name.components().count() > 1 {
        vec![cwd.join(name)]
    } else {
        path.map(|p| {
            env::split_paths(&p)
                .map(|dir| cwd.join(dir).join(name))
                .collect()
        })
        .unwrap_or_default()
    };
    options.into_iter().any(|p| {
        if executable(&p) {
            return true;
        }
        #[cfg(windows)]
        if p.extension().is_none() {
            return ["exe", "com", "cmd", "bat"]
                .iter()
                .any(|ext| executable(&p.with_extension(ext)));
        }
        false
    })
}

impl Manifest {
    /// Static hints describe this machine now, never proof that a service will start.
    /// Do not execute --version, a shell, npm, or any project-supplied program here.
    pub fn launch_warnings(&self, root: &Path) -> Vec<Issue> {
        let mut remaining = u64::MAX;
        self.launch_warnings_budgeted(root, &mut remaining)
            .unwrap_or_else(|e| vec![Issue::new("launch_check", e.to_string())])
    }

    pub(crate) fn launch_warnings_budgeted(
        &self,
        root: &Path,
        remaining: &mut u64,
    ) -> crate::Result<Vec<Issue>> {
        let mut warnings = Vec::new();
        for (index, service) in self.services.iter().enumerate() {
            if crate::manifest::relative_directory(root, &service.cwd).is_err() {
                continue;
            }
            let cwd = root.join(&service.cwd);
            let Some(command) = service.command.first() else {
                continue;
            };
            let field = format!("services[{index}].command");
            let path = service
                .env
                .get("PATH")
                .map(std::ffi::OsString::from)
                .or_else(|| env::var_os("PATH"));
            if !found(command, &cwd, path) {
                warnings.push(Issue::new(&field, format!("Executable {command:?} was not found with execute permission in this service's cwd/PATH")));
            }
            // M1: finding npm.cmd/.bat on Windows is not proof that direct spawn works.
            // The launcher must explicitly handle the shell and its argument quoting.
            // Only an unambiguous direct run form is checked; wrappers and manager-specific
            // workspace/--prefix resolution belong to their runtime, not a second shell parser.
            if ["npm", "pnpm", "yarn", "bun"].contains(&command.as_str())
                && service.command.get(1).is_some_and(|s| s == "run")
                && let Some(script) = service.command.get(2)
                && !script.starts_with('-')
            {
                let bytes = storage::read_budgeted(&cwd.join("package.json"), remaining);
                if bytes.as_ref().is_err_and(|e| e.code == "SCAN_LIMIT") {
                    return Err(bytes.unwrap_err());
                }
                let result = bytes.map_err(|e| e.to_string()).and_then(|bytes| {
                    serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| e.to_string())
                });
                match result {
                    Ok(value)
                        if value
                            .get("scripts")
                            .and_then(|s| s.get(script))
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| !s.trim().is_empty()) => {}
                    Ok(_) => warnings.push(Issue::new(
                        &field,
                        format!(
                            "Script {script:?} is absent or empty in the service's package.json"
                        ),
                    )),
                    Err(error) => warnings.push(Issue::new(
                        &field,
                        format!("Cannot inspect package.json: {error}"),
                    )),
                }
            }
        }
        Ok(warnings)
    }
}
