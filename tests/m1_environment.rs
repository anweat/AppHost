#![cfg(target_os = "macos")]
use apphost::{
    Manifest,
    environment::{self, Baseline, SYSTEM_PATH},
};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn manifest(command: &[&str]) -> Manifest {
    serde_json::from_value(serde_json::json!({
        "schema_version":1,"project_id":uuid::Uuid::new_v4(),"name":"fixture",
        "services":[{"name":"service","command":command}]
    }))
    .unwrap()
}

#[test]
fn shared_path_defaults_are_read_only_and_manual_updates_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    assert_eq!(
        environment::load(&data).unwrap().path,
        Baseline::system().unwrap().path
    );
    assert!(!data.exists());
    for path in [
        "", ".", "~/bin", "$PATH", "/bin:", ":/bin", "/a\n/bin", "/a\0/bin",
    ] {
        assert!(environment::save_path(&data, path.into()).is_err());
        assert!(!data.exists());
    }
    let path = "/a path that need not exist:/usr/bin:/bin";
    environment::save_path(&data, path.into()).unwrap();
    assert_eq!(environment::load(&data).unwrap().path, path);
    assert_eq!(
        fs::metadata(&data).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(data.join("environment.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let original = fs::read(data.join("environment.json")).unwrap();
    for path in [
        vec!["/bin"; 257].join(":"),
        format!("/{}", "a".repeat(16 * 1024)),
    ] {
        assert!(environment::save_path(&data, path).is_err());
        assert_eq!(fs::read(data.join("environment.json")).unwrap(), original);
    }
}

#[test]
fn final_environment_is_explicit_and_service_overrides_win() {
    let temp = TempDir::new().unwrap();
    let mut declaration = manifest(&["/usr/bin/env"]);
    declaration.services[0]
        .env
        .insert("APPHOST_FIXTURE".into(), "declared".into());
    declaration.services[0]
        .env
        .insert("PATH".into(), ":./bin:/usr/bin".into());
    let prepared =
        environment::prepare(&declaration, temp.path(), &Baseline::system().unwrap()).unwrap();
    let output = prepared
        .into_iter()
        .next()
        .unwrap()
        .into_command()
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let vars: std::collections::BTreeMap<_, _> = text
        .lines()
        .map(|line| line.split_once('=').unwrap())
        .collect();
    assert_eq!(vars.len(), 7); // Six controlled variables plus one explicit declaration.
    assert_eq!(vars["APPHOST_FIXTURE"], "declared");
    let root = fs::canonicalize(temp.path()).unwrap();
    assert_eq!(
        std::env::split_paths(vars["PATH"]).collect::<Vec<_>>(),
        vec![root.clone(), root.join("bin"), "/usr/bin".into()]
    );
    assert!(Path::new(vars["HOME"]).is_absolute());
    assert!(Path::new(vars["TMPDIR"]).is_absolute());
}

#[test]
fn malformed_settings_are_not_reset_and_update_lock_is_observed() {
    use fs2::FileExt;
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    environment::save_path(temp.path(), SYSTEM_PATH.into()).unwrap();
    let config = temp.path().join("environment.json");
    let original = fs::read(&config).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(temp.path().join("environment.lock"))
        .unwrap();
    FileExt::lock_exclusive(&lock).unwrap();
    assert_eq!(
        environment::save_path(temp.path(), "/bin".into())
            .unwrap_err()
            .code,
        "BUSY"
    );
    assert_eq!(environment::load(temp.path()).unwrap().path, SYSTEM_PATH);
    assert_eq!(fs::read(&config).unwrap(), original);
    FileExt::unlock(&lock).unwrap();
    for bad in [
        "not json",
        r#"{"schema_version":2,"path":"/bin"}"#,
        r#"{"schema_version":1,"path":"/bin","unknown":true}"#,
    ] {
        fs::write(&config, bad).unwrap();
        assert_eq!(
            environment::load(temp.path()).unwrap_err().code,
            "ENVIRONMENT_INVALID"
        );
        assert!(environment::save_path(temp.path(), "/bin".into()).is_err());
        assert_eq!(fs::read_to_string(&config).unwrap(), bad);
    }
    fs::remove_file(&config).unwrap();
    std::os::unix::fs::symlink(temp.path().join("missing"), &config).unwrap();
    assert!(environment::load(temp.path()).is_err());
    assert!(environment::save_path(temp.path(), "/bin".into()).is_err());
}

#[test]
fn explicit_paths_preserve_symlinks_and_arguments_without_shell_interpretation() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("bin")).unwrap();
    std::os::unix::fs::symlink("/usr/bin/printf", temp.path().join("bin/helper")).unwrap();
    let declaration = manifest(&[
        "bin/helper",
        "%s\n",
        "space argument",
        "$(touch should-not-exist)",
    ]);
    let services = environment::prepare(
        &declaration,
        temp.path(),
        &Baseline {
            schema_version: 1,
            path: "/does-not-exist".into(),
            capture: None,
        },
    )
    .unwrap();
    let prepared = services.into_iter().next().unwrap();
    assert_eq!(
        prepared.executable,
        fs::canonicalize(temp.path()).unwrap().join("bin/helper")
    );
    let output = prepared.into_command().output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"space argument\n$(touch should-not-exist)\n"
    );
    assert!(!temp.path().join("should-not-exist").exists());
    fs::write(temp.path().join("bin/not-executable"), b"fixture").unwrap();
    for executable in ["bin/not-executable", "bin", "nonexistent"] {
        let error = environment::prepare(
            &manifest(&[executable]),
            temp.path(),
            &Baseline::system().unwrap(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "INVALID_COMMAND");
        assert_eq!(error.details["service"], "service");
    }
    let mut outside = declaration;
    outside.services[0].cwd = "..".into();
    assert_eq!(
        environment::prepare(&outside, temp.path(), &Baseline::system().unwrap())
            .err()
            .unwrap()
            .code,
        "VALIDATION_ERROR"
    );
}

