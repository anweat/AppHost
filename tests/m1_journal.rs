use apphost::{Manifest, journal};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use uuid::Uuid;

fn command(data: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_apphost"));
    c.arg("--data-dir").arg(data).env_remove("APPHOST_ACTOR");
    c
}

#[cfg(target_os = "macos")]
#[test]
fn invalid_actor_cannot_bootstrap_a_daemon_or_create_storage() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("absent");
    let result = command(&data)
        .env("APPHOST_ACTOR", "admin")
        .arg("resume-wake")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stderr).unwrap()["error"]["code"],
        "INVALID_ACTOR"
    );
    assert!(!data.exists());
}

#[test]
fn offline_journal_is_read_only_and_label_records_actor_without_copying_values() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let empty = command(&data).args(["journal", "--json"]).output().unwrap();
    assert!(empty.status.success());
    assert!(empty.stderr.is_empty());
    assert!(!data.exists());
    let root = temp.path().join("app");
    fs::create_dir(&root).unwrap();
    let manifest: Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture","services":[{"name":"service","command":["unused"]}]})).unwrap();
    fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    let id = apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
    let labeled = command(&data)
        .env("APPHOST_ACTOR", "agent")
        .args(["label", &id.to_string(), "private-label-value"])
        .output()
        .unwrap();
    assert!(labeled.status.success(), "{:?}", labeled);
    assert!(labeled.stderr.is_empty());
    let page = journal::read(&data, None, 10).unwrap();
    assert_eq!(page["events"][0]["actor"], "agent");
    assert_eq!(page["events"][0]["installation_id"], json!(id));
    assert_eq!(page["events"][0]["outcome"], "succeeded");
    assert!(!page.to_string().contains("private-label-value"));
    assert!(!data.join("daemon.sock").exists());

    let registry = fs::read(data.join("registry.json")).unwrap();
    let bad_actor = command(&data)
        .env("APPHOST_ACTOR", "admin")
        .args(["label", &id.to_string(), "must-not-change"])
        .output()
        .unwrap();
    assert!(!bad_actor.status.success());
    assert!(bad_actor.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&bad_actor.stderr).unwrap()["error"]["code"],
        "INVALID_ACTOR"
    );
    assert_eq!(fs::read(data.join("registry.json")).unwrap(), registry);

    fs::write(data.join("journal/0.jsonl"), b"incomplete").unwrap();
    let warned = command(&data)
        .args(["label", &id.to_string(), "changed-successfully"])
        .output()
        .unwrap();
    assert!(warned.status.success());
    assert!(warned.stderr.is_empty());
    let receipt: Value = serde_json::from_slice(&warned.stdout).unwrap();
    assert_eq!(receipt["warnings"][0]["code"], "JOURNAL_WRITE_FAILED");
    assert_ne!(fs::read(data.join("registry.json")).unwrap(), registry);
    let broken = command(&data).arg("journal").output().unwrap();
    assert!(!broken.status.success());
    assert!(broken.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&broken.stderr).unwrap()["error"]["code"],
        "JOURNAL_INVALID"
    );
}
