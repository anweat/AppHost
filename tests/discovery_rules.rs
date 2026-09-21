use apphost::{
    Result,
    discovery::{ScanOptions, scan_with_options, scan_with_rules},
    manifest::{Manifest, Service},
    rules::{BUILTIN_RULES, DiscoveryRule, Proposal, RuleContext},
};
use std::{collections::BTreeMap, fs, path::Path};
use tempfile::TempDir;
use uuid::Uuid;

fn node(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(
        path.join("package.json"),
        r#"{"name":"app","scripts":{"dev":"vite"}}"#,
    )
    .unwrap();
}
struct CustomRule {
    id: &'static str,
    invalid_command: bool,
}
impl DiscoveryRule for CustomRule {
    fn id(&self) -> &str {
        self.id
    }
    fn markers(&self) -> &[&str] {
        &["demo.config"]
    }
    fn evaluate(&self, context: &RuleContext<'_>) -> Result<Proposal> {
        assert_eq!(context.file("demo.config"), Some(&b"serve"[..]));
        assert!(context.file("secret.txt").is_none());
        Ok(Proposal {
            name: context.name().into(),
            description: "Community rule example".into(),
            requires_confirmation: false,
            services: vec![Service {
                name: "server".into(),
                command: if self.invalid_command {
                    vec![]
                } else {
                    vec!["demo-server".into()]
                },
                cwd: ".".into(),
                port: None,
                env: BTreeMap::new(),
            }],
        })
    }
}

#[test]
fn injected_rule_supports_scan_import_and_stale_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("demo.config"), "serve").unwrap();
    fs::write(root.join("secret.txt"), "never read").unwrap();
    let rule = CustomRule {
        id: "community-demo",
        invalid_command: false,
    };
    let draft = scan_with_rules(&root, &ScanOptions::default(), &[&rule]).unwrap();
    assert!(draft.candidates[0].selected);
    assert_eq!(draft.candidates[0].rule_inputs, vec!["demo.config"]);
    assert!(
        draft.candidates[0]
            .suggestions
            .contains_key("community-demo")
    );
    let data = temp.path().join("registry");
    apphost::import(&data, &draft).unwrap();
    assert_eq!(
        apphost::validate(&root).unwrap().services[0].command,
        vec!["demo-server"]
    );
    fs::write(root.join("demo.config"), "changed").unwrap();
    assert_eq!(
        apphost::import(&data, &draft).unwrap_err().code,
        "STALE_DRAFT"
    );
}

#[test]
fn conflicting_rules_do_not_use_registration_order_as_priority() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("demo.config"), "serve").unwrap();
    let a = CustomRule {
        id: "a",
        invalid_command: false,
    };
    let b = CustomRule {
        id: "b",
        invalid_command: false,
    };
    for rules in [
        [&a as &dyn DiscoveryRule, &b as &dyn DiscoveryRule],
        [&b, &a],
    ] {
        let draft = scan_with_rules(temp.path(), &ScanOptions::default(), &rules).unwrap();
        let c = &draft.candidates[0];
        assert!(!c.selected);
        assert_eq!(c.suggestions.len(), 2);
        assert!(c.manifest.as_ref().unwrap().services.is_empty());
        assert!(
            c.issues
                .iter()
                .any(|i| i.message.contains("Multiple discovery"))
        );
    }
}

#[test]
fn core_validates_rule_output_and_manifest_remains_authoritative() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("demo.config"), "serve").unwrap();
    let rule = CustomRule {
        id: "invalid",
        invalid_command: true,
    };
    let draft = scan_with_rules(temp.path(), &ScanOptions::default(), &[&rule]).unwrap();
    assert!(!draft.candidates[0].selected);
    assert!(
        draft.candidates[0]
            .issues
            .iter()
            .any(|i| i.field.ends_with("command"))
    );
    let mut m = draft.candidates[0].manifest.clone().unwrap();
    m.services[0].command = vec!["explicit".into()];
    fs::write(
        temp.path().join("apphost.toml"),
        toml::to_string(&m).unwrap(),
    )
    .unwrap();
    let draft = scan_with_rules(temp.path(), &ScanOptions::default(), &[&rule]).unwrap();
    assert!(draft.candidates[0].selected);
    assert!(draft.candidates[0].suggestions.is_empty());
    assert_eq!(
        draft.candidates[0].manifest.as_ref().unwrap().services[0].command,
        vec!["explicit"]
    );
}

