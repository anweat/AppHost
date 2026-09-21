use apphost::{discovery::ScanOptions, registry::import_with_receipts, *};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
    process::{Command, Stdio},
};
use tempfile::TempDir;

fn node(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"shared-name","scripts":{"dev":"vite"}}"#,
    )
    .unwrap();
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn requirements_and_markerless_backends_never_silently_import_only_the_frontend() {
    for requirements in [false, true] {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("app");
        node(&root.join("frontend"));
        fs::create_dir(root.join("backend")).unwrap();
        if requirements {
            fs::write(root.join("backend/requirements.txt"), "fastapi\nuvicorn\n").unwrap();
        }
        let mut draft = scan(temp.path(), 2).unwrap();
        assert_eq!(draft.candidates.len(), 2);
        assert_eq!(draft.groups.len(), 1);
        assert!(
            draft
                .candidates
                .iter()
                .all(|c| !c.selected && !c.notes.is_empty())
        );
        let data = temp.path().join("data");
        assert!(import(&data, &draft).unwrap().is_empty());
        assert!(!data.exists());
        let services = &draft.groups[0]
            .candidate
            .manifest
            .as_ref()
            .unwrap()
            .services;
        assert!(
            services
                .iter()
                .any(|s| s.cwd == "backend" && s.command.is_empty())
        );
        review::review(
            &mut draft,
            &mut Cursor::new(b"y\n[\"uv\",\"run\",\"uvicorn\",\"main:app\"]\n"),
            &mut Vec::new(),
        )
        .unwrap();
        fs::write(root.join("backend/requirements.txt"), "changed\n").unwrap();
        assert_eq!(import(&data, &draft).unwrap_err().code, "STALE_DRAFT");
        assert!(!root.join("apphost.toml").exists());
    }
}

#[test]
fn root_node_and_python_backend_require_boundary_confirmation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    node(&root);
    fs::create_dir(root.join("backend")).unwrap();
    fs::write(root.join("backend/requirements.txt"), "fastapi\n").unwrap();
    let draft = scan(temp.path(), 2).unwrap();
    assert!(!draft.candidates[0].selected);
    assert_eq!(draft.groups.len(), 1);
    assert!(draft.candidates[0].notes.iter().any(|n| n.field == "group"));
    node(&root.join("docs"));
    let draft = scan(temp.path(), 2).unwrap();
    assert!(draft.groups.is_empty());
    assert!(!draft.candidates[0].selected);
    assert!(
        draft.candidates[0]
            .notes
            .iter()
            .any(|n| n.message.contains("additional services"))
    );
}

#[test]
fn complete_docs_candidate_makes_siblings_a_collection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("collection");
    node(&root.join("frontend"));
    node(&root.join("docs"));
    fs::create_dir(root.join("backend")).unwrap();
    fs::write(root.join("backend/requirements.txt"), "fastapi").unwrap();
    let draft = scan(temp.path(), 2).unwrap();
    assert!(draft.groups.is_empty());
    assert_eq!(draft.candidates.len(), 3);
    assert_eq!(draft.candidates.iter().filter(|c| c.selected).count(), 1);
    for candidate in &draft.candidates {
        if candidate.path.ends_with("docs") {
            assert!(candidate.selected);
            assert!(!candidate.notes.iter().any(|n| n.field == "group"));
        } else {
            assert!(!candidate.selected);
            assert!(candidate.notes.iter().any(|n| n.field == "group"
                && n.message.contains("apphost.toml")
                && n.message.contains("[[services]]")
                && n.message.contains("cwd")));
        }
    }
    let data = temp.path().join("data");
    assert_eq!(import(&data, &draft).unwrap().len(), 1);
    assert!(!root.join("apphost.toml").exists());
    assert!(root.join("docs/apphost.toml").exists());
}

#[test]
fn competing_pairs_and_depth_limits_are_not_resolved_by_guessing() {
    let temp = TempDir::new().unwrap();
    for dir in ["frontend", "web"] {
        node(&temp.path().join(dir));
    }
    for dir in ["backend", "api"] {
        fs::create_dir(temp.path().join(dir)).unwrap();
    }
    let draft = scan(temp.path(), 1).unwrap();
    assert!(draft.groups.is_empty());
    assert!(draft.candidates.iter().all(|c| !c.selected));
    assert!(scan(temp.path(), 0).unwrap().candidates.is_empty());
    assert!(
        scan(&temp.path().join("frontend"), 0)
            .unwrap()
            .groups
            .is_empty()
    );
}

