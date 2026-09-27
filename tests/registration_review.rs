use apphost::{registry::RetentionPolicy, *};
use serde_json::{Value, json};
use std::{
    fs,
    io::Cursor,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use tempfile::TempDir;
use uuid::Uuid;

fn node(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"web","scripts":{"dev":"vite"}}"#,
    )
    .unwrap();
}
fn fullstack(root: &Path) {
    node(&root.join("frontend"));
    fs::create_dir_all(root.join("backend")).unwrap();
    fs::write(root.join("backend/pyproject.toml"), "[project]\nname='api'").unwrap();
}
fn reviewed(root: &Path) -> Draft {
    let mut draft = scan(root.parent().unwrap(), 3).unwrap();
    assert_eq!(draft.groups.len(), 1);
    assert!(draft.candidates.iter().all(|c| !c.selected));
    review::review(
        &mut draft,
        &mut Cursor::new(b"y\n[\"uv\",\"run\",\"uvicorn\",\"main:app\"]\n"),
        &mut Vec::new(),
    )
    .unwrap();
    draft
}

#[test]
fn fullstack_confirm_import_and_nested_change_detection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fullstack");
    fullstack(&root);
    let draft = reviewed(&root);
    assert_eq!(draft.candidates.len(), 1);
    let c = &draft.candidates[0];
    assert!(c.selected);
    assert_eq!(c.manifest.as_ref().unwrap().services.len(), 2);
    let data = temp.path().join("data");
    let entries = import(&data, &draft).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, root.canonicalize().unwrap());
    assert!(root.join("apphost.toml").exists());
    assert!(!root.join("frontend/apphost.toml").exists());
    assert!(!root.join("backend/apphost.toml").exists());
    import(&data, &draft).unwrap(); // Partial-write retry evidence remains valid.
    fs::write(
        root.join("backend/pyproject.toml"),
        "[project]\nname='changed'",
    )
    .unwrap();
    assert_eq!(import(&data, &draft).unwrap_err().code, "STALE_DRAFT");
}

#[test]
fn grouping_respects_existing_manifests_ambiguous_siblings_and_rejection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fullstack");
    fullstack(&root);
    let mut draft = scan(temp.path(), 2).unwrap();
    assert_eq!(draft.groups.len(), 1);
    review::review(&mut draft, &mut Cursor::new(b"n\nn\nn\n"), &mut Vec::new()).unwrap();
    assert_eq!(draft.candidates.len(), 2);
    assert!(draft.candidates.iter().all(|c| !c.selected));
    node(&root.join("another"));
    let with_extra = scan(temp.path(), 2).unwrap();
    assert!(with_extra.groups.is_empty());
    assert_eq!(
        with_extra.candidates.iter().filter(|c| c.selected).count(),
        1
    );
    let other = temp.path().join("other");
    fullstack(&other);
    import(
        &temp.path().join("data"),
        &scan(&other.join("frontend"), 0).unwrap(),
    )
    .unwrap();
    assert!(scan(temp.path(), 2).unwrap().groups.is_empty());
}

#[test]
fn grouped_evidence_cannot_be_omitted_or_escape_source_root() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fullstack");
    fullstack(&root);
    let original = reviewed(&root);
    let data = temp.path().join("data");
    let mut draft = original.clone();
    draft.candidates[0]
        .evidence
        .retain(|e| !e.path.starts_with("backend/"));
    assert_eq!(import(&data, &draft).unwrap_err().code, "INVALID_INPUT");
    let mut draft = original;
    draft.candidates[0].source_dirs[0] = "../other".into();
    assert_eq!(import(&data, &draft).unwrap_err().code, "INVALID_INPUT");
    assert!(!temp.path().join("apphost.toml").exists());
}

#[test]
fn association_shares_project_identity_without_merging_installations() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    node(&a);
    node(&b);
    let data = temp.path().join("data");
    let first = import(&data, &scan(&a, 0).unwrap()).unwrap().remove(0);
    let mut copy = scan(&b, 0).unwrap();
    assert_ne!(
        copy.candidates[0].manifest.as_ref().unwrap().project_id,
        first.project_id
    );
    associate(&data, &mut copy, &b, first.installation_id).unwrap();
    let second = import(&data, &copy).unwrap().remove(0);
    assert_eq!(first.project_id, second.project_id);
    assert_ne!(first.installation_id, second.installation_id);
    let c = temp.path().join("c");
    node(&c);
    let third = import(&data, &scan(&c, 0).unwrap()).unwrap().remove(0);
    assert_eq!(
        associate(&data, &mut scan(&c, 0).unwrap(), &c, first.installation_id)
            .unwrap_err()
            .code,
        "CONFLICT"
    );
    assert_eq!(validate(&c).unwrap().project_id, third.project_id);
}

