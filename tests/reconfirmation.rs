use apphost::{
    discovery::{self, ScanOptions},
    *,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

fn node(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo","scripts":{"dev":"vite"}}"#,
    )
    .unwrap();
}
fn changed_app(root: &Path, data: &Path) -> registry::Installation {
    node(root);
    let saved = import(data, &scan(root, 0).unwrap()).unwrap().remove(0);
    let mut manifest = validate(root).unwrap();
    manifest.services[0].command[2] = "start".into();
    fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    saved
}
fn aware(root: &Path, data: &Path) -> Draft {
    discovery::scan_with_registry(root, &ScanOptions::default(), Some(data)).unwrap()
}
fn cli(data: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_apphost"))
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
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
fn changed_declarations_need_review_even_when_rescanned_or_manually_selected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    let data = temp.path().join("data");
    let old = changed_app(&root, &data);
    let bytes = fs::read(data.join("registry.json")).unwrap();
    let mut draft = aware(&root, &data);
    let candidate = &draft.candidates[0];
    assert!(!candidate.selected);
    let notes = candidate
        .notes
        .iter()
        .map(|n| n.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(notes.contains("npm run dev"));
    assert!(notes.contains("npm run start"));
    assert!(
        candidate
            .notes
            .iter()
            .any(|n| n.field.starts_with("launch_check.") && n.message.contains("absent"))
    );
    assert!(import(&data, &draft).unwrap().is_empty());
    draft.candidates[0].selected = true;
    assert_eq!(import(&data, &draft).unwrap_err().code, "REVIEW_REQUIRED");
    assert_eq!(
        import(&data, &scan(&root, 0).unwrap()).unwrap_err().code,
        "REVIEW_REQUIRED"
    );
    assert_eq!(fs::read(data.join("registry.json")).unwrap(), bytes);
    draft = aware(&root, &data);
    let mut prompts = vec![];
    review::review(&mut draft, &mut Cursor::new(b"y\n"), &mut prompts).unwrap();
    assert!(String::from_utf8(prompts).unwrap().contains("absent"));
    let current = draft.candidates[0]
        .manifest
        .as_ref()
        .unwrap()
        .launch_sha256();
    assert_eq!(
        draft.candidates[0].reviewed_launch_sha256.as_deref(),
        Some(current.as_str())
    );
    let new = import(&data, &draft).unwrap().remove(0);
    assert_eq!(old.installation_id, new.installation_id);
    assert_ne!(old.confirmed_launch_sha256, new.confirmed_launch_sha256);
    assert_eq!(
        new.confirmed_services.unwrap()[0].command,
        vec!["npm", "run", "start"]
    );
    assert!(!list(&data, false).unwrap()[0].requires_launch_confirmation);
}

#[test]
fn rejection_eof_and_changed_content_do_not_reuse_review_confirmation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    let data = temp.path().join("data");
    changed_app(&root, &data);
    for answers in [b"n\n".as_slice(), b"", b"[\"npm\",\"start\"]\n"] {
        let mut draft = aware(&root, &data);
        let result = review::review(&mut draft, &mut Cursor::new(answers), &mut vec![]);
        assert_eq!(result.is_ok(), answers == b"n\n");
        assert!(!draft.candidates[0].selected);
        assert!(draft.candidates[0].reviewed_launch_sha256.is_none());
        assert!(list(&data, false).unwrap()[0].requires_launch_confirmation);
    }
    let mut reviewed = aware(&root, &data);
    review::review(&mut reviewed, &mut Cursor::new(b"y\n"), &mut vec![]).unwrap();
    let stamp = reviewed.candidates[0].reviewed_launch_sha256.clone();
    reviewed.candidates[0].selected = false;
    review::review(&mut reviewed, &mut Cursor::new(b"n\n"), &mut vec![]).unwrap();
    assert!(reviewed.candidates[0].reviewed_launch_sha256.is_none());
    let mut manifest = validate(&root).unwrap();
    manifest.services[0]
        .env
        .insert("MODE".into(), "changed".into());
    fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    let mut fresh = scan(&root, 0).unwrap();
    fresh.candidates[0].reviewed_launch_sha256 = stamp;
    assert_eq!(import(&data, &fresh).unwrap_err().code, "REVIEW_REQUIRED");
}

