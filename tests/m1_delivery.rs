use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use uuid::Uuid;
fn cli(data: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_apphost"))
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .env_remove("APPHOST_ACTOR")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn shipped_manifest_examples_are_valid_single_and_multi_service_declarations() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("web")).unwrap();
    let guide = apphost::agent::guide();
    let examples: Vec<apphost::Manifest> = guide["manifest"]
        .as_str()
        .unwrap()
        .split("```toml\n")
        .skip(1)
        .map(|block| toml::from_str(block.split("```").next().unwrap()).unwrap())
        .collect();
    assert_eq!(examples.len(), 2);
    for manifest in &examples {
        assert!(manifest.validate(temp.path()).is_empty());
    }
    assert_eq!(examples[0].services.len(), 1);
    assert_eq!(examples[1].services.len(), 2);
    assert_eq!(examples[1].open.as_ref().unwrap().service, "web");
}
#[test]
fn embedded_guide_safe_inventory_and_retention_work_without_daemon() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let guide = cli(&data, &["agent-guide"]);
    assert_eq!(guide["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        guide["troubleshoot"]
            .as_str()
            .unwrap()
            .contains("list --redacted")
    );
    assert!(!data.exists());
    assert_eq!(cli(&data, &["log-retention", "show"])["days"], 7);
    assert!(!data.exists());
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let manifest: apphost::Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"private-label","services":[{"name":"service","command":["unused","private-env-value"],"env":{"EXAMPLE":"private-env-value"}}]})).unwrap();
    fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    let id = apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
    let old = cli(&data, &["list"]);
    assert!(old.to_string().contains("private-env-value"));
    let safe = cli(&data, &["list", "--redacted"]);
    assert_eq!(safe[0]["installation_id"], json!(id));
    assert!(!safe.to_string().contains("private-env-value"));
    assert!(!safe.to_string().contains("private-label"));
    assert!(cli(&data, &["icon", &id.to_string()]).is_null());
    assert_eq!(
        cli(
            &data,
            &["log-retention", "set", "--days", "3", "--max-mib", "32"]
        )["days"],
        3
    );
    assert_eq!(cli(&data, &["log-retention", "show"])["max_mib"], 32);
    assert!(!data.join("daemon.sock").exists());
}
