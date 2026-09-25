use apphost::{
    Manifest, discovery,
    registry::{self, sources},
};
use serde_json::json;
use std::fs;
use uuid::Uuid;

fn draft(root: &std::path::Path, data: &std::path::Path) -> apphost::Draft {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"external","scripts":{"dev":"node index.js"}}"#,
    )
    .unwrap();
    let mut d = apphost::scan(root, 0).unwrap();
    let m: Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"external","services":[{"name":"worker","command":["/bin/sleep","30"]}]})).unwrap();
    d.candidates[0].manifest = Some(m);
    d.candidates[0].selected = true;
    sources::select_external(data, &mut d).unwrap();
    d
}
#[test]
fn external_import_rescan_and_launch_use_one_source_without_project_writes() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    let root = t.path().join("app");
    let d = draft(&root, &data);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let first = registry::import_with_receipts(&data, &d).unwrap();
    let i = &first[0].installation;
    assert!(!root.join("apphost.toml").exists());
    assert_eq!(i.manifest_source, registry::ManifestSource::External);
    assert_eq!(
        first[0].created_files[0].path,
        sources::external_path(&data, i.installation_id)
    );
    assert_eq!(registry::import(&data, &d).unwrap()[0], *i);
    let d2 = discovery::scan_with_registry(&root, &discovery::ScanOptions::default(), Some(&data))
        .unwrap();
    assert_eq!(d2.schema_version, 2);
    assert_eq!(d2.candidates.len(), 1);
    assert_eq!(registry::import(&data, &d2).unwrap()[0], *i);
    assert_eq!(
        registry::launch_snapshot(&data, i.installation_id)
            .unwrap()
            .manifest(),
        d.candidates[0].manifest.as_ref().unwrap()
    );
    assert_eq!(
        registry::editing::read(&data, i.installation_id).unwrap()["manifest_source"],
        "external"
    );
    assert_eq!(
        registry::list(&data, false).unwrap()[0].manifest.as_ref(),
        d.candidates[0].manifest.as_ref()
    );
    let detached = registry::detach(&data, i.installation_id).unwrap();
    assert!(detached.retained_files_in(&data)[0].matches_created_hash);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
#[test]
fn old_drafts_and_foreign_reserved_ids_cannot_change_write_target() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    let mut d = draft(&t.path().join("app"), &data);
    d.schema_version = 1;
    assert_eq!(
        registry::import(&data, &d).unwrap_err().code,
        "INVALID_INPUT"
    );
    assert!(!data.exists());
    d.schema_version = 2;
    let i = registry::import(&data, &d).unwrap().remove(0);
    let mut other = draft(&t.path().join("other"), &data);
    other.candidates[0]
        .external
        .as_mut()
        .unwrap()
        .installation_id = i.installation_id;
    assert_eq!(
        registry::import(&data, &other).unwrap_err().code,
        "SOURCE_CONFLICT"
    );
    d.candidates[0].external = None;
    assert_eq!(
        registry::import(&data, &d).unwrap_err().code,
        "SOURCE_CONFLICT"
    );
}
#[test]
fn source_conflicts_are_visible_and_relink_validates_external_cwd() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    let root = t.path().join("app");
    let d = draft(&root, &data);
    let i = registry::import(&data, &d).unwrap().remove(0);
    fs::write(
        root.join("apphost.toml"),
        d.candidates[0]
            .manifest
            .as_ref()
            .unwrap()
            .to_toml()
            .unwrap(),
    )
    .unwrap();
    let scan =
        discovery::scan_with_registry(&root, &discovery::ScanOptions::default(), Some(&data))
            .unwrap();
    assert!(!scan.candidates[0].selected);
    assert!(
        scan.candidates[0]
            .notes
            .iter()
            .any(|n| n.field == "manifest_source")
    );
    fs::remove_file(root.join("apphost.toml")).unwrap();
    let new = t.path().join("moved");
    fs::rename(&root, &new).unwrap();
    registry::relink(&data, i.installation_id, &new).unwrap();
    assert_eq!(
        registry::launch_snapshot(&data, i.installation_id)
            .err()
            .unwrap()
            .code,
        "REVIEW_REQUIRED"
    );
    assert!(sources::external_path(&data, i.installation_id).exists());
}
#[test]
fn orphan_external_file_retries_exact_content_and_stale_changes_are_rejected() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    let d = draft(&t.path().join("app"), &data);
    let i = registry::import(&data, &d).unwrap().remove(0);
    fs::remove_file(data.join("registry.json")).unwrap();
    assert_eq!(
        registry::import(&data, &d).unwrap()[0].installation_id,
        i.installation_id
    );
    let scan =
        discovery::scan_with_registry(&i.path, &discovery::ScanOptions::default(), Some(&data))
            .unwrap();
    let path = sources::external_path(&data, i.installation_id);
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("# changed\n{original}")).unwrap();
    assert_eq!(
        registry::import(&data, &scan).unwrap_err().code,
        "STALE_DRAFT"
    );
    assert!(fs::read_to_string(path).unwrap().starts_with("# changed"));
}