struct BadInput;
impl DiscoveryRule for BadInput {
    fn id(&self) -> &str {
        "bad"
    }
    fn markers(&self) -> &[&str] {
        &["../outside"]
    }
    fn evaluate(&self, _: &RuleContext<'_>) -> Result<Proposal> {
        panic!("Invalid rules must be rejected before execution")
    }
}
#[test]
fn rule_registration_rejects_duplicate_ids_and_out_of_scope_inputs() {
    let temp = TempDir::new().unwrap();
    let rule = CustomRule {
        id: "duplicate",
        invalid_command: false,
    };
    assert_eq!(
        scan_with_rules(temp.path(), &ScanOptions::default(), &[&BadInput])
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        scan_with_rules(temp.path(), &ScanOptions::default(), &[&rule, &rule])
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    for name in [
        ".env",
        ".env.local",
        "app/config.toml",
        "../file",
        "C:\\file",
    ] {
        assert!(!apphost::rules::valid_input_name(name));
    }
}

#[test]
fn python_entry_is_suggested_but_requires_explicit_confirmation() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("pyproject.toml"),
        "[project]\nname='client'\n[project.scripts]\nfetch='client:main'",
    )
    .unwrap();
    let draft = apphost::scan(temp.path(), 0).unwrap();
    let c = &draft.candidates[0];
    assert!(!c.selected);
    assert_eq!(
        c.manifest.as_ref().unwrap().services[0].command,
        vec!["fetch"]
    );
    assert!(!c.notes.is_empty());
    node(temp.path());
    let draft = apphost::scan(temp.path(), 0).unwrap();
    assert!(!draft.candidates[0].selected);
    assert_eq!(draft.candidates[0].suggestions.len(), 2);
}

#[test]
fn nested_projects_are_hints_with_depth_and_count_caps() {
    let temp = TempDir::new().unwrap();
    node(temp.path());
    node(&temp.path().join("server"));
    node(&temp.path().join("group/web"));
    node(&temp.path().join("deep/a/b"));
    node(&temp.path().join("node_modules/ignore"));
    let draft = apphost::scan(temp.path(), 3).unwrap();
    let c = &draft.candidates[0];
    assert_eq!(draft.candidates.len(), 1);
    assert_eq!(c.nested_projects.len(), 2);
    assert!(!c.hints_truncated);
    assert!(
        apphost::scan(temp.path(), 0).unwrap().candidates[0]
            .nested_projects
            .is_empty()
    );
    for i in 0..40 {
        node(&temp.path().join(format!("p{i}")));
    }
    let c = apphost::scan(temp.path(), 3).unwrap().candidates.remove(0);
    assert_eq!(c.nested_projects.len(), 32);
    assert!(c.hints_truncated);
}

#[test]
fn directory_entry_read_and_exclusion_budgets_are_enforced() {
    let temp = TempDir::new().unwrap();
    node(&temp.path().join("include"));
    node(&temp.path().join("exclude"));
    let opts = ScanOptions {
        exclude: vec!["exclude".into()],
        ..Default::default()
    };
    let draft = scan_with_options(temp.path(), &opts).unwrap();
    assert_eq!(draft.candidates.len(), 1);
    assert!(draft.candidates[0].path.ends_with("include"));
    for options in [
        ScanOptions {
            max_directories: 1,
            ..Default::default()
        },
        ScanOptions {
            max_entries: 1,
            ..Default::default()
        },
        ScanOptions {
            max_read_bytes: 1,
            ..Default::default()
        },
    ] {
        assert_eq!(
            scan_with_options(temp.path(), &options).unwrap_err().code,
            "SCAN_LIMIT"
        );
    }
    for options in [
        ScanOptions {
            max_depth: 17,
            ..Default::default()
        },
        ScanOptions {
            max_directories: 0,
            ..Default::default()
        },
        ScanOptions {
            max_read_bytes: 0,
            ..Default::default()
        },
        ScanOptions {
            exclude: vec!["../escape".into()],
            ..Default::default()
        },
    ] {
        assert_eq!(
            scan_with_options(temp.path(), &options).unwrap_err().code,
            "INVALID_INPUT"
        );
    }
}