#[test]
fn ambiguous_web_api_server_review_explains_the_manual_multiservice_route() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    for dir in ["web", "api", "server"] {
        node(&root.join(dir));
    }
    let mut draft = scan(temp.path(), 2).unwrap();
    assert!(draft.groups.is_empty());
    assert_eq!(draft.candidates.len(), 3);
    assert!(draft.candidates.iter().all(|c| !c.selected));
    let mut prompts = vec![];
    review::review(&mut draft, &mut Cursor::new(b"n\nn\nn\n"), &mut prompts).unwrap();
    let text = String::from_utf8(prompts).unwrap();
    assert_eq!(text.matches("多个 [[services]] 和各自 cwd").count(), 3);
    assert!(!text.contains("接受合并"));
    assert!(!root.join("apphost.toml").exists());
    // Follow the displayed route: the project author chooses the root and services.
    let mut manifest = draft.candidates[0].manifest.clone().unwrap();
    manifest.services = draft
        .candidates
        .iter()
        .map(|c| {
            let mut service = c.manifest.as_ref().unwrap().services[0].clone();
            service.name = c.path.file_name().unwrap().to_str().unwrap().into();
            service.cwd = service.name.clone();
            service
        })
        .collect();
    fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    let fresh = scan(temp.path(), 2).unwrap();
    assert_eq!(fresh.candidates.len(), 1);
    assert_eq!(
        fresh.candidates[0]
            .manifest
            .as_ref()
            .unwrap()
            .services
            .len(),
        3
    );
    assert_eq!(import(&temp.path().join("data"), &fresh).unwrap().len(), 1);
}

#[test]
fn provenance_records_only_successful_publication_and_keeps_original_digest() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    node(&root);
    let data = temp.path().join("data");
    let draft = scan(&root, 0).unwrap();
    let first = import_with_receipts(&data, &draft).unwrap().remove(0);
    assert_eq!(first.created_files.len(), 1);
    let original = fs::read(root.join("apphost.toml")).unwrap();
    assert_eq!(first.created_files[0].sha256, digest(&original));
    assert_eq!(
        first.installation.created_manifest.as_ref().unwrap().sha256,
        digest(&original)
    );
    let again = import_with_receipts(&data, &draft).unwrap().remove(0);
    assert!(again.created_files.is_empty());
    assert_eq!(
        again.installation.created_manifest.as_ref().unwrap().sha256,
        digest(&original)
    );
    fs::write(
        root.join("apphost.toml"),
        [original.as_slice(), b"\n# user edit\n"].concat(),
    )
    .unwrap();
    let detached = detach(&data, first.installation.installation_id).unwrap();
    let files = detached.retained_files();
    assert_eq!(files.len(), 1);
    assert!(!files[0].matches_created_hash);
    assert!(files[0].current_sha256.is_some());
    assert_eq!(files[0].created_sha256, digest(&original));
    assert!(root.join("apphost.toml").exists());
    fs::remove_file(root.join("apphost.toml")).unwrap();
    assert!(detached.retained_files()[0].current_sha256.is_none());
    assert!(!detached.retained_files()[0].matches_created_hash);
}

#[test]
fn preexisting_manifests_and_legacy_records_have_unknown_creation_origin() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    let manifest = scan(temp.path(), 0)
        .unwrap()
        .candidates
        .remove(0)
        .manifest
        .unwrap();
    fs::write(
        temp.path().join("apphost.toml"),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
    let data = TempDir::new().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(data.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let first = import_with_receipts(data.path(), &scan(temp.path(), 0).unwrap())
        .unwrap()
        .remove(0);
    assert!(first.created_files.is_empty());
    assert!(first.installation.created_manifest.is_none());
    assert!(first.installation.retained_files().is_empty());
    let path = data.path().join("registry.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["installations"][0]
        .as_object_mut()
        .unwrap()
        .remove("created_manifest");
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        list(data.path(), false).unwrap()[0]
            .installation
            .created_manifest
            .is_none()
    );
}

#[test]
fn compact_manifest_round_trips_escaping_order_and_nonempty_environment() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    let mut manifest = scan(temp.path(), 0)
        .unwrap()
        .candidates
        .remove(0)
        .manifest
        .unwrap();
    manifest.services[0].command = vec![
        "server".into(),
        "a b".into(),
        "引号\"和反斜杠\\\nnext".into(),
    ];
    let text = manifest.to_toml().unwrap();
    assert!(!text.contains("[services.env]"));
    assert!(
        text.lines()
            .find(|l| l.starts_with("command ="))
            .unwrap_or_else(|| panic!("Unexpected TOML formatting: {text}"))
            .ends_with(']')
    );
    assert!(text.find("cwd =").unwrap() < text.find("command =").unwrap());
    assert_eq!(toml::from_str::<Manifest>(&text).unwrap(), manifest);
    manifest.services[0]
        .env
        .insert("DISPLAY_NAME".into(), "space and 中文".into());
    let text = manifest.to_toml().unwrap();
    assert!(text.contains("[services.env]"));
    assert_eq!(toml::from_str::<Manifest>(&text).unwrap(), manifest);
}