#[test]
fn cosmetic_edits_and_first_import_keep_batch_behavior_while_legacy_hashes_are_honest() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    let data = temp.path().join("data");
    node(&root);
    let draft = aware(&root, &data);
    assert!(draft.candidates[0].selected);
    import(&data, &draft).unwrap();
    let mut manifest = validate(&root).unwrap();
    manifest.description = "new description".into();
    fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    let draft = aware(&root, &data);
    assert!(draft.candidates[0].selected);
    import(&data, &draft).unwrap();
    let path = data.join("registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    registry["installations"][0]
        .as_object_mut()
        .unwrap()
        .remove("confirmed_services");
    fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
    manifest.services[0].command[2] = "start".into();
    fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    let draft = aware(&root, &data);
    assert!(!draft.candidates[0].selected);
    assert!(
        draft.candidates[0]
            .notes
            .iter()
            .any(|n| n.message.contains("旧命令未保存"))
    );
    registry["installations"][0]
        .as_object_mut()
        .unwrap()
        .remove("confirmed_launch_sha256");
    fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
    assert!(!aware(&root, &data).candidates[0].selected);
    assert_eq!(
        import(&data, &scan(&root, 0).unwrap()).unwrap_err().code,
        "REVIEW_REQUIRED"
    );
}

#[test]
fn reconfirmation_is_preflighted_before_any_batch_manifest_is_written() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let root = temp.path().join("z-changed");
    changed_app(&root, &data);
    node(&temp.path().join("a-new"));
    let draft = scan(temp.path(), 1).unwrap();
    assert_eq!(import(&data, &draft).unwrap_err().code, "REVIEW_REQUIRED");
    assert!(!temp.path().join("a-new/apphost.toml").exists());
    assert!(list(&data, false).unwrap()[0].requires_launch_confirmation);
}

#[test]
fn cli_rescan_yes_cannot_reconfirm_and_review_refreshes_an_old_selected_draft() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    let data = temp.path().join("data");
    changed_app(&root, &data);
    let path = temp.path().join("draft.json");
    let output = temp.path().join("reviewed.json");
    let draft = value(cli(
        &data,
        &[
            "scan",
            root.to_str().unwrap(),
            "--output",
            path.to_str().unwrap(),
        ],
        b"",
    ));
    assert_eq!(draft["candidates"][0]["selected"], false);
    assert_eq!(
        value(cli(
            &data,
            &["import", path.to_str().unwrap(), "--yes"],
            b""
        )),
        json!([])
    );
    assert!(list(&data, false).unwrap()[0].requires_launch_confirmation);
    // A library-created draft lacks registry annotations and is still selected; CLI review must refresh it.
    fs::write(&path, serde_json::to_vec(&scan(&root, 0).unwrap()).unwrap()).unwrap();
    let bypass = cli(&data, &["import", path.to_str().unwrap(), "--yes"], b"");
    assert!(!bypass.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&bypass.stderr).unwrap()["error"]["code"],
        "REVIEW_REQUIRED"
    );
    let reviewed = cli(
        &data,
        &[
            "review",
            path.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ],
        b"y\n",
    );
    assert!(
        reviewed.status.success(),
        "{}",
        String::from_utf8_lossy(&reviewed.stderr)
    );
    assert!(reviewed.stdout.is_empty());
    let prompts = String::from_utf8(reviewed.stderr).unwrap();
    assert!(prompts.contains("npm run dev"));
    assert!(prompts.contains("npm run start"));
    assert!(prompts.contains("确认这个 App"));
    assert!(!prompts.contains("project_id"));
    assert!(!prompts.contains("schema_version"));
    value(cli(
        &data,
        &["import", output.to_str().unwrap(), "--yes"],
        b"",
    ));
    assert!(!list(&data, false).unwrap()[0].requires_launch_confirmation);
}

#[test]
fn cli_review_preview_is_quiet_and_never_persists_confirmation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    let data = temp.path().join("data");
    node(&root);
    let draft = temp.path().join("draft.json");
    write_draft(&draft, &scan(&root, 0).unwrap()).unwrap();
    let original = fs::read(&draft).unwrap();
    let preview = cli(&data, &["review", draft.to_str().unwrap()], b"");
    assert!(preview.status.success());
    assert!(preview.stdout.is_empty());
    let text = String::from_utf8(preview.stderr).unwrap();
    assert!(text.contains("npm run dev"));
    assert!(text.contains("cwd="));
    assert!(!text.contains("project_id"));
    assert!(!text.contains("evidence"));
    assert_eq!(fs::read(&draft).unwrap(), original);
    assert!(!root.join("apphost.toml").exists());
    assert!(!data.exists());
    changed_app(&root, &data);
    fs::write(
        &draft,
        serde_json::to_vec(&scan(&root, 0).unwrap()).unwrap(),
    )
    .unwrap();
    let before = fs::read(data.join("registry.json")).unwrap();
    let preview = cli(&data, &["review", draft.to_str().unwrap()], b"y\n");
    assert!(preview.status.success());
    assert!(preview.stdout.is_empty());
    assert_eq!(fs::read(data.join("registry.json")).unwrap(), before);
    let cancelled = temp.path().join("cancelled.json");
    let aborted = cli(
        &data,
        &[
            "review",
            draft.to_str().unwrap(),
            "--output",
            cancelled.to_str().unwrap(),
        ],
        b"",
    );
    assert!(!aborted.status.success());
    assert!(!cancelled.exists());
    assert!(aborted.stdout.is_empty());
}

