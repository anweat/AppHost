use apphost::{
    discovery::{self, ScanOptions},
    rules::{self, grouping::GroupingRule},
    *,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, io::Cursor, path::Path};
use tempfile::TempDir;

fn node(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo","scripts":{"dev":"vite"}}"#,
    )
    .unwrap();
}
fn python(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("requirements.txt"), "fastapi\nuvicorn").unwrap();
}

#[test]
fn projects_roots_and_nested_collections_keep_independent_apps_selected() {
    for nested in [false, true] {
        for empty_server in [false, true] {
            let temp = TempDir::new().unwrap();
            let collection = if nested {
                temp.path().join("Projects")
            } else {
                temp.path().to_owned()
            };
            for name in if empty_server {
                vec!["todo", "web"]
            } else {
                vec!["todo", "notes", "blog", "web"]
            } {
                node(&collection.join(name));
            }
            if empty_server {
                fs::create_dir(collection.join("server")).unwrap();
            } else {
                python(&collection.join("api"));
            }
            let mut draft = scan(temp.path(), 2).unwrap();
            assert!(draft.groups.is_empty());
            assert_eq!(draft.candidates.len(), if empty_server { 2 } else { 5 });
            assert_eq!(
                draft.candidates.iter().filter(|c| c.selected).count(),
                if empty_server { 1 } else { 3 }
            );
            let mut output = vec![];
            review::review(&mut draft, &mut Cursor::new(b"n\nn\n"), &mut output).unwrap();
            let prompts = String::from_utf8(output).unwrap();
            assert_eq!(
                prompts.matches("确认这个 App").count(),
                1 + usize::from(!empty_server)
            );
            assert!(!prompts.contains("接受合并"));
            let data = temp.path().join("registry");
            let imported = import(&data, &draft).unwrap();
            assert_eq!(imported.len(), if empty_server { 1 } else { 3 });
            assert!(!collection.join("apphost.toml").exists());
        }
    }
}

#[test]
fn accepting_pair_retains_unrelated_candidates_and_preflights_overlap() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    node(&root.join("frontend"));
    python(&root.join("backend"));
    python(&root.join("docs"));
    node(&temp.path().join("unrelated"));
    let mut draft = scan(temp.path(), 2).unwrap();
    assert_eq!(draft.groups.len(), 1);
    assert!(
        draft
            .candidates
            .iter()
            .find(|c| c.path.ends_with("unrelated"))
            .unwrap()
            .selected
    );
    let docs = draft
        .candidates
        .iter()
        .find(|c| c.path.ends_with("docs"))
        .unwrap()
        .clone();
    discovery::accept_group(&mut draft, 0).unwrap();
    assert_eq!(draft.candidates.len(), 3);
    let retained = draft
        .candidates
        .iter()
        .find(|c| c.path == docs.path)
        .unwrap();
    assert_eq!(
        serde_json::to_value(retained).unwrap(),
        serde_json::to_value(docs).unwrap()
    );
    for c in &mut draft.candidates {
        let m = c.manifest.as_mut().unwrap();
        if m.services.is_empty() {
            m.services = draft_service();
        }
        for service in &mut m.services {
            if service.command.is_empty() {
                service.command = vec!["server".into()];
            }
        }
        c.selected = true;
    }
    let data = temp.path().join("data");
    assert_eq!(import(&data, &draft).unwrap_err().code, "CONFLICT");
    assert!(!root.join("apphost.toml").exists());
}

#[test]
fn runnable_grandchild_also_prevents_a_collection_from_becoming_one_app() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("Projects");
    node(&root.join("web"));
    python(&root.join("api"));
    node(&root.join("tools/widget"));
    let draft = scan(temp.path(), 3).unwrap();
    assert!(draft.groups.is_empty());
    assert_eq!(draft.candidates.iter().filter(|c| c.selected).count(), 1);
}
fn draft_service() -> Vec<manifest::Service> {
    vec![manifest::Service {
        name: "app".into(),
        cwd: ".".into(),
        command: vec!["server".into()],
        port: None,
        env: Default::default(),
    }]
}

