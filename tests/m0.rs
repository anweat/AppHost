use apphost::{
    discovery::{Draft, INPUTS},
    manifest::{Manifest, Service},
    *,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;
use uuid::Uuid;

fn node(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("package.json"),
        json!({"name": name, "scripts": {"dev": "touch SHOULD_NOT_RUN"}}).to_string(),
    )
    .unwrap();
    path
}

fn declaration(name: &str, command: &[&str]) -> Manifest {
    Manifest {
        open: None,
        schema_version: 0,
        project_id: Uuid::new_v4(),
        name: name.into(),
        description: String::new(),
        services: vec![Service {
            startup_timeout_ms: None,
            stop_timeout_ms: None,
            name: "app".into(),
            command: command.iter().map(|s| s.to_string()).collect(),
            cwd: ".".into(),
            port: None,
            env: BTreeMap::new(),
        }],
    }
}

fn write_manifest(path: &Path, manifest: &Manifest) {
    fs::write(
        path.join("apphost.toml"),
        toml::to_string_pretty(manifest).unwrap(),
    )
    .unwrap();
}

fn cli(args: &[&str], data: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_apphost"))
        .args(args)
        .env("APPHOST_DATA_DIR", data)
        .output()
        .unwrap()
}

fn value(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn cli_smoke_node_python_and_explicit_native_manifest() {
    let temp = TempDir::new().unwrap();
    let projects = temp.path().join("projects");
    let node = node(&projects, "前端 with spaces");
    let python = projects.join("python");
    fs::create_dir_all(&python).unwrap();
    fs::write(
        python.join("pyproject.toml"),
        "[project]\nname='python'\n[project.scripts]\nserver='demo:main'\n",
    )
    .unwrap();
    let native = projects.join("native");
    fs::create_dir_all(&native).unwrap();
    let native_manifest = declaration("Native", &["./server", "--port", "8080"]);
    write_manifest(&native, &native_manifest);
    let original_native = fs::read(native.join("apphost.toml")).unwrap();
    let data = temp.path().join("data");
    let draft_path = temp.path().join("draft.json");
    let scanned = value(cli(
        &[
            "scan",
            projects.to_str().unwrap(),
            "--output",
            draft_path.to_str().unwrap(),
        ],
        &data,
    ));
    assert_eq!(scanned["candidates"].as_array().unwrap().len(), 3);
    let mut reviewed = read_draft(&draft_path).unwrap();
    let python_candidate = reviewed
        .candidates
        .iter_mut()
        .find(|c| c.path.ends_with("python"))
        .unwrap();
    assert!(!python_candidate.selected);
    python_candidate.selected = true;
    fs::write(&draft_path, serde_json::to_vec(&reviewed).unwrap()).unwrap();
    assert!(!node.join("apphost.toml").exists());
    assert!(!data.exists());
    let refused = cli(&["import", draft_path.to_str().unwrap()], &data);
    assert_eq!(refused.status.code(), Some(2));
    assert!(!node.join("apphost.toml").exists());
    let imported = value(cli(
        &["import", draft_path.to_str().unwrap(), "--yes"],
        &data,
    ));
    assert_eq!(imported.as_array().unwrap().len(), 3);
    let again = value(cli(
        &["import", draft_path.to_str().unwrap(), "--yes"],
        &data,
    ));
    let mut expected = imported.clone();
    for item in expected.as_array_mut().unwrap() {
        item["created_files"] = json!([]);
    }
    assert_eq!(again, expected);
    assert_eq!(
        fs::read(native.join("apphost.toml")).unwrap(),
        original_native
    );
    let listed = value(cli(&["list"], &data));
    assert_eq!(listed.as_array().unwrap().len(), 3);
    for path in [&node, &python, &native] {
        value(cli(&["validate", path.to_str().unwrap()], &data));
        assert!(!path.join("SHOULD_NOT_RUN").exists());
    }
    let id = imported[0]["installation_id"].as_str().unwrap();
    value(cli(&["label", id, "My App"], &data));
    value(cli(&["detach", id], &data));
    value(cli(&["detach", id], &data));
    assert_eq!(value(cli(&["list"], &data)).as_array().unwrap().len(), 2);
    assert_eq!(
        value(cli(&["list", "--all"], &data))
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let restored = value(cli(
        &["import", draft_path.to_str().unwrap(), "--yes"],
        &data,
    ));
    assert_eq!(restored[0]["installation_id"], id);
    assert_eq!(restored[0]["human_label"], "My App");
}

#[test]
fn field_validation_and_working_directory_boundaries() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let good = declaration("Demo", &["program", ""]);
    assert!(good.validate(root).is_empty());
    for cwd in [
        "..",
        "../outside",
        "/tmp",
        "C:\\temp",
        "..\\outside",
        "missing",
        "",
    ] {
        let mut manifest = good.clone();
        manifest.services[0].cwd = cwd.into();
        assert!(
            manifest
                .validate(root)
                .iter()
                .any(|i| i.field.ends_with("cwd")),
            "{cwd}"
        );
    }
    fs::write(root.join("file"), "").unwrap();
    let mut bad = good.clone();
    bad.services[0].cwd = "file".into();
    assert!(!bad.validate(root).is_empty());
    bad = good.clone();
    bad.project_id = Uuid::nil();
    // Version 1 is defined by M1; unknown future versions must still fail.
    bad.schema_version = 2;
    bad.name = " ".into();
    bad.services[0].port = Some(0);
    bad.services[0].command = vec![];
    bad.services[0].env.insert("BAD=KEY".into(), "x".into());
    assert_eq!(bad.validate(root).len(), 6);
    bad = good.clone();
    bad.services.push(bad.services[0].clone());
    assert!(
        bad.validate(root)
            .iter()
            .any(|i| i.field == "services[1].name")
    );
    for command in [vec!["".into()], vec!["ok".into(), "\0".into()]] {
        bad = good.clone();
        bad.services[0].command = command;
        assert!(!bad.validate(root).is_empty());
    }
    bad = good.clone();
    bad.services.clear();
    assert!(!bad.validate(root).is_empty());
    bad = good;
    bad.services[0].port = Some(65535);
    assert!(bad.validate(root).is_empty());
}

#[test]
fn manifest_parse_rejects_unknown_fields_duplicate_keys_and_out_of_range_port() {
    let temp = TempDir::new().unwrap();
    let manifest = declaration("Test", &["test"]);
    let text = toml::to_string_pretty(&manifest).unwrap();
    for invalid in [
        format!("typo = 1\n{text}"),
        format!("name = 'duplicate'\n{text}"),
        text.replace("name = \"app\"", "name = \"app\"\nport = 65536"),
        "not toml = [".into(),
    ] {
        fs::write(temp.path().join("apphost.toml"), invalid).unwrap();
        assert_eq!(validate(temp.path()).unwrap_err().code, "VALIDATION_ERROR");
    }
}

#[test]
fn scans_skip_dependencies_stop_at_projects_and_obey_depth() {
    let temp = TempDir::new().unwrap();
    let app = node(temp.path(), "app");
    node(&app, "nested");
    for excluded in ["node_modules", ".git", ".venv", "target", "dist"] {
        node(&temp.path().join(excluded), "ignored");
    }
    node(&temp.path().join("group"), "second");
    assert!(scan(temp.path(), 0).unwrap().candidates.is_empty());
    assert_eq!(scan(temp.path(), 1).unwrap().candidates.len(), 1);
    assert_eq!(scan(temp.path(), 2).unwrap().candidates.len(), 2);
    assert!(scan(temp.path(), 17).is_err());
    assert!(scan(&temp.path().join("missing"), 3).is_err());
    assert!(scan(&app.join("package.json"), 3).is_err());
}

#[test]
fn ambiguous_and_broken_projects_remain_visible_unselected() {
    let temp = TempDir::new().unwrap();
    let mixed = node(temp.path(), "mixed");
    fs::write(mixed.join("pyproject.toml"), "[project]\nname='mixed'").unwrap();
    let conflicting = node(temp.path(), "conflicting");
    fs::write(conflicting.join("yarn.lock"), "").unwrap();
    fs::write(conflicting.join("package-lock.json"), "{}").unwrap();
    let broken = node(temp.path(), "broken");
    fs::write(broken.join("apphost.toml"), "bad = [").unwrap();
    let noscripts = node(temp.path(), "noscripts");
    fs::write(noscripts.join("package.json"), "{}").unwrap();
    let draft = scan(temp.path(), 3).unwrap();
    assert_eq!(draft.candidates.len(), 4);
    assert!(
        draft
            .candidates
            .iter()
            .all(|c| !c.selected && !c.issues.is_empty())
    );
    assert!(
        draft
            .candidates
            .iter()
            .find(|c| c.path == broken.canonicalize().unwrap())
            .unwrap()
            .manifest
            .is_none()
    );
}

#[test]
fn package_manager_selection_and_python_ambiguity() {
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    fs::write(
        path.join("package.json"),
        r#"{"packageManager":"pnpm@10.0.0","scripts":{"start":"server"}}"#,
    )
    .unwrap();
    let draft = scan(&path, 0).unwrap();
    assert_eq!(
        draft.candidates[0].manifest.as_ref().unwrap().services[0].command,
        ["pnpm", "run", "start"]
    );
    fs::write(path.join("package-lock.json"), "{}").unwrap();
    assert!(!scan(&path, 0).unwrap().candidates[0].selected);
    fs::remove_file(path.join("package-lock.json")).unwrap();
    fs::write(
        path.join("package.json"),
        r#"{"packageManager":"unknown@1","scripts":{"dev":"server"}}"#,
    )
    .unwrap();
    assert!(!scan(&path, 0).unwrap().candidates[0].selected);
    fs::remove_file(path.join("package.json")).unwrap();
    fs::write(
        path.join("pyproject.toml"),
        "[project.scripts]\na='a:main'\nb='b:main'",
    )
    .unwrap();
    assert!(!scan(&path, 0).unwrap().candidates[0].selected);
}

#[test]
fn stale_draft_and_invalid_later_item_write_no_manifests() {
    let temp = TempDir::new().unwrap();
    let projects = temp.path().join("projects");
    let a = node(&projects, "a");
    let b = node(&projects, "b");
    let data = temp.path().join("data");
    let mut draft = scan(&projects, 1).unwrap();
    fs::write(b.join("package.json"), r#"{"scripts":{"dev":"changed"}}"#).unwrap();
    assert_eq!(import(&data, &draft).unwrap_err().code, "STALE_DRAFT");
    assert!(!a.join("apphost.toml").exists());
    assert!(!b.join("apphost.toml").exists());
    draft = scan(&projects, 1).unwrap();
    draft.candidates[1]
        .manifest
        .as_mut()
        .unwrap()
        .services
        .clear();
    assert_eq!(import(&data, &draft).unwrap_err().code, "VALIDATION_ERROR");
    assert!(!a.join("apphost.toml").exists());
    draft.candidates[1].selected = false;
    assert_eq!(import(&data, &draft).unwrap().len(), 1);
}

#[test]
fn new_lockfile_or_changed_existing_manifest_invalidates_draft() {
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let draft = scan(&path, 0).unwrap();
    fs::write(path.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'").unwrap();
    let data = temp.path().join("data");
    assert_eq!(import(&data, &draft).unwrap_err().code, "STALE_DRAFT");
    let draft = scan(&path, 0).unwrap();
    import(&data, &draft).unwrap();
    let draft = scan(&path, 0).unwrap();
    let mut text = fs::read_to_string(path.join("apphost.toml")).unwrap();
    text.push_str("\n# user edit\n");
    fs::write(path.join("apphost.toml"), &text).unwrap();
    assert_eq!(import(&data, &draft).unwrap_err().code, "STALE_DRAFT");
    assert_eq!(fs::read_to_string(path.join("apphost.toml")).unwrap(), text);
}

#[test]
fn existing_manifest_is_never_overwritten_and_identity_conflict_is_explicit() {
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let data = temp.path().join("data");
    let draft = scan(&path, 0).unwrap();
    let original = import(&data, &draft).unwrap();
    let mut draft = scan(&path, 0).unwrap();
    draft.candidates[0].manifest.as_mut().unwrap().name = "Changed".into();
    assert_eq!(import(&data, &draft).unwrap_err().code, "CONFLICT");
    let mut manifest = validate(&path).unwrap();
    manifest.project_id = Uuid::new_v4();
    write_manifest(&path, &manifest);
    assert_eq!(
        import(&data, &scan(&path, 0).unwrap()).unwrap_err().code,
        "CONFLICT"
    );
    let listed = list(&data, false).unwrap();
    assert_eq!(
        listed[0].installation.installation_id,
        original[0].installation_id
    );
    assert_eq!(listed[0].manifest_error.as_ref().unwrap().code, "CONFLICT");
}

#[test]
fn copied_project_gets_separate_installation_and_distinct_default_label() {
    let temp = TempDir::new().unwrap();
    let a = node(temp.path(), "a");
    let b = node(temp.path(), "b");
    let manifest = declaration("Same", &["server"]);
    write_manifest(&a, &manifest);
    write_manifest(&b, &manifest);
    let data = temp.path().join("data");
    let entries = import(&data, &scan(temp.path(), 1).unwrap()).unwrap();
    assert_eq!(entries[0].project_id, entries[1].project_id);
    assert_ne!(entries[0].installation_id, entries[1].installation_id);
    assert_ne!(entries[0].human_label, entries[1].human_label);
}

#[test]
fn source_missing_detach_and_broken_manifest_are_separate_facts() {
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let data = temp.path().join("data");
    let entry = import(&data, &scan(&path, 0).unwrap()).unwrap().remove(0);
    fs::remove_file(path.join("apphost.toml")).unwrap();
    let listed = list(&data, false).unwrap();
    assert_eq!(listed[0].registration_state, "REGISTERED");
    assert_eq!(listed[0].manifest_error.as_ref().unwrap().code, "NOT_FOUND");
    fs::rename(&path, temp.path().join("moved")).unwrap();
    assert_eq!(
        list(&data, false).unwrap()[0].registration_state,
        "SOURCE_MISSING"
    );
    detach(&data, entry.installation_id).unwrap();
    assert!(list(&data, false).unwrap().is_empty());
    assert_eq!(list(&data, true).unwrap()[0].registration_state, "DETACHED");
    assert!(temp.path().join("moved/package.json").exists());
}

#[test]
fn corrupted_registry_is_not_reset() {
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let data = temp.path().join("data");
    fs::create_dir(&data).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::write(data.join("registry.json"), "broken").unwrap();
    assert_eq!(
        import(&data, &scan(&path, 0).unwrap()).unwrap_err().code,
        "REGISTRY_INVALID"
    );
    assert!(!path.join("apphost.toml").exists());
    assert_eq!(
        fs::read_to_string(data.join("registry.json")).unwrap(),
        "broken"
    );
}

#[test]
fn duplicate_paths_missing_evidence_and_invalid_versions_are_rejected() {
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let data = temp.path().join("data");
    let original = scan(&path, 0).unwrap();
    assert_eq!(original.candidates[0].evidence.len(), INPUTS.len());
    let mut draft = original.clone();
    draft.candidates.push(draft.candidates[0].clone());
    assert_eq!(import(&data, &draft).unwrap_err().code, "INVALID_INPUT");
    draft = original.clone();
    draft.candidates[0].evidence.clear();
    assert_eq!(import(&data, &draft).unwrap_err().code, "INVALID_INPUT");
    draft = original.clone();
    draft.candidates[0].evidence[0].path = "../../secret".into();
    assert_eq!(import(&data, &draft).unwrap_err().code, "INVALID_INPUT");
    draft = original.clone();
    draft.schema_version = 3;
    assert_eq!(import(&data, &draft).unwrap_err().code, "INVALID_INPUT");
    draft = original;
    draft.root = temp.path().join("data");
    assert_eq!(import(&data, &draft).unwrap_err().code, "STALE_DRAFT");
    assert!(!path.join("apphost.toml").exists());
}

#[test]
fn failed_registry_publish_keeps_manifest_and_allows_retry() {
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let data = temp.path().join("data");
    fs::create_dir(&data).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let draft = scan(&path, 0).unwrap();
    // A nearly full valid registry fails its size boundary after the first project file is published.
    let huge_label = "x".repeat(1024 * 1024 - 300);
    let registry = json!({"schema_version":0,"installations":[{
        "installation_id": Uuid::new_v4(), "project_id": Uuid::new_v4(),
        "path":"/already/registered", "human_label":huge_label, "detached":false
    }]});
    let bytes = serde_json::to_vec(&registry).unwrap();
    assert!(bytes.len() < 1024 * 1024);
    fs::write(data.join("registry.json"), bytes).unwrap();
    let error = import(&data, &draft).unwrap_err();
    assert_eq!(error.code, "REGISTRY_INVALID");
    assert!(error.details["completed"].as_array().unwrap().is_empty());
    assert_eq!(error.details["created_files"].as_array().unwrap().len(), 1);
    assert!(path.join("apphost.toml").exists());
    let manifest = fs::read(path.join("apphost.toml")).unwrap();
    fs::write(
        data.join("registry.json"),
        r#"{"schema_version":0,"installations":[]}"#,
    )
    .unwrap();
    let recovered = import(&data, &draft).unwrap();
    // Content equality permits retry, but cannot prove who created a pre-existing file.
    assert!(recovered[0].created_manifest.is_none());
    assert_eq!(fs::read(path.join("apphost.toml")).unwrap(), manifest);
}

#[test]
fn lock_contention_fails_without_writing_project_and_release_recovers() {
    use fs2::FileExt;
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let data = temp.path().join("data");
    fs::create_dir(&data).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let lock = fs::File::create(data.join("registry.lock")).unwrap();
    lock.lock_exclusive().unwrap();
    let draft = scan(&path, 0).unwrap();
    assert_eq!(import(&data, &draft).unwrap_err().code, "BUSY");
    assert!(!path.join("apphost.toml").exists());
    FileExt::unlock(&lock).unwrap();
    drop(lock);
    import(&data, &draft).unwrap();
}

#[test]
fn oversized_and_malformed_input_and_draft_output_collisions() {
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let draft = scan(&path, 0).unwrap();
    let output = temp.path().join("draft.json");
    write_draft(&output, &draft).unwrap();
    assert_eq!(write_draft(&output, &draft).unwrap_err().code, "CONFLICT");
    let mut value = serde_json::to_value(&draft).unwrap();
    value["unexpected"] = json!(1);
    fs::write(&output, value.to_string()).unwrap();
    assert_eq!(read_draft(&output).unwrap_err().code, "INVALID_INPUT");
    fs::write(&output, "x".repeat(1024 * 1024 + 1)).unwrap();
    assert_eq!(read_draft(&output).unwrap_err().code, "INVALID_INPUT");
    fs::write(path.join("package.json"), "x".repeat(1024 * 1024 + 1)).unwrap();
    assert!(!scan(&path, 0).unwrap().candidates[0].selected);
}

#[test]
fn empty_selection_has_no_registry_side_effect_and_unknown_ids_fail() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let draft = Draft {
        scope: Default::default(),
        groups: vec![],
        schema_version: 0,
        root: temp.path().canonicalize().unwrap(),
        candidates: vec![],
        issues: vec![],
    };
    assert!(import(&data, &draft).unwrap().is_empty());
    assert!(list(&data, false).unwrap().is_empty());
    assert!(!data.exists());
    assert_eq!(detach(&data, Uuid::new_v4()).unwrap_err().code, "NOT_FOUND");
    assert_eq!(
        label(&data, Uuid::new_v4(), "\n").unwrap_err().code,
        "VALIDATION_ERROR"
    );
}

#[cfg(unix)]
#[test]
fn symlink_cycles_project_aliases_manifest_links_and_cwd_escape() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let data = temp.path().join("data");
    symlink(temp.path(), temp.path().join("loop")).unwrap();
    symlink(&path, temp.path().join("alias")).unwrap();
    assert_eq!(scan(temp.path(), 3).unwrap().candidates.len(), 1);
    let first = import(&data, &scan(&path, 0).unwrap()).unwrap();
    let second = import(&data, &scan(&temp.path().join("alias"), 0).unwrap()).unwrap();
    assert_eq!(first[0].installation_id, second[0].installation_id);
    symlink(temp.path(), path.join("escape")).unwrap();
    let mut manifest = validate(&path).unwrap();
    manifest.services[0].cwd = "escape".into();
    assert!(!manifest.validate(&path).is_empty());
    fs::rename(path.join("apphost.toml"), temp.path().join("external.toml")).unwrap();
    symlink(temp.path().join("external.toml"), path.join("apphost.toml")).unwrap();
    assert!(validate(&path).is_err());
    assert!(!scan(&path, 0).unwrap().candidates[0].selected);
}

#[cfg(unix)]
#[test]
fn unreadable_root_and_project_errors_are_reported() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let path = node(temp.path(), "app");
    let package = path.join("package.json");
    fs::set_permissions(&package, fs::Permissions::from_mode(0o000)).unwrap();
    let can_read_package = fs::read(&package).is_ok();
    let draft = scan(&path, 0).unwrap();
    fs::set_permissions(&package, fs::Permissions::from_mode(0o600)).unwrap();
    // Privileged test runners can read mode-000 files; do not misreport that as a permission failure.
    if !can_read_package {
        assert!(!draft.candidates[0].selected);
    }
    let empty = temp.path().join("unreadable");
    fs::create_dir(&empty).unwrap();
    fs::set_permissions(&empty, fs::Permissions::from_mode(0o000)).unwrap();
    let can_read_directory = fs::read_dir(&empty).is_ok();
    let result = scan(&empty, 0);
    fs::set_permissions(&empty, fs::Permissions::from_mode(0o700)).unwrap();
    if !can_read_directory {
        assert!(result.is_err());
    }
}