#[test]
fn cli_mixed_batch_confirms_owned_manifest_once_and_only_edits_new_manifest() {
    let temp = TempDir::new().unwrap();
    let projects = temp.path().join("projects");
    let owned = projects.join("a-owned");
    let inferred = projects.join("z-inferred");
    let data = temp.path().join("data");
    node(&owned);
    node(&inferred);
    let mut manifest = scan(&owned, 0)
        .unwrap()
        .candidates
        .remove(0)
        .manifest
        .unwrap();
    manifest.services.push(manifest.services[0].clone());
    manifest.services[1].name = "second".into();
    let original = manifest.to_toml().unwrap();
    fs::write(owned.join("apphost.toml"), &original).unwrap();
    let draft = temp.path().join("draft.json");
    let reviewed = temp.path().join("reviewed.json");
    write_draft(&draft, &scan(&projects, 1).unwrap()).unwrap();
    let output = cli(
        &data,
        &[
            "review",
            draft.to_str().unwrap(),
            "--all",
            "--describe",
            "--output",
            reviewed.to_str().unwrap(),
        ],
        "y\ny\n[\"npm\",\"run\",\"dev\",\"--\",\"--host\"]\n新应用用途\n".as_bytes(),
    );
    let prompts = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{prompts}");
    assert!(output.stdout.is_empty());
    assert!(prompts.contains("如需修改，请直接编辑 apphost.toml 后重新扫描"));
    assert_eq!(prompts.matches("确认这个 App").count(), 2);
    assert_eq!(prompts.matches("或输入参数数组").count(), 1);
    assert_eq!(prompts.matches("这个 App 用来做什么").count(), 1);
    let imported = value(cli(
        &data,
        &["import", reviewed.to_str().unwrap(), "--yes"],
        b"",
    ));
    assert_eq!(imported.as_array().unwrap().len(), 2);
    assert_eq!(
        fs::read_to_string(owned.join("apphost.toml")).unwrap(),
        original
    );
    let actual = validate(&inferred).unwrap();
    assert_eq!(
        actual.services[0].command,
        vec!["npm", "run", "dev", "--", "--host"]
    );
    assert_eq!(actual.description, "新应用用途");
    assert!(
        list(&data, false)
            .unwrap()
            .iter()
            .all(|i| !i.requires_launch_confirmation)
    );
}

#[test]
fn declining_changed_manifest_leaves_it_unconfirmed_and_imports_other_selected_apps() {
    let temp = TempDir::new().unwrap();
    let projects = temp.path().join("projects");
    let owned = projects.join("a-owned");
    let data = temp.path().join("data");
    changed_app(&owned, &data);
    let original = fs::read(owned.join("apphost.toml")).unwrap();
    node(&projects.join("z-new"));
    let mut draft = aware(&projects, &data);
    let mut prompts = vec![];
    review::review(&mut draft, &mut Cursor::new(b"n\n"), &mut prompts).unwrap();
    assert!(
        !String::from_utf8(prompts)
            .unwrap()
            .contains("或输入参数数组")
    );
    let imported = import(&data, &draft).unwrap();
    assert_eq!(imported.len(), 1);
    assert!(imported[0].path.ends_with("z-new"));
    assert_eq!(fs::read(owned.join("apphost.toml")).unwrap(), original);
    assert!(
        list(&data, false)
            .unwrap()
            .iter()
            .find(|i| i.installation.path.ends_with("a-owned"))
            .unwrap()
            .requires_launch_confirmation
    );
}

#[test]
fn manifest_published_after_scan_is_also_read_only_during_review() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    let mut draft = scan(temp.path(), 0).unwrap();
    let data = TempDir::new().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(data.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    import(data.path(), &draft).unwrap();
    let original = fs::read(temp.path().join("apphost.toml")).unwrap();
    draft.candidates[0].selected = false;
    let mut prompts = vec![];
    review::review_with_descriptions(&mut draft, &mut Cursor::new(b"y\n"), &mut prompts, true)
        .unwrap();
    let prompts = String::from_utf8(prompts).unwrap();
    assert!(!prompts.contains("或输入参数数组"));
    assert!(!prompts.contains("这个 App 用来做什么"));
    assert_eq!(import(data.path(), &draft).unwrap().len(), 1);
    assert_eq!(
        fs::read(temp.path().join("apphost.toml")).unwrap(),
        original
    );
}