fn switch_request(
    data: &std::path::Path,
    id: Uuid,
    target: registry::ManifestSource,
) -> sources::SwitchRequest {
    let review = sources::review_switch(data, id, target).unwrap();
    sources::SwitchRequest {
        revision: review["revision"].as_u64().unwrap(),
        base_sha256: review["base_sha256"].as_str().unwrap().into(),
        target_source: target,
        target_sha256: review["target_sha256"].as_str().map(str::to_owned),
        confirmed_launch_sha256: review["launch_sha256"].as_str().unwrap().into(),
    }
}
#[test]
fn switching_preserves_both_files_and_requires_fresh_review_of_both_sources() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    let root = t.path().join("app");
    let d = draft(&root, &data);
    let i = registry::import(&data, &d).unwrap().remove(0);
    let request = switch_request(&data, i.installation_id, registry::ManifestSource::Project);
    let external = fs::read(sources::external_path(&data, i.installation_id)).unwrap();
    fs::write(root.join("apphost.toml"), &external).unwrap();
    assert_eq!(
        sources::switch(
            &data,
            i.installation_id,
            &request,
            apphost::journal::Actor::Human
        )
        .unwrap_err()
        .code,
        "EDIT_CONFLICT"
    );
    let request = switch_request(&data, i.installation_id, registry::ManifestSource::Project);
    let queued = registry::launch_snapshot(&data, i.installation_id).unwrap();
    sources::switch(
        &data,
        i.installation_id,
        &request,
        apphost::journal::Actor::Human,
    )
    .unwrap();
    assert_eq!(
        queued.with_current(&data, || Ok(())).unwrap_err().code,
        "STALE_LAUNCH"
    );
    assert_eq!(
        registry::registered(&data, i.installation_id)
            .unwrap()
            .manifest_source,
        registry::ManifestSource::Project
    );
    assert_eq!(
        fs::read(sources::external_path(&data, i.installation_id)).unwrap(),
        external
    );
    let back = switch_request(&data, i.installation_id, registry::ManifestSource::External);
    let changed = String::from_utf8(external.clone())
        .unwrap()
        .replace("\"30\"", "\"31\"");
    fs::write(sources::external_path(&data, i.installation_id), changed).unwrap();
    assert_eq!(
        sources::switch(
            &data,
            i.installation_id,
            &back,
            apphost::journal::Actor::Human
        )
        .unwrap_err()
        .code,
        "EDIT_CONFLICT"
    );
    let back = switch_request(&data, i.installation_id, registry::ManifestSource::External);
    sources::switch(
        &data,
        i.installation_id,
        &back,
        apphost::journal::Actor::Human,
    )
    .unwrap();
    assert_eq!(fs::read(root.join("apphost.toml")).unwrap(), external);
    assert!(registry::launch_snapshot(&data, i.installation_id).is_ok());
}
#[test]
fn switch_failure_retains_target_and_never_changes_active_pointer() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    let root = t.path().join("app");
    let d = draft(&root, &data);
    let i = registry::import(&data, &d).unwrap().remove(0);
    let request = switch_request(&data, i.installation_id, registry::ManifestSource::Project);
    fs::create_dir(root.join("apphost.toml")).unwrap();
    assert!(
        sources::switch(
            &data,
            i.installation_id,
            &request,
            apphost::journal::Actor::Human
        )
        .is_err()
    );
    assert_eq!(
        registry::registered(&data, i.installation_id)
            .unwrap()
            .manifest_source,
        registry::ManifestSource::External
    );
    fs::remove_dir(root.join("apphost.toml")).unwrap();
    let mut wrong = request.clone();
    wrong.confirmed_launch_sha256 = "0".repeat(64);
    assert_eq!(
        sources::switch(
            &data,
            i.installation_id,
            &wrong,
            apphost::journal::Actor::Human
        )
        .unwrap_err()
        .code,
        "REVIEW_REQUIRED"
    );
    assert!(!root.join("apphost.toml").exists());
    sources::switch(
        &data,
        i.installation_id,
        &request,
        apphost::journal::Actor::Human,
    )
    .unwrap();
    let record = registry::registered(&data, i.installation_id).unwrap();
    assert_eq!(record.retained_files_in(&data).len(), 2);
}

#[test]
#[cfg(target_os = "macos")]
fn cli_external_scan_import_validate_and_switch_are_json() {
    use std::process::Command;
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    let root = t.path().join("app");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"app","scripts":{"dev":"node index.js"}}"#,
    )
    .unwrap();
    let call = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_apphost"))
            .arg("--data-dir")
            .arg(&data)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stderr.is_empty());
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let draft_file = t.path().join("draft.json");
    call(&[
        "scan",
        root.to_str().unwrap(),
        "--external",
        "--output",
        draft_file.to_str().unwrap(),
    ]);
    let imported = call(&["import", draft_file.to_str().unwrap(), "--yes"]);
    let id = imported[0]["installation_id"].as_str().unwrap();
    let id_value = Uuid::parse_str(id).unwrap();
    let external = sources::external_path(&data, id_value);
    call(&[
        "validate",
        external.to_str().unwrap(),
        "--project-root",
        root.to_str().unwrap(),
    ]);
    let review = call(&["manifest", "review-switch", id, "project"]);
    call(&[
        "manifest",
        "switch",
        id,
        "project",
        "--revision",
        &review["revision"].to_string(),
        "--base-sha256",
        review["base_sha256"].as_str().unwrap(),
        "--confirm",
        review["launch_sha256"].as_str().unwrap(),
    ]);
    assert!(root.join("apphost.toml").is_file());
    assert!(external.is_file());
}