#[test]
fn relink_preserves_identity_label_and_retention_and_rejects_copies() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    node(&root);
    let data = temp.path().join("data");
    let first = import(&data, &scan(&root, 0).unwrap()).unwrap().remove(0);
    label(&data, first.installation_id, "My App").unwrap();
    retention(&data, first.installation_id, RetentionPolicy::Dormant).unwrap();
    let copy = temp.path().join("copy");
    fs::create_dir(&copy).unwrap();
    fs::copy(root.join("apphost.toml"), copy.join("apphost.toml")).unwrap();
    assert_eq!(
        relink(&data, first.installation_id, &copy)
            .unwrap_err()
            .code,
        "CONFLICT"
    );
    let moved = temp.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    assert_eq!(
        list(&data, false).unwrap()[0].registration_state,
        "SOURCE_MISSING"
    );
    let restored = relink(&data, first.installation_id, &moved).unwrap();
    assert_eq!(restored.installation_id, first.installation_id);
    assert_eq!(restored.human_label, "My App");
    assert_eq!(restored.retention_policy, RetentionPolicy::Dormant);
    assert_eq!(
        relink(&data, first.installation_id, &moved)
            .unwrap()
            .installation_id,
        first.installation_id
    );
    assert_eq!(
        list(&data, false).unwrap()[0].registration_state,
        "REGISTERED"
    );
}

#[test]
fn relink_rejects_wrong_identity_or_registered_target() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    node(&a);
    node(&b);
    let first = import(&data, &scan(&a, 0).unwrap()).unwrap().remove(0);
    let second = import(&data, &scan(&b, 0).unwrap()).unwrap().remove(0);
    fs::rename(&a, temp.path().join("moved")).unwrap();
    assert_eq!(
        relink(&data, first.installation_id, &b).unwrap_err().code,
        "CONFLICT"
    );
    let mut manifest = validate(&b).unwrap();
    manifest.project_id = first.project_id;
    fs::write(b.join("apphost.toml"), toml::to_string(&manifest).unwrap()).unwrap();
    assert_eq!(
        relink(&data, first.installation_id, &b).unwrap_err().code,
        "CONFLICT"
    );
    assert_eq!(list(&data, true).unwrap().len(), 2);
    assert_ne!(first.installation_id, second.installation_id);
}

#[test]
fn batch_detach_preflights_all_ids_and_preserves_project_files() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    node(&temp.path().join("a"));
    node(&temp.path().join("b"));
    let entries = import(&data, &scan(temp.path(), 1).unwrap()).unwrap();
    let ids: Vec<_> = entries.iter().map(|i| i.installation_id).collect();
    assert_eq!(
        detach_many(&data, &[ids[0], Uuid::new_v4()])
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
    assert_eq!(list(&data, false).unwrap().len(), 2);
    assert_eq!(detach_many(&data, &ids).unwrap().len(), 2);
    assert!(list(&data, false).unwrap().is_empty());
    assert_eq!(detach_many(&data, &[ids[0], ids[0]]).unwrap().len(), 1);
    for entry in entries {
        assert!(entry.path.join("apphost.toml").exists());
    }
}

#[test]
fn labels_use_directory_branch_and_short_collision_suffixes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("pdf-reader");
    node(&root);
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/test\n").unwrap();
    assert_eq!(
        scan(&root, 0).unwrap().candidates[0].human_label,
        "pdf-reader"
    );
    let a = temp.path().join("a/app");
    let b = temp.path().join("b/app");
    node(&a);
    node(&b);
    let data = temp.path().join("data");
    let first = import(&data, &scan(&a, 0).unwrap()).unwrap().remove(0);
    let second = import(&data, &scan(&b, 0).unwrap()).unwrap().remove(0);
    assert_eq!(second.human_label, "app · 2");
    assert_eq!(
        label(&data, second.installation_id, &first.human_label)
            .unwrap_err()
            .code,
        "CONFLICT"
    );
    detach(&data, first.installation_id).unwrap();
    label(&data, second.installation_id, &first.human_label).unwrap();
    let restored = import(&data, &scan(&a, 0).unwrap()).unwrap().remove(0);
    assert_ne!(restored.human_label, first.human_label);
}

