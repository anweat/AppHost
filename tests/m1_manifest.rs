use apphost::{Manifest, manifest::Open};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn declaration(version: u32) -> Manifest {
    toml::from_str(&format!(
        r#"
schema_version = {version}
project_id = "ef42ed4a-b42b-45a6-92fa-069a0e997bd1"
name = "Demo"
[[services]]
name = "web"
command = ["apphost-test-missing-program"]
port = 5173
"#
    ))
    .unwrap()
}

fn project(parent: &Path, name: &str, version: u32) -> std::path::PathBuf {
    let path = parent.join(name);
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join("apphost.toml"),
        declaration(version).to_toml().unwrap(),
    )
    .unwrap();
    path
}

// Genuine historical wire input: current imports now write Registry v1 even
// when the project's manifest is v0. Never use a new import as a v0 fixture.
fn legacy_registry(data: &Path) {
    let path = data.join("registry.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["schema_version"] = 0.into();
    for installation in value["installations"].as_array_mut().unwrap() {
        for key in [
            "revision",
            "confirmed_root",
            "confirmed_manifest",
            "confirmed_actor",
            "confirmed_at_ms",
        ] {
            installation.as_object_mut().unwrap().remove(key);
        }
    }
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn legacy_label(data: &Path, label: &str) {
    let path = data.join("registry.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["installations"][0]["human_label"] = label.into();
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
fn legacy_digest_is_a_fixed_wire_contract() {
    let manifest = declaration(0);
    // This is the M0 wire representation, not serialization of today's Service type.
    let old = r#"["apphost.launch.v1",0,"ef42ed4a-b42b-45a6-92fa-069a0e997bd1",[{"name":"web","cwd":".","command":["apphost-test-missing-program"],"port":5173}]]"#;
    assert_eq!(
        manifest.launch_sha256(),
        format!("{:x}", Sha256::digest(old))
    );
    let json = serde_json::to_value(manifest).unwrap();
    assert!(json.get("open").is_none());
    assert!(json["services"][0].get("startup_timeout_ms").is_none());
}

#[test]
fn v1_defaults_are_semantic_but_execution_changes_revoke_confirmation() {
    let manifest = declaration(1);
    let hash = manifest.launch_sha256();
    let mut explicit = manifest.clone();
    explicit.services[0].startup_timeout_ms = Some(30_000);
    explicit.services[0].stop_timeout_ms = Some(5_000);
    assert_eq!(hash, explicit.launch_sha256());
    explicit.name = "new label".into();
    explicit.description = "new description".into();
    assert_eq!(hash, explicit.launch_sha256());
    explicit.services[0].startup_timeout_ms = Some(31_000);
    assert_ne!(hash, explicit.launch_sha256());
    explicit = manifest.clone();
    explicit.open = Some(Open {
        service: "web".into(),
        path: "/app".into(),
    });
    assert_ne!(hash, explicit.launch_sha256());
    assert_ne!(hash, declaration(0).launch_sha256());
}

#[test]
fn entry_selection_is_explicit_or_unambiguous_and_never_probes() {
    let mut manifest = declaration(1);
    assert_eq!(manifest.open_target().unwrap().unwrap().port, 5173);
    let mut backend = manifest.services[0].clone();
    backend.name = "api".into();
    backend.port = Some(8000);
    manifest.services.push(backend);
    assert_eq!(
        manifest.open_target().unwrap_err().code,
        "OPEN_TARGET_REQUIRED"
    );
    manifest.open = Some(Open {
        service: "web".into(),
        path: "/".into(),
    });
    assert_eq!(manifest.open_target().unwrap().unwrap().service, "web");
    manifest.open.as_mut().unwrap().service = "missing".into();
    assert_eq!(manifest.open_target().unwrap_err().code, "VALIDATION_ERROR");
    manifest.open = None;
    for service in &mut manifest.services {
        service.port = None;
    }
    assert_eq!(manifest.open_target().unwrap(), None);
}

#[test]
fn v1_fields_cannot_sneak_into_v0_and_paths_and_timeouts_are_bounded() {
    let root = TempDir::new().unwrap();
    for path in [
        "",
        "relative",
        "//host",
        "/\\host",
        "https://host/",
        "/?token=x",
        "/#fragment",
        "/\n",
    ] {
        let mut m = declaration(1);
        m.open = Some(Open {
            service: "web".into(),
            path: path.into(),
        });
        assert!(!m.validate(root.path()).is_empty(), "{path:?}");
    }
    for (startup, stop) in [
        (0, 5000),
        (999, 5000),
        (600001, 5000),
        (30000, 0),
        (30000, 60001),
    ] {
        let mut m = declaration(1);
        m.services[0].startup_timeout_ms = Some(startup);
        m.services[0].stop_timeout_ms = Some(stop);
        assert!(!m.validate(root.path()).is_empty());
    }
    let mut m = declaration(0);
    m.open = Some(Open {
        service: "web".into(),
        path: "/".into(),
    });
    m.services[0].startup_timeout_ms = Some(30_000);
    assert_eq!(m.validate(root.path()).len(), 2);
    let mut m = declaration(1);
    m.services[0].startup_timeout_ms = Some(600_000);
    m.services[0].stop_timeout_ms = Some(60_000);
    assert!(m.validate(root.path()).is_empty());
}

#[test]
fn migration_preserves_original_bytes_identity_and_old_confirmation() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let old_root = project(temp.path(), "old", 0);
    let old = apphost::import(&data, &apphost::scan(&old_root, 0).unwrap())
        .unwrap()
        .remove(0);
    legacy_registry(&data);
    let bytes = fs::read(data.join("registry.json")).unwrap();
    let new_root = project(temp.path(), "new", 1);
    let draft = apphost::scan(&new_root, 0).unwrap();
    assert_eq!(draft.schema_version, 1);
    let new = apphost::import(&data, &draft).unwrap().remove(0);
    assert!(new.confirmed_manifest.is_some());
    assert_eq!(
        fs::read(data.join("registry.v0.backup.json")).unwrap(),
        bytes
    );
    let listed = apphost::list(&data, false).unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|i| !i.requires_launch_confirmation));
    assert_eq!(
        listed
            .iter()
            .find(|i| i.installation.installation_id == old.installation_id)
            .unwrap()
            .installation
            .confirmed_launch_sha256,
        old.confirmed_launch_sha256
    );
    assert_eq!(
        apphost::import(&data, &draft).unwrap()[0].installation_id,
        new.installation_id
    );
    // Read/list/reimport cannot rewrite the historical backup.
    assert_eq!(
        fs::read(data.join("registry.v0.backup.json")).unwrap(),
        bytes
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(data.join("registry.v0.backup.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}

#[test]
fn v1_changed_entry_requires_review_and_tampered_snapshots_are_rejected() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let root = project(temp.path(), "app", 1);
    apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap();
    let mut changed = declaration(1);
    changed.open = Some(Open {
        service: "web".into(),
        path: "/changed".into(),
    });
    fs::write(root.join("apphost.toml"), changed.to_toml().unwrap()).unwrap();
    let mut draft = apphost::scan(&root, 0).unwrap();
    assert_eq!(
        apphost::import(&data, &draft).unwrap_err().code,
        "REVIEW_REQUIRED"
    );
    apphost::registry::annotate_launch_changes(&data, &mut draft).unwrap();
    assert!(
        draft.candidates[0]
            .notes
            .iter()
            .any(|n| n.message.contains("/changed"))
    );
    let mut output = Vec::new();
    apphost::review::review(&mut draft, &mut "y\n".as_bytes(), &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("服务 web，路径 /changed"));
    assert!(!output.contains("Some(Open"));
    assert!(
        draft.candidates[0]
            .notes
            .iter()
            .all(|note| !note.message.contains("Some(Open"))
    );
    apphost::import(&data, &draft).unwrap();
    let registry_path = data.join("registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    registry["installations"][0]["confirmed_manifest"]["open"]["path"] = "/tampered".into();
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    assert_eq!(
        apphost::list(&data, false).unwrap_err().code,
        "REGISTRY_INVALID"
    );
}

#[test]
fn relink_preserves_history_but_requires_confirmation_at_the_new_root() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let root = project(temp.path(), "old", 0);
    let initial = apphost::import(&data, &apphost::scan(&root, 0).unwrap())
        .unwrap()
        .remove(0);
    let moved = temp.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    apphost::relink(&data, initial.installation_id, &moved).unwrap();
    let listed = apphost::list(&data, false).unwrap().remove(0);
    assert_eq!(listed.launch_changed, Some(false));
    assert!(listed.requires_launch_confirmation);
    assert_eq!(
        listed.installation.confirmed_launch_sha256,
        initial.confirmed_launch_sha256
    );
    let mut draft = apphost::scan(&moved, 0).unwrap();
    assert_eq!(
        apphost::import(&data, &draft).unwrap_err().code,
        "REVIEW_REQUIRED"
    );
    apphost::registry::annotate_launch_changes(&data, &mut draft).unwrap();
    apphost::review::review(&mut draft, &mut "y\n".as_bytes(), &mut Vec::new()).unwrap();
    apphost::import(&data, &draft).unwrap();
    assert!(!apphost::list(&data, false).unwrap()[0].requires_launch_confirmation);
}

#[test]
fn cli_v1_validate_scan_import_and_list_smoke() {
    let temp = TempDir::new().unwrap();
    let root = project(temp.path(), "app", 1);
    let data = temp.path().join("data");
    let run = |args: &[&str]| {
        let result = Command::new(env!("CARGO_BIN_EXE_apphost"))
            .arg("--data-dir")
            .arg(&data)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()
    };
    let checked = run(&["validate", root.to_str().unwrap(), "--check-launch"]);
    assert_eq!(checked["manifest"]["schema_version"], 1);
    assert!(!checked["warnings"].as_array().unwrap().is_empty());
    let draft = temp.path().join("draft.json");
    run(&[
        "scan",
        root.to_str().unwrap(),
        "--output",
        draft.to_str().unwrap(),
    ]);
    run(&["import", draft.to_str().unwrap(), "--yes"]);
    assert_eq!(run(&["list"])[0]["requires_launch_confirmation"], false);
}

#[test]
fn version_mismatch_and_conflicting_backup_do_not_mutate_registry() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let old = project(temp.path(), "old", 0);
    apphost::import(&data, &apphost::scan(&old, 0).unwrap()).unwrap();
    legacy_registry(&data);
    let original = fs::read(data.join("registry.json")).unwrap();
    let new = project(temp.path(), "new", 1);
    let mut draft = apphost::scan(&new, 0).unwrap();
    draft.schema_version = 0;
    assert_eq!(
        apphost::import(&data, &draft).unwrap_err().code,
        "INVALID_INPUT"
    );
    assert!(!data.join("registry.v0.backup.json").exists());
    draft.schema_version = 1;
    fs::write(
        data.join("registry.v0.backup.json"),
        b"independent historical backup",
    )
    .unwrap();
    assert_eq!(apphost::import(&data, &draft).unwrap_err().code, "CONFLICT");
    assert_eq!(fs::read(data.join("registry.json")).unwrap(), original);
    assert_eq!(
        fs::read(data.join("registry.v0.backup.json")).unwrap(),
        b"independent historical backup"
    );
}

#[test]
fn unknown_versions_and_legacy_snapshots_with_new_fields_remain_invalid() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let root = project(temp.path(), "old", 0);
    apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap();
    let path = data.join("registry.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut changed = original.clone();
    changed["schema_version"] = 2.into();
    fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(
        apphost::list(&data, false).unwrap_err().code,
        "REGISTRY_INVALID"
    );
    changed = original.clone();
    changed["installations"][0]["confirmed_services"][0]["startup_timeout_ms"] = 30_000.into();
    fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(
        apphost::list(&data, false).unwrap_err().code,
        "REGISTRY_INVALID"
    );
    changed = original;
    changed["installations"][0]
        .as_object_mut()
        .unwrap()
        .remove("confirmed_services");
    fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    let listed = apphost::list(&data, false).unwrap();
    assert!(listed[0].installation.confirmed_services.is_none());
    assert!(!listed[0].requires_launch_confirmation);
}

#[test]
fn v1_explicit_open_is_validated_on_read_without_rewriting_the_file() {
    let temp = TempDir::new().unwrap();
    let root = project(temp.path(), "app", 1);
    let path = root.join("apphost.toml");
    let text = fs::read_to_string(&path).unwrap().replace(
        "name = \"Demo\"",
        "name = \"Demo\"\nopen = { service = \"web\" }",
    );
    fs::write(&path, &text).unwrap();
    let valid = apphost::validate(&path).unwrap();
    assert_eq!(valid.open.unwrap().path, "/");
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    for invalid in [
        text.replace("service = \"web\"", "service = \"missing\""),
        text.replace("service = \"web\"", "service = \"web\", typo = 1"),
        text.replace("port = 5173", "port = 0"),
        text.replace("schema_version = 1", "schema_version = 2"),
    ] {
        fs::write(&path, invalid).unwrap();
        assert_eq!(
            apphost::validate(&path).unwrap_err().code,
            "VALIDATION_ERROR"
        );
    }
}

#[cfg(unix)]
#[test]
fn failed_first_v1_import_then_v0_edit_can_retry_migration() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let old = project(temp.path(), "old", 0);
    apphost::import(&data, &apphost::scan(&old, 0).unwrap()).unwrap();
    legacy_registry(&data);
    let new = temp.path().join("new");
    fs::create_dir(&new).unwrap();
    fs::write(new.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
    let mut draft = apphost::scan(&new, 0).unwrap();
    draft.schema_version = 1;
    draft.candidates[0]
        .manifest
        .as_mut()
        .unwrap()
        .schema_version = 1;
    fs::set_permissions(&new, fs::Permissions::from_mode(0o500)).unwrap();
    let result = apphost::import(&data, &draft);
    fs::set_permissions(&new, fs::Permissions::from_mode(0o700)).unwrap();
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let error = result.unwrap_err();
    assert!(error.details["completed"].as_array().unwrap().is_empty());
    let previous = fs::read(data.join("registry.v0.backup.json")).unwrap();
    legacy_label(&data, "renamed by historical v0 writer after failure");
    let current = fs::read(data.join("registry.json")).unwrap();
    assert_ne!(previous, current);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&current).unwrap()["schema_version"],
        0
    );
    apphost::import(&data, &draft).unwrap();
    assert_eq!(
        fs::read(data.join("registry.v0.backup.json")).unwrap(),
        current
    );
    assert_eq!(apphost::list(&data, false).unwrap().len(), 2);
}

#[test]
fn relink_refreshes_provisional_v0_backup_and_preserves_identity() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let old = project(temp.path(), "old", 0);
    let id = apphost::import(&data, &apphost::scan(&old, 0).unwrap()).unwrap()[0].installation_id;
    legacy_registry(&data);
    fs::copy(
        data.join("registry.json"),
        data.join("registry.v0.backup.json"),
    )
    .unwrap();
    legacy_label(&data, "new label");
    let current = fs::read(data.join("registry.json")).unwrap();
    let moved = temp.path().join("moved");
    fs::rename(old, &moved).unwrap();
    let updated = apphost::relink(&data, id, &moved).unwrap();
    assert_eq!(updated.installation_id, id);
    assert_eq!(updated.human_label, "new label");
    assert_eq!(
        fs::read(data.join("registry.v0.backup.json")).unwrap(),
        current
    );
}