#[test]
fn launch_notes_are_shown_for_selected_candidates_refreshed_after_edit_and_listed() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    let mut manifest = scan(temp.path(), 0)
        .unwrap()
        .candidates
        .remove(0)
        .manifest
        .unwrap();
    manifest.services[0].command = vec!["missing-apphost-test-command".into()];
    manifest.services[0].env.insert("PATH".into(), "".into());
    fs::write(
        temp.path().join("apphost.toml"),
        manifest.to_toml().unwrap(),
    )
    .unwrap();
    let mut draft = scan(temp.path(), 0).unwrap();
    assert!(draft.candidates[0].selected);
    assert!(
        draft.candidates[0]
            .notes
            .iter()
            .any(|n| n.field.starts_with("launch_check."))
    );
    let mut output = Vec::new();
    review::review(&mut draft, &mut Cursor::new(b""), &mut output).unwrap();
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("missing-apphost-test-command")
    );
    let data = TempDir::new().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(data.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    import(data.path(), &draft).unwrap();
    assert!(
        !list(data.path(), false).unwrap()[0]
            .launch_warnings
            .is_empty()
    );
    // Only inferred manifests can be edited in review; project-owned ones stay read-only.
    let new_app = TempDir::new().unwrap();
    node(new_app.path());
    let mut draft = scan(new_app.path(), 0).unwrap();
    draft.candidates[0].manifest = Some(manifest);
    draft.candidates[0].selected = false;
    review::review(
        &mut draft,
        &mut Cursor::new(b"y\n[\"different-missing-command\"]\n"),
        &mut Vec::new(),
    )
    .unwrap();
    let warnings: Vec<_> = draft.candidates[0]
        .notes
        .iter()
        .filter(|n| n.field.starts_with("launch_check."))
        .collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("different-missing-command"));
    assert!(!warnings[0].message.contains("missing-apphost-test-command"));
}

#[test]
fn copy_hints_supply_ids_but_do_not_change_identity_or_selection() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    node(&a);
    node(&b);
    let data = temp.path().join("data");
    let first = import(&data, &scan(&a, 0).unwrap()).unwrap().remove(0);
    let draft = discovery::scan_with_registry(&b, &ScanOptions::default(), Some(&data)).unwrap();
    let c = &draft.candidates[0];
    assert!(c.selected);
    assert_ne!(c.manifest.as_ref().unwrap().project_id, first.project_id);
    assert!(
        c.notes.iter().any(|n| n.field == "possible_copy"
            && n.message.contains(&first.installation_id.to_string()))
    );
    let same = discovery::scan_with_registry(&a, &ScanOptions::default(), Some(&data)).unwrap();
    assert!(
        !same.candidates[0]
            .notes
            .iter()
            .any(|n| n.field == "possible_copy")
    );
    let options = ScanOptions {
        max_read_bytes: 150,
        ..Default::default()
    };
    let limited = discovery::scan_with_registry(&b, &options, Some(&data)).unwrap();
    assert!(limited.issues.iter().any(|i| i.field == "copy_hints"));
    assert!(
        !limited.candidates[0]
            .notes
            .iter()
            .any(|n| n.field == "possible_copy")
    );
}

#[test]
fn cli_receipts_copy_hints_and_review_warnings_smoke() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    node(&a);
    node(&b);
    let path = temp.path().join("draft.json");
    write_draft(&path, &scan(&a, 0).unwrap()).unwrap();
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
    let receipt = cli(&["import", path.to_str().unwrap(), "--yes"]);
    assert_eq!(receipt[0]["created_files"].as_array().unwrap().len(), 1);
    let id = receipt[0]["installation_id"].as_str().unwrap();
    let draft = cli(&["scan", b.to_str().unwrap()]);
    assert!(
        draft["candidates"][0]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["field"] == "possible_copy")
    );
    let detached = cli(&["detach", id]);
    assert_eq!(detached["retained_files"][0]["matches_created_hash"], true);
    assert!(a.join("apphost.toml").exists());
    let mut draft = scan(&b, 0).unwrap();
    draft.candidates[0].selected = false;
    let draft_path = temp.path().join("review.json");
    write_draft(&draft_path, &draft).unwrap();
    let output_path = temp.path().join("reviewed.json");
    let mut child = Command::new(env!("CARGO_BIN_EXE_apphost"))
        .args([
            "review",
            draft_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
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
        .write_all(b"y\n[\"missing-cli-smoke-executable\"]\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("missing-cli-smoke-executable")
    );
    assert!(result.stdout.is_empty());
    let result: Value = serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert!(
        result["candidates"][0]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["field"].as_str().unwrap().starts_with("launch_check."))
    );
}