struct Community;
impl GroupingRule for Community {
    fn is_service_directory(&self, name: &str) -> bool {
        ["ui", "worker"].contains(&name)
    }
    fn sibling_pair(&self, names: &BTreeSet<String>) -> Option<[String; 2]> {
        ["ui", "worker"]
            .iter()
            .all(|n| names.contains(*n))
            .then(|| ["ui".into(), "worker".into()])
    }
}
struct InvalidPair;
impl GroupingRule for InvalidPair {
    fn is_service_directory(&self, _: &str) -> bool {
        false
    }
    fn sibling_pair(&self, _: &BTreeSet<String>) -> Option<[String; 2]> {
        Some(["ui".into(), "../escape".into()])
    }
}
#[test]
fn community_grouping_is_replaceable_without_bypassing_core_boundaries() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    node(&root.join("ui"));
    python(&root.join("worker"));
    let opts = ScanOptions::default();
    assert!(scan(temp.path(), 3).unwrap().groups.is_empty());
    let custom = |path: &Path| {
        discovery::scan_with_policies(path, &opts, rules::BUILTIN_RULES, &Community).unwrap()
    };
    assert_eq!(custom(temp.path()).groups.len(), 1);
    assert!(custom(&root).groups.is_empty());
    assert_eq!(
        discovery::scan_with_policies(temp.path(), &opts, rules::BUILTIN_RULES, &InvalidPair)
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    node(&root.join("another"));
    assert!(custom(temp.path()).groups.is_empty());
    fs::remove_dir_all(root.join("another")).unwrap();
    import(
        &temp.path().join("data"),
        &scan(&root.join("ui"), 0).unwrap(),
    )
    .unwrap();
    assert!(custom(temp.path()).groups.is_empty());
}

#[test]
fn edited_group_cannot_use_scan_root_or_consume_outside_members() {
    let temp = TempDir::new().unwrap();
    node(&temp.path().join("app/frontend"));
    python(&temp.path().join("app/backend"));
    node(&temp.path().join("other"));
    let original = scan(temp.path(), 3).unwrap();
    for outside in [false, true] {
        let mut draft = original.clone();
        if outside {
            draft.groups[0].members.push(draft.root.join("other"));
        } else {
            draft.groups[0].candidate.path = draft.root.clone();
        }
        let before = serde_json::to_value(&draft.candidates).unwrap();
        assert_eq!(
            discovery::accept_group(&mut draft, 0).unwrap_err().code,
            "CONFLICT"
        );
        assert_eq!(serde_json::to_value(&draft.candidates).unwrap(), before);
    }
}

#[test]
fn all_manifest_origins_record_confirmation_and_only_reimport_refreshes_it() {
    for preexisting in [false, true] {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("app");
        node(&root);
        let data = temp.path().join("data");
        if preexisting {
            let m = scan(&root, 0)
                .unwrap()
                .candidates
                .remove(0)
                .manifest
                .unwrap();
            fs::write(root.join("apphost.toml"), m.to_toml().unwrap()).unwrap();
        }
        let first = import(&data, &scan(&root, 0).unwrap()).unwrap().remove(0);
        assert_eq!(first.created_manifest.is_none(), preexisting);
        let original = first.confirmed_launch_sha256.clone();
        let mut m = validate(&root).unwrap();
        m.name = "Budget".into();
        m.description = "记账工具".into();
        fs::write(
            root.join("apphost.toml"),
            format!("# cosmetic edit\n{}", m.to_toml().unwrap()),
        )
        .unwrap();
        let listing = list(&data, false).unwrap().remove(0);
        assert_eq!(listing.launch_changed, Some(false));
        assert!(!listing.requires_launch_confirmation);
        assert_eq!(listing.manifest.as_ref().unwrap().description, "记账工具");
        assert_eq!(
            listing.manifest.as_ref().unwrap().services[0].command,
            vec!["npm", "run", "dev"]
        );
        m.services[0].command[2] = "start".into();
        fs::write(root.join("apphost.toml"), m.to_toml().unwrap()).unwrap();
        for _ in 0..2 {
            let listing = list(&data, false).unwrap().remove(0);
            assert_eq!(listing.launch_changed, Some(true));
            assert!(listing.requires_launch_confirmation);
            assert_eq!(listing.installation.confirmed_launch_sha256, original);
        }
        label(&data, first.installation_id, "Named").unwrap();
        assert!(list(&data, false).unwrap()[0].requires_launch_confirmation);
        let mut fresh =
            discovery::scan_with_registry(&root, &ScanOptions::default(), Some(&data)).unwrap();
        assert!(!fresh.candidates[0].selected);
        review::review(&mut fresh, &mut Cursor::new(b"y\n"), &mut vec![]).unwrap();
        let confirmed = import(&data, &fresh).unwrap().remove(0);
        assert_ne!(confirmed.confirmed_launch_sha256, original);
        assert_eq!(confirmed.installation_id, first.installation_id);
        assert!(!list(&data, false).unwrap()[0].requires_launch_confirmation);
        fs::write(root.join("apphost.toml"), "invalid").unwrap();
        let listing = list(&data, false).unwrap().remove(0);
        assert!(listing.requires_launch_confirmation);
        assert_eq!(listing.launch_changed, None);
        assert!(listing.manifest_error.is_some());
    }
}

