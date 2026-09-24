#![cfg(unix)]
use apphost::{
    Manifest,
    journal::Actor,
    registry::editing::{self, SaveRequest},
    runtime::LaunchPlan,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use uuid::Uuid;

struct Fixture {
    _temp: tempfile::TempDir,
    data: PathBuf,
    root: PathBuf,
    id: Uuid,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let root = temp.path().join("app");
        fs::create_dir(&root).unwrap();
        let manifest: Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture","services":[{"name":"worker","command":["/bin/sleep","30"]}]})).unwrap();
        fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id =
            apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        Self {
            _temp: temp,
            data,
            root,
            id,
        }
    }
    fn edit(&self) -> SaveRequest {
        let before = editing::read(&self.data, self.id).unwrap();
        let text = format!(
            "# Preserve this comment and formatting\n{}",
            before["text"].as_str().unwrap().replace("\"30\"", "\"31\"")
        );
        let review = editing::review(&self.data, self.id, Some(&text)).unwrap();
        SaveRequest {
            revision: review["revision"].as_u64().unwrap(),
            base_sha256: review["base_sha256"].as_str().unwrap().into(),
            confirmed_launch_sha256: review["launch_sha256"].as_str().unwrap().into(),
            text: Some(text),
        }
    }
}
#[test]
fn reviewed_save_preserves_text_records_actor_and_invalidates_queued_plan() {
    let f = Fixture::new();
    let plan = LaunchPlan::prepare(&f.data, f.id).unwrap();
    let request = f.edit();
    let result = editing::save(&f.data, f.id, &request, Actor::Agent).unwrap();
    assert_eq!(
        fs::read_to_string(f.root.join("apphost.toml")).unwrap(),
        request.text.unwrap()
    );
    assert_eq!(result["installation"]["confirmed_actor"], "agent");
    assert!(result["installation"]["confirmed_at_ms"].is_u64());
    assert_eq!(
        plan.with_current(&f.data, |_| Ok(())).unwrap_err().code,
        "STALE_LAUNCH"
    );
    assert_eq!(
        LaunchPlan::prepare(&f.data, f.id)
            .unwrap()
            .context()
            .confirmed_actor,
        Some(Actor::Agent)
    );
}
#[test]
fn stale_buffer_revision_and_review_digest_never_overwrite_source() {
    let f = Fixture::new();
    let request = f.edit();
    let path = f.root.join("apphost.toml");
    let original = fs::read_to_string(&path).unwrap();
    let mut bad = request.clone();
    bad.confirmed_launch_sha256 = "0".repeat(64);
    assert_eq!(
        editing::save(&f.data, f.id, &bad, Actor::Human)
            .unwrap_err()
            .code,
        "REVIEW_REQUIRED"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    fs::write(&path, format!("# outside edit\n{original}")).unwrap();
    assert_eq!(
        editing::save(&f.data, f.id, &request, Actor::Human)
            .unwrap_err()
            .code,
        "EDIT_CONFLICT"
    );
    fs::write(&path, &original).unwrap();
    apphost::label(&f.data, f.id, "renamed").unwrap();
    assert_eq!(
        editing::save(&f.data, f.id, &request, Actor::Human)
            .unwrap_err()
            .code,
        "EDIT_CONFLICT"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
}
#[test]
fn external_edits_can_be_confirmed_without_rewriting_or_fake_changes() {
    let f = Fixture::new();
    let request = f.edit();
    let path = f.root.join("apphost.toml");
    fs::write(&path, request.text.unwrap()).unwrap();
    assert_eq!(
        LaunchPlan::prepare(&f.data, f.id).err().unwrap().code,
        "REVIEW_REQUIRED"
    );
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let review = editing::review(&f.data, f.id, None).unwrap();
    let result = editing::save(
        &f.data,
        f.id,
        &SaveRequest {
            revision: review["revision"].as_u64().unwrap(),
            base_sha256: review["base_sha256"].as_str().unwrap().into(),
            confirmed_launch_sha256: review["launch_sha256"].as_str().unwrap().into(),
            text: None,
        },
        Actor::Human,
    )
    .unwrap();
    assert_eq!(result["file_written"], false);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    assert!(LaunchPlan::prepare(&f.data, f.id).is_ok());
}
#[test]
fn confirmation_publish_failure_retains_saved_file_without_trusting_new_command() {
    let f = Fixture::new();
    let path = f.data.join("registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    registry["installations"][0]["revision"] = json!(u64::MAX);
    fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
    let request = f.edit();
    let error = editing::save(&f.data, f.id, &request, Actor::Human).unwrap_err();
    assert_eq!(error.code, "MANIFEST_SAVED_UNCONFIRMED");
    assert_eq!(
        fs::read_to_string(f.root.join("apphost.toml")).unwrap(),
        request.text.unwrap()
    );
    assert_eq!(
        LaunchPlan::prepare(&f.data, f.id).err().unwrap().code,
        "REVIEW_REQUIRED"
    );
}
#[test]
fn invalid_identity_cwd_and_oversized_buffer_have_no_side_effects() {
    let f = Fixture::new();
    let request = f.edit();
    let text = request.text.unwrap();
    for proposed in [
        text.replace(
            &apphost::registry::registered(&f.data, f.id)
                .unwrap()
                .project_id
                .to_string(),
            &Uuid::new_v4().to_string(),
        ),
        format!("{}\ncwd = '../escape'\n", text),
        " ".repeat(1024 * 1024 + 1),
    ] {
        assert!(editing::review(&f.data, f.id, Some(&proposed)).is_err());
    }
    assert!(LaunchPlan::prepare(&f.data, f.id).is_ok());
}

#[test]
fn cli_read_review_and_save_keep_json_channels_and_confirmation_attribution() {
    let f = Fixture::new();
    let request = f.edit();
    let buffer = f._temp.path().join("buffer.toml");
    fs::write(&buffer, request.text.as_ref().unwrap()).unwrap();
    let run = |args: Vec<String>| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_apphost"))
            .arg("--data-dir")
            .arg(&f.data)
            .args(args)
            .env("APPHOST_ACTOR", "agent")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let read = run(vec!["manifest".into(), "read".into(), f.id.to_string()]);
    assert_eq!(read["base_sha256"], request.base_sha256);
    let review = run(vec![
        "manifest".into(),
        "review".into(),
        f.id.to_string(),
        "--file".into(),
        buffer.display().to_string(),
    ]);
    let result = run(vec![
        "manifest".into(),
        "save".into(),
        f.id.to_string(),
        "--file".into(),
        buffer.display().to_string(),
        "--base-sha256".into(),
        request.base_sha256,
        "--revision".into(),
        request.revision.to_string(),
        "--confirm".into(),
        review["launch_sha256"].as_str().unwrap().into(),
    ]);
    assert_eq!(result["confirmed"], true);
    assert_eq!(result["installation"]["confirmed_actor"], "agent");
    assert!(!f.data.join("runtime").exists());
    assert!(!f.data.join("daemon.sock").exists());
}