#[test]
fn cli_validates_relative_filename_and_reports_structured_errors() {
    let temp = TempDir::new().unwrap();
    write_manifest(temp.path(), &declaration("Relative", &["server"]));
    let output = Command::new(env!("CARGO_BIN_EXE_apphost"))
        .args(["validate", "apphost.toml"])
        .current_dir(temp.path())
        .env_remove("HOME")
        .output()
        .unwrap();
    assert_eq!(value(output)["name"], "Relative");
    let data = temp.path().join("data");
    let output = cli(
        &["validate", temp.path().join("missing").to_str().unwrap()],
        &data,
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "NOT_FOUND");
    assert!(!data.exists());
}

#[cfg(unix)]
#[test]
fn partial_batch_reports_completed_items_and_retries_without_replacing_identity() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let projects = temp.path().join("projects");
    let a = node(&projects, "a");
    let b = node(&projects, "b");
    let data = temp.path().join("data");
    let draft = scan(&projects, 1).unwrap();
    fs::set_permissions(&b, fs::Permissions::from_mode(0o500)).unwrap();
    let probe = b.join("probe");
    if fs::write(&probe, "").is_ok() {
        fs::remove_file(probe).unwrap();
        fs::set_permissions(&b, fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    let result = import(&data, &draft);
    fs::set_permissions(&b, fs::Permissions::from_mode(0o700)).unwrap();
    let error = result.unwrap_err();
    assert_eq!(error.code, "IO_ERROR");
    assert_eq!(error.details["completed"].as_array().unwrap().len(), 1);
    assert!(a.join("apphost.toml").exists());
    assert!(!b.join("apphost.toml").exists());
    let completed_id = error.details["completed"][0]["installation_id"].clone();
    let entries = import(&data, &draft).unwrap();
    assert_eq!(json!(entries[0].installation_id), completed_id);
    assert_eq!(list(&data, false).unwrap().len(), 2);
}

#[test]
fn oversized_later_manifest_is_rejected_before_first_project_write() {
    let temp = TempDir::new().unwrap();
    let projects = temp.path().join("projects");
    let a = node(&projects, "a");
    node(&projects, "b");
    let mut draft = scan(&projects, 1).unwrap();
    draft.candidates[1].manifest.as_mut().unwrap().description = "x".repeat(1024 * 1024);
    let error = import(&temp.path().join("data"), &draft).unwrap_err();
    assert_eq!(error.code, "INVALID_INPUT");
    assert!(!a.join("apphost.toml").exists());
}

// APFS rejects non-UTF-8 names at creation; Unix filesystems that permit them exercise this path.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn non_utf8_child_is_reported_without_breaking_json_output() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let temp = TempDir::new().unwrap();
    let invalid = temp.path().join(OsString::from_vec(vec![b'a', 0xff]));
    fs::create_dir(&invalid).unwrap();
    fs::write(
        invalid.join("package.json"),
        r#"{"scripts":{"dev":"server"}}"#,
    )
    .unwrap();
    let draft = scan(temp.path(), 1).unwrap();
    assert!(draft.candidates.is_empty());
    assert_eq!(draft.issues.len(), 1);
    assert!(serde_json::to_string(&draft).is_ok());
}