#[test]
fn first_legacy_write_migrates_for_each_registration_mutation() {
    for operation in [
        "label",
        "detach",
        "retention",
        "relink",
        "import",
        "detach-many",
    ] {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join("data");
        let root = project(temp.path(), "old", 0);
        let original = apphost::import(&data, &apphost::scan(&root, 0).unwrap())
            .unwrap()
            .remove(0);
        legacy_registry(&data);
        let before = fs::read(data.join("registry.json")).unwrap();
        let updated = match operation {
            "label" => apphost::label(&data, original.installation_id, "changed").unwrap(),
            "detach" => apphost::detach(&data, original.installation_id).unwrap(),
            "retention" => apphost::retention(
                &data,
                original.installation_id,
                apphost::registry::RetentionPolicy::Dormant,
            )
            .unwrap(),
            "relink" => {
                let moved = temp.path().join("moved");
                fs::rename(&root, &moved).unwrap();
                apphost::relink(&data, original.installation_id, &moved).unwrap()
            }
            "import" => apphost::import(&data, &apphost::scan(&root, 0).unwrap())
                .unwrap()
                .remove(0),
            _ => apphost::registry::detach_many(
                &data,
                &[original.installation_id, original.installation_id],
            )
            .unwrap()
            .remove(0),
        };
        assert_eq!(updated.installation_id, original.installation_id);
        assert_eq!(
            updated.confirmed_launch_sha256,
            original.confirmed_launch_sha256
        );
        assert_eq!(updated.revision, 1, "{operation}");
        assert_eq!(
            fs::read(data.join("registry.v0.backup.json")).unwrap(),
            before
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(data.join("registry.json")).unwrap()).unwrap();
        assert_eq!(saved["schema_version"], 1);
        assert_eq!(saved["installations"][0]["revision"], updated.revision);
    }
}

#[test]
fn revision_overflow_rejects_changes_without_wrapping_or_resetting() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let root = project(temp.path(), "app", 1);
    let id = apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
    apphost::label(&data, id, "custom").unwrap();
    let path = data.join("registry.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["installations"][0]["revision"] = u64::MAX.into();
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    assert_eq!(
        apphost::label(&data, id, "custom").unwrap().revision,
        u64::MAX
    );
    let before = fs::read(&path).unwrap();
    assert_eq!(
        apphost::label(&data, id, "changed").unwrap_err().code,
        "REGISTRY_INVALID"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}