#[test]
fn old_registry_defaults_retention_without_inventing_runtime_state() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    node(&temp.path().join("app"));
    import(&data, &scan(&temp.path().join("app"), 0).unwrap()).unwrap();
    let path = data.join("registry.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["installations"][0]
        .as_object_mut()
        .unwrap()
        .remove("retention_policy");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let entry = list(&data, false).unwrap().remove(0);
    assert_eq!(entry.installation.retention_policy, RetentionPolicy::Active);
    assert_eq!(entry.registration_state, "REGISTERED");
    value["installations"][0]["retention_policy"] = json!("unknown");
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(list(&data, false).unwrap_err().code, "REGISTRY_INVALID");
}

#[test]
fn static_launch_warnings_check_path_permissions_and_package_scripts_without_execution() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    let mut manifest = scan(temp.path(), 0)
        .unwrap()
        .candidates
        .remove(0)
        .manifest
        .unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("npm"), "#!/bin/sh\ntouch should-never-exist\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin.join("npm"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    manifest.services[0]
        .env
        .insert("PATH".into(), bin.to_string_lossy().into_owned());
    assert!(manifest.launch_warnings(temp.path()).is_empty());
    manifest.services[0].command[2] = "missing".into();
    assert_eq!(manifest.launch_warnings(temp.path()).len(), 1);
    manifest.services[0].command[0] = "not-installed".into();
    assert_eq!(manifest.launch_warnings(temp.path()).len(), 1);
    assert!(!temp.path().join("should-never-exist").exists());
    manifest.services[0].command = vec!["./bin/npm".into()];
    assert!(manifest.launch_warnings(temp.path()).is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(bin.join("npm"), fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(manifest.launch_warnings(temp.path()).len(), 1);
    }
}

#[test]
fn cli_review_import_associate_relink_and_batch_detach_smoke() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let root = temp.path().join("fullstack");
    fullstack(&root);
    let draft = temp.path().join("draft.json");
    write_draft(&draft, &scan(temp.path(), 3).unwrap()).unwrap();
    let output = temp.path().join("reviewed.json");
    let mut child = Command::new(env!("CARGO_BIN_EXE_apphost"))
        .args([
            "review",
            draft.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"y\n[\"uv\",\"run\",\"uvicorn\",\"main:app\"]\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{:?}", result);
    let cli = |args: &[&str]| -> Value {
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
        serde_json::from_slice(&result.stdout).unwrap()
    };
    let entries = cli(&["import", output.to_str().unwrap(), "--yes"]);
    let id = entries[0]["installation_id"].as_str().unwrap();
    assert!(cli(&["validate", root.to_str().unwrap(), "--check-launch"])["warnings"].is_array());
    cli(&["retention", id, "archived"]);
    let moved = temp.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    assert_eq!(
        cli(&["relink", id, moved.to_str().unwrap()])["retention_policy"],
        "archived"
    );
    let copy = temp.path().join("copy");
    node(&copy);
    let copy_draft = temp.path().join("copy.json");
    write_draft(&copy_draft, &scan(&copy, 0).unwrap()).unwrap();
    let associated = temp.path().join("associated.json");
    cli(&[
        "associate",
        copy_draft.to_str().unwrap(),
        copy.to_str().unwrap(),
        id,
        "--output",
        associated.to_str().unwrap(),
    ]);
    let copy_entries = cli(&["import", associated.to_str().unwrap(), "--yes"]);
    assert_eq!(entries[0]["project_id"], copy_entries[0]["project_id"]);
    assert_eq!(
        cli(&[
            "detach",
            id,
            copy_entries[0]["installation_id"].as_str().unwrap()
        ])
        .as_array()
        .unwrap()
        .len(),
        2
    );
    assert!(cli(&["list"]).as_array().unwrap().is_empty());
}

#[test]
fn eof_during_review_is_not_consent_and_does_not_publish_a_draft() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fullstack");
    fullstack(&root);
    let mut draft = scan(temp.path(), 2).unwrap();
    assert_eq!(
        review::review(&mut draft, &mut Cursor::new(b""), &mut Vec::new())
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(draft.candidates.len(), 2);
    assert!(draft.candidates.iter().all(|c| !c.selected));
    assert!(!temp.path().join("apphost.toml").exists());
}