#[test]
fn cli_environment_check_uses_saved_path_and_never_executes_a_project() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let bin = project.join("bin");
    fs::create_dir(&bin).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let helper = bin.join("apphost-fixture-helper");
    fs::write(&helper, b"#!/bin/sh\ntouch should-not-run\n").unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let mut declaration = manifest(&["apphost-fixture-helper"]);
    declaration.services[0]
        .env
        .insert("SECRET".into(), "do-not-print-this-value".into());
    fs::write(project.join("apphost.toml"), declaration.to_toml().unwrap()).unwrap();
    fs::write(project.join(".env"), format!("PATH={}\n", bin.display())).unwrap();
    let cli = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_apphost"));
        command
            .arg("--data-dir")
            .arg(&data)
            .arg("environment")
            .env("PATH", &bin);
        command
    };
    let show = cli().arg("show").output().unwrap();
    assert!(show.status.success());
    assert!(!data.exists());
    let before = cli().arg("check").arg(&project).output().unwrap();
    assert!(!before.status.success());
    let error: serde_json::Value = serde_json::from_slice(&before.stderr).unwrap();
    assert_eq!(error["error"]["code"], "INVALID_COMMAND");
    assert!(
        cli()
            .arg("set-path")
            .arg(&bin)
            .output()
            .unwrap()
            .status
            .success()
    );
    let checked = cli().arg("check").arg(&project).output().unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let services: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(services["services"][0]["name"], "service");
    assert!(!String::from_utf8_lossy(&checked.stdout).contains("do-not-print-this-value"));
    assert!(!project.join("should-not-run").exists());
}

#[test]
fn existing_shared_directory_is_rejected_without_permission_or_file_changes() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("shared");
    fs::create_dir(&data).unwrap();
    fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(data.join("unrelated"), b"keep").unwrap();
    assert_eq!(
        environment::save_path(&data, SYSTEM_PATH.into())
            .unwrap_err()
            .code,
        "DATA_DIR_PERMISSIONS"
    );
    assert_eq!(
        apphost::detach(&data, uuid::Uuid::new_v4())
            .unwrap_err()
            .code,
        "DATA_DIR_PERMISSIONS"
    );
    assert_eq!(
        fs::metadata(&data).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read(data.join("unrelated")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&data).unwrap().count(), 1);
}