#[test]
fn confirmation_digest_covers_execution_fields_but_not_cosmetics() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    let m = scan(temp.path(), 0)
        .unwrap()
        .candidates
        .remove(0)
        .manifest
        .unwrap();
    let original = m.launch_sha256();
    for field in [
        "command",
        "cwd",
        "port",
        "env",
        "name",
        "project_id",
        "order",
    ] {
        let mut changed = m.clone();
        match field {
            "command" => changed.services[0].command.push("--open".into()),
            "cwd" => changed.services[0].cwd = "backend".into(),
            "port" => changed.services[0].port = Some(9000),
            "env" => {
                changed.services[0].env.insert("MODE".into(), "dev".into());
            }
            "name" => changed.services[0].name = "backend".into(),
            "project_id" => changed.project_id = uuid::Uuid::new_v4(),
            _ => changed.services.extend(draft_service()),
        }
        assert_ne!(changed.launch_sha256(), original, "{field}");
    }
    let mut changed = m.clone();
    changed.description = "new".into();
    changed.name = "Name".into();
    assert_eq!(changed.launch_sha256(), original);
}

#[test]
fn legacy_confirmation_is_unknown_and_branch_switch_never_stales_auto_label() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    node(&root);
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature-x\n").unwrap();
    let data = temp.path().join("data");
    let first = import(&data, &scan(&root, 0).unwrap()).unwrap().remove(0);
    assert_eq!(first.human_label, "app");
    assert!(!first.label_is_custom);
    assert_eq!(
        list(&data, false).unwrap()[0].display_label,
        "app · feature-x"
    );
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    assert_eq!(list(&data, false).unwrap()[0].display_label, "app · main");
    label(&data, first.installation_id, "My App").unwrap();
    let listing = list(&data, false).unwrap().remove(0);
    assert_eq!(listing.display_label, "My App");
    assert_eq!(listing.current_branch.as_deref(), Some("main"));
    let path = data.join("registry.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let obj = value["installations"][0].as_object_mut().unwrap();
    obj.remove("confirmed_launch_sha256");
    obj.remove("confirmed_services");
    obj.remove("label_is_custom");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let listing = list(&data, false).unwrap().remove(0);
    assert!(listing.requires_launch_confirmation);
    assert_eq!(listing.launch_changed, None);
    assert_eq!(listing.display_label, "My App");
    value["installations"][0]["confirmed_launch_sha256"] = json!("bad");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(list(&data, false).unwrap_err().code, "REGISTRY_INVALID");
}

#[test]
fn optional_description_is_skippable_and_never_overwrites_existing_manifest() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    for response in ["\n", "记账工具\n"] {
        let mut draft = scan(temp.path(), 0).unwrap();
        review::review_with_descriptions(&mut draft, &mut Cursor::new(response), &mut vec![], true)
            .unwrap();
        assert_eq!(
            draft.candidates[0].manifest.as_ref().unwrap().description,
            response.trim()
        );
    }
    let data = temp.path().join("data");
    import(&data, &scan(temp.path(), 0).unwrap()).unwrap();
    let mut draft = scan(temp.path(), 0).unwrap();
    review::review_with_descriptions(&mut draft, &mut Cursor::new(b""), &mut vec![], true).unwrap();
    assert!(
        draft.candidates[0]
            .manifest
            .as_ref()
            .unwrap()
            .description
            .is_empty()
    );
}

#[test]
fn cli_description_confirmation_and_live_branch_smoke() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    node(&root);
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature\n").unwrap();
    let data = temp.path().join("data");
    let cli = |args: &[&str], input: &[u8]| -> Value {
        let mut child = Command::new(env!("CARGO_BIN_EXE_apphost"))
            .arg("--data-dir")
            .arg(&data)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if args[0] == "review" {
            assert!(result.stdout.is_empty());
            Value::Null
        } else {
            serde_json::from_slice(&result.stdout).unwrap()
        }
    };
    let draft = temp.path().join("draft.json");
    let reviewed = temp.path().join("reviewed.json");
    cli(
        &[
            "scan",
            root.to_str().unwrap(),
            "--output",
            draft.to_str().unwrap(),
        ],
        b"",
    );
    cli(
        &[
            "review",
            draft.to_str().unwrap(),
            "--describe",
            "--output",
            reviewed.to_str().unwrap(),
        ],
        "记账工具\n".as_bytes(),
    );
    assert!(!root.join("apphost.toml").exists());
    let imported = cli(&["import", reviewed.to_str().unwrap(), "--yes"], b"");
    assert!(imported[0]["confirmed_launch_sha256"].is_string());
    let listed = cli(&["list"], b"");
    assert_eq!(listed[0]["manifest"]["description"], "记账工具");
    assert_eq!(
        listed[0]["manifest"]["services"][0]["command"],
        json!(["npm", "run", "dev"])
    );
    assert_eq!(listed[0]["display_label"], "app · feature");
    assert_eq!(listed[0]["requires_launch_confirmation"], false);
    let mut m = validate(&root).unwrap();
    m.services[0].command = vec!["new-command".into()];
    fs::write(root.join("apphost.toml"), m.to_toml().unwrap()).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let listed = cli(&["list"], b"");
    assert_eq!(listed[0]["display_label"], "app · main");
    assert_eq!(listed[0]["launch_changed"], true);
    assert_eq!(listed[0]["requires_launch_confirmation"], true);
}