#[test]
fn cli_cancelled_review_keeps_output_absent_and_all_can_edit_selected_command() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    let draft_path = temp.path().join("draft.json");
    write_draft(&draft_path, &scan(temp.path(), 0).unwrap()).unwrap();
    let output_path = temp.path().join("reviewed.json");
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_apphost"));
        command.args([
            "review",
            draft_path.to_str().unwrap(),
            "--all",
            "--output",
            output_path.to_str().unwrap(),
        ]);
        command
    };
    let result = command().stdin(Stdio::null()).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(!output_path.exists());
    let mut child = command()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"y\n[\"custom-server\",\"--local\"]\n")
        .unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    let reviewed = read_draft(&output_path).unwrap();
    assert_eq!(
        reviewed.candidates[0].manifest.as_ref().unwrap().services[0].command,
        vec!["custom-server", "--local"]
    );
    assert!(!temp.path().join("apphost.toml").exists());
}

#[test]
fn real_manifest_examples_round_trip_without_implicitly_assigning_service_order() {
    for text in [
        include_str!("../examples/manifests/paper-reader.toml"),
        include_str!("../examples/manifests/comfyui.toml"),
        include_str!("../examples/manifests/kamaen-wiki.toml"),
    ] {
        let manifest: Manifest = toml::from_str(text).unwrap();
        let root = TempDir::new().unwrap();
        for service in &manifest.services {
            fs::create_dir_all(root.path().join(&service.cwd)).unwrap();
        }
        assert!(manifest.validate(root.path()).is_empty());
        assert_eq!(
            toml::from_str::<Manifest>(&toml::to_string(&manifest).unwrap()).unwrap(),
            manifest
        );
    }
}

#[test]
fn git_worktree_and_detached_branches_are_read_live_and_scan_keeps_stable_labels() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("worktree");
    node(&root);
    let metadata = temp.path().join("git-meta");
    fs::create_dir(&metadata).unwrap();
    fs::write(root.join(".git"), "gitdir: ../git-meta\n").unwrap();
    fs::write(metadata.join("HEAD"), "ref: refs/heads/feature/test\n").unwrap();
    let data = temp.path().join("data");
    import(&data, &scan(&root, 0).unwrap()).unwrap();
    assert_eq!(
        list(&data, false).unwrap()[0].display_label,
        "worktree · feature/test"
    );
    fs::write(metadata.join("HEAD"), "a".repeat(40)).unwrap();
    assert_eq!(
        list(&data, false).unwrap()[0].display_label,
        "worktree · aaaaaaaa"
    );
    assert_eq!(
        scan(&root, 0).unwrap().candidates[0].human_label,
        "worktree"
    );
    let options = discovery::ScanOptions {
        max_read_bytes: 1,
        ..Default::default()
    };
    assert_eq!(
        discovery::scan_with_options(&root, &options)
            .unwrap_err()
            .code,
        "SCAN_LIMIT"
    );
}

#[test]
fn root_backend_with_nested_web_forms_one_confirmed_app_and_checks_nested_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("reader");
    node(&root.join("web"));
    fs::write(root.join("pyproject.toml"), "[project]\nname='reader'").unwrap();
    let mut draft = scan(temp.path(), 3).unwrap();
    assert_eq!(draft.candidates.len(), 1);
    assert_eq!(draft.groups.len(), 1);
    let project_id = draft.candidates[0].manifest.as_ref().unwrap().project_id;
    assert!(!draft.candidates[0].selected);
    review::review(
        &mut draft,
        &mut Cursor::new(b"y\n[\"uv\",\"run\",\"uvicorn\",\"backend.main:app\"]\n"),
        &mut Vec::new(),
    )
    .unwrap();
    let manifest = draft.candidates[0].manifest.as_ref().unwrap();
    assert_eq!(manifest.project_id, project_id);
    assert_eq!(manifest.services.len(), 2);
    assert_eq!(manifest.services[0].cwd, ".");
    assert_eq!(manifest.services[1].cwd, "web");
    let data = temp.path().join("data");
    assert_eq!(import(&data, &draft).unwrap().len(), 1);
    assert!(!root.join("web/apphost.toml").exists());
    fs::write(
        root.join("web/package.json"),
        r#"{"scripts":{"dev":"changed"}}"#,
    )
    .unwrap();
    assert_eq!(import(&data, &draft).unwrap_err().code, "STALE_DRAFT");
}