#[test]
fn cli_respects_scope_and_emits_no_partial_draft_on_budget_failure() {
    let temp = TempDir::new().unwrap();
    node(&temp.path().join("include"));
    node(&temp.path().join("exclude"));
    let success = std::process::Command::new(env!("CARGO_BIN_EXE_apphost"))
        .arg("scan")
        .arg(temp.path())
        .args([
            "--exclude",
            "exclude",
            "--max-directories",
            "2",
            "--max-depth",
            "1",
        ])
        .output()
        .unwrap();
    assert!(success.status.success());
    let draft: apphost::discovery::Draft = serde_json::from_slice(&success.stdout).unwrap();
    assert_eq!(draft.candidates.len(), 1);
    assert!(draft.candidates[0].path.ends_with("include"));
    assert_eq!(draft.scope.max_directories, 2);
    assert_eq!(draft.scope.exclude, vec!["exclude"]);

    for flag in ["--max-directories", "--max-entries", "--max-read-bytes"] {
        let failed = std::process::Command::new(env!("CARGO_BIN_EXE_apphost"))
            .arg("scan")
            .arg(temp.path())
            .args([flag, "1"])
            .output()
            .unwrap();
        assert_eq!(failed.status.code(), Some(1));
        assert!(failed.stdout.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&failed.stderr).unwrap();
        assert_eq!(error["error"]["code"], "SCAN_LIMIT");
    }
    assert!(!temp.path().join("include/apphost.toml").exists());
}

#[test]
fn lockfiles_above_one_mib_work_but_are_bounded_and_fingerprinted() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    node(&root);
    let lock = root.join("pnpm-lock.yaml");
    fs::write(&lock, vec![b'x'; 2 * 1024 * 1024]).unwrap();
    let draft = apphost::scan(&root, 0).unwrap();
    assert!(draft.candidates[0].selected);
    let data = temp.path().join("registry");
    apphost::import(&data, &draft).unwrap();
    fs::write(&lock, vec![b'y'; 2 * 1024 * 1024]).unwrap();
    assert_eq!(
        apphost::import(&data, &draft).unwrap_err().code,
        "STALE_DRAFT"
    );
    fs::remove_file(root.join("apphost.toml")).unwrap();
    fs::File::create(&lock)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    let c = apphost::scan(&root, 0).unwrap().candidates.remove(0);
    assert!(!c.selected);
    assert!(c.issues.iter().any(|i| i.message.contains("16 MiB")));
}

#[test]
fn authoritative_manifest_does_not_expand_nested_hints_or_rules() {
    let temp = TempDir::new().unwrap();
    node(&temp.path().join("nested"));
    let m = Manifest {
        schema_version: 0,
        project_id: Uuid::new_v4(),
        name: "explicit".into(),
        description: String::new(),
        services: vec![Service {
            name: "app".into(),
            command: vec!["server".into()],
            cwd: ".".into(),
            port: None,
            env: BTreeMap::new(),
        }],
    };
    fs::write(
        temp.path().join("apphost.toml"),
        toml::to_string(&m).unwrap(),
    )
    .unwrap();
    let c = scan_with_rules(temp.path(), &ScanOptions::default(), BUILTIN_RULES)
        .unwrap()
        .candidates
        .remove(0);
    assert!(c.selected);
    assert!(c.nested_projects.is_empty());
}