#[test]
fn nested_grouping_preserves_existing_boundaries_and_read_depth_budgets() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    node(&temp.path().join("web"));
    assert!(scan(temp.path(), 0).unwrap().groups.is_empty());
    let bytes = fs::metadata(temp.path().join("package.json"))
        .unwrap()
        .len();
    let options = discovery::ScanOptions {
        max_depth: 1,
        max_read_bytes: bytes,
        ..Default::default()
    };
    assert_eq!(
        discovery::scan_with_options(temp.path(), &options)
            .unwrap_err()
            .code,
        "SCAN_LIMIT"
    );
    node(&temp.path().join("frontend"));
    assert!(scan(temp.path(), 1).unwrap().groups.is_empty());
    let other = TempDir::new().unwrap();
    node(other.path());
    node(&other.path().join("web"));
    import(
        &temp.path().join("data"),
        &scan(&other.path().join("web"), 0).unwrap(),
    )
    .unwrap();
    assert!(scan(other.path(), 1).unwrap().groups.is_empty());
}

fn python_without_script(root: &Path) -> Draft {
    fs::create_dir_all(root.join("web")).unwrap();
    fs::write(
        root.join("pyproject.toml"),
        "[project]\nname='paper-reader'\n",
    )
    .unwrap();
    scan(root, 0).unwrap()
}
fn explicit_fullstack(draft: &Draft) -> String {
    let mut manifest = draft.candidates[0].manifest.clone().unwrap();
    manifest.schema_version = 1;
    manifest.services = serde_json::from_value(json!([
        {"name":"api","command":["uv","run","uvicorn","backend.main:app","--port","8000"],"cwd":".","port":8000},
        {"name":"web","command":["node","node_modules/vite/bin/vite.js","--port","5173"],"cwd":"web","port":5173}
    ])).unwrap();
    manifest.open = Some(apphost::manifest::Open {
        service: "web".into(),
        path: "/".into(),
    });
    manifest.to_toml().unwrap()
}
#[test]
fn desktop_python_review_completes_an_unknown_launch_without_cli_or_execution() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    let data = temp.path().join("data");
    let mut draft = python_without_script(&root);
    assert!(
        draft.candidates[0]
            .issues
            .iter()
            .any(|i| i.field == "rules.python")
    );
    let template = review::candidate_template(&draft.candidates[0]).unwrap();
    assert!(template.contains("command = []"));
    let text = explicit_fullstack(&draft);
    review::edit_candidate(&mut draft, 0, &text).unwrap();
    assert!(draft.candidates[0].issues.is_empty());
    assert!(!draft.candidates[0].selected);
    assert!(!root.join("apphost.toml").exists());
    assert!(!data.exists());
    draft.candidates[0].selected = true;
    let receipt = import(&data, &draft).unwrap();
    assert_eq!(receipt.len(), 1);
    let manifest = Manifest::read(&root.join("apphost.toml")).unwrap();
    assert_eq!(manifest.services.len(), 2);
    assert_eq!(manifest.open_target().unwrap().unwrap().port, 5173);
    assert!(!data.join("runtime").exists());
}
#[test]
fn desktop_review_rejects_invalid_input_and_preserves_identity_and_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    let mut draft = python_without_script(&root);
    let before = serde_json::to_value(&draft).unwrap();
    assert!(review::edit_candidate(&mut draft, 0, "bad toml").is_err());
    let text = explicit_fullstack(&draft);
    let changed_id = text.replace(
        &draft.candidates[0]
            .manifest
            .as_ref()
            .unwrap()
            .project_id
            .to_string(),
        &Uuid::new_v4().to_string(),
    );
    assert!(review::edit_candidate(&mut draft, 0, &changed_id).is_err());
    assert!(
        review::edit_candidate(
            &mut draft,
            0,
            &text.replace("cwd = \"web\"", "cwd = \"../\"")
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&draft).unwrap(), before);
    review::edit_candidate(&mut draft, 0, &text).unwrap();
    draft.candidates[0].selected = true;
    fs::write(root.join("pyproject.toml"), "[project]\nname='changed'\n").unwrap();
    assert_eq!(
        import(&temp.path().join("data"), &draft).unwrap_err().code,
        "STALE_DRAFT"
    );
    assert!(!root.join("apphost.toml").exists());
}
#[test]
fn desktop_review_cannot_replace_a_manifest_that_appeared_after_scan() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    let mut draft = python_without_script(&root);
    let text = explicit_fullstack(&draft);
    fs::write(root.join("apphost.toml"), &text).unwrap();
    assert!(!review::can_edit_candidate(&draft.candidates[0]));
    assert_eq!(
        review::edit_candidate(&mut draft, 0, &text)
            .unwrap_err()
            .code,
        "READ_ONLY_MANIFEST"
    );
    assert_eq!(fs::read_to_string(root.join("apphost.toml")).unwrap(), text);
}
