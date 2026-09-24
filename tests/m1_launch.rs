#![cfg(target_os = "macos")]
use apphost::{Manifest, registry, runtime::LaunchPlan};
use std::{fs, path::PathBuf, process::Command};
use tempfile::TempDir;
use uuid::Uuid;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    data: PathBuf,
    id: Uuid,
    manifest: Manifest,
}
impl Fixture {
    fn new(version: u32) -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        let data = temp.path().join("data");
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "schema_version":version,"project_id":Uuid::new_v4(),"name":"Fixture",
            "services":[{"name":"service","command":["/bin/sh","-c","printf launched > launched"],
                "env":{"PRIVATE_TOKEN":"sensitive-fixture-value"}}]
        }))
        .unwrap();
        fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id =
            apphost::import(&data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        Self {
            _temp: temp,
            root,
            data,
            id,
            manifest,
        }
    }
    fn write(&self, manifest: &Manifest) {
        fs::write(self.root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    }
    fn error(&self) -> String {
        LaunchPlan::prepare(&self.data, self.id).err().unwrap().code
    }
}

#[test]
fn legacy_and_v1_preparation_is_read_only_and_summary_omits_sensitive_values() {
    for version in [0, 1] {
        let fixture = Fixture::new(version);
        if version == 0 {
            let path = fixture.data.join("registry.json");
            let mut legacy: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            legacy["schema_version"] = 0.into();
            for key in [
                "revision",
                "confirmed_root",
                "confirmed_manifest",
                "confirmed_actor",
                "confirmed_at_ms",
            ] {
                legacy["installations"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
            }
            fs::write(path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        }
        let original = fs::read(fixture.data.join("registry.json")).unwrap();
        let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
        let summary = serde_json::to_value(plan.summary()).unwrap();
        assert_eq!(summary["manifest_version"], version);
        assert_eq!(summary["installation_id"], fixture.id.to_string());
        assert_eq!(summary["services"][0]["executable"], "/bin/sh");
        let text = summary.to_string();
        assert!(!text.contains("sensitive-fixture-value"));
        assert!(!text.contains("PRIVATE_TOKEN"));
        assert!(!text.contains("printf launched"));
        assert!(!fixture.root.join("launched").exists());
        assert_eq!(
            fs::read(fixture.data.join("registry.json")).unwrap(),
            original
        );
        assert!(!fixture.data.join("environment.json").exists());
        assert!(!fixture.data.join("instances").exists());
    }
}

#[test]
fn missing_detached_and_unconfirmed_installations_cannot_prepare() {
    let fixture = Fixture::new(1);
    assert_eq!(
        registry::launch_snapshot(&fixture.data, Uuid::new_v4())
            .err()
            .unwrap()
            .code,
        "NOT_FOUND"
    );
    let path = fixture.data.join("registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for field in [
        "confirmed_launch_sha256",
        "confirmed_services",
        "confirmed_manifest",
        "confirmed_root",
    ] {
        registry["installations"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
    }
    fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
    assert_eq!(fixture.error(), "REVIEW_REQUIRED");
    apphost::detach(&fixture.data, fixture.id).unwrap();
    assert_eq!(fixture.error(), "DETACHED");
}

#[test]
fn changed_declaration_never_executes_and_cosmetic_changes_keep_confirmation() {
    let fixture = Fixture::new(1);
    let mut manifest = fixture.manifest.clone();
    manifest.name = "New visible name".into();
    manifest.description = "New description".into();
    fixture.write(&manifest);
    LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    for kind in 0..5 {
        let mut changed = manifest.clone();
        match kind {
            0 => changed.services[0].command.push("new argument".into()),
            1 => {
                changed.services[0].env.insert("NEW".into(), "value".into());
            }
            2 => changed.services[0].port = Some(54321),
            3 => changed.services[0].startup_timeout_ms = Some(45000),
            _ => changed.services[0].stop_timeout_ms = Some(7000),
        }
        fixture.write(&changed);
        assert_eq!(fixture.error(), "REVIEW_REQUIRED");
    }
    assert!(!fixture.root.join("launched").exists());
}

#[test]
fn missing_source_manifest_and_changed_identity_never_use_historical_commands() {
    let fixture = Fixture::new(1);
    fs::remove_file(fixture.root.join("apphost.toml")).unwrap();
    assert_eq!(fixture.error(), "NOT_FOUND");
    let mut changed = fixture.manifest.clone();
    changed.project_id = Uuid::new_v4();
    fixture.write(&changed);
    assert_eq!(fixture.error(), "CONFLICT");
    fs::remove_dir_all(&fixture.root).unwrap();
    assert_eq!(fixture.error(), "SOURCE_MISSING");
}

#[test]
fn invalid_later_service_prevents_preparation_of_the_entire_app() {
    let fixture = Fixture::new(1);
    let mut changed = fixture.manifest.clone();
    let mut later = changed.services[0].clone();
    later.name = "later".into();
    later.command = vec!["/missing-apphost-executable".into()];
    changed.services.push(later);
    fixture.write(&changed);
    let mut draft = apphost::scan(&fixture.root, 0).unwrap();
    registry::annotate_launch_changes(&fixture.data, &mut draft).unwrap();
    apphost::review::review(&mut draft, &mut "y\n".as_bytes(), &mut Vec::new()).unwrap();
    apphost::import(&fixture.data, &draft).unwrap();
    assert_eq!(fixture.error(), "INVALID_COMMAND");
    assert!(!fixture.root.join("launched").exists());
}

#[test]
fn cli_registered_check_requires_one_target_and_never_launches() {
    let fixture = Fixture::new(1);
    let cli = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_apphost"));
        command
            .arg("--data-dir")
            .arg(&fixture.data)
            .args(["environment", "check"]);
        command
    };
    assert_eq!(cli().output().unwrap().status.code(), Some(2));
    assert_eq!(
        cli()
            .arg(&fixture.root)
            .args(["--installation", &fixture.id.to_string()])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let output = cli()
        .args(["--installation", &fixture.id.to_string()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["installation_id"], fixture.id.to_string());
    assert!(!fixture.root.join("launched").exists());
}

#[test]
fn revision_is_local_to_an_installation_and_actual_spawn_runs_under_recheck_lock() {
    let fixture = Fixture::new(1);
    let other_root = fixture._temp.path().join("other");
    fs::create_dir(&other_root).unwrap();
    let mut other = fixture.manifest.clone();
    other.project_id = Uuid::new_v4();
    fs::write(other_root.join("apphost.toml"), other.to_toml().unwrap()).unwrap();
    let other_id = apphost::import(&fixture.data, &apphost::scan(&other_root, 0).unwrap()).unwrap()
        [0]
    .installation_id;
    let before = registry::launch_snapshot(&fixture.data, fixture.id)
        .unwrap()
        .installation()
        .revision;
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    apphost::label(&fixture.data, other_id, "other changed").unwrap();
    assert_eq!(
        registry::launch_snapshot(&fixture.data, fixture.id)
            .unwrap()
            .installation()
            .revision,
        before
    );
    let mut child = plan
        .with_current(&fixture.data, |mut services| {
            assert_eq!(
                apphost::label(&fixture.data, fixture.id, "must wait")
                    .unwrap_err()
                    .code,
                "BUSY"
            );
            let mut command = services.remove(0).into_command();
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            apphost::process::spawn_group(&mut command)
        })
        .unwrap();
    // Waiting is deliberately outside the registration lock.
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read(fixture.root.join("launched")).unwrap(),
        b"launched"
    );
    apphost::label(&fixture.data, fixture.id, "now allowed").unwrap();
}

#[test]
fn own_updates_and_detach_reattach_cannot_reuse_a_queued_snapshot() {
    let fixture = Fixture::new(1);
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    apphost::label(&fixture.data, fixture.id, "updated label").unwrap();
    assert_eq!(
        plan.with_current::<()>(&fixture.data, |_| panic!("stale plan dispatched"))
            .unwrap_err()
            .code,
        "STALE_LAUNCH"
    );
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    let before = registry::launch_snapshot(&fixture.data, fixture.id)
        .unwrap()
        .installation()
        .revision;
    apphost::detach(&fixture.data, fixture.id).unwrap();
    apphost::import(&fixture.data, &apphost::scan(&fixture.root, 0).unwrap()).unwrap();
    let after = registry::launch_snapshot(&fixture.data, fixture.id)
        .unwrap()
        .installation()
        .revision;
    assert_eq!(after, before + 2);
    assert_eq!(
        plan.with_current::<()>(&fixture.data, |_| panic!("old registration reused"))
            .unwrap_err()
            .code,
        "STALE_LAUNCH"
    );
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    let imported =
        apphost::import(&fixture.data, &apphost::scan(&fixture.root, 0).unwrap()).unwrap();
    assert_eq!(imported[0].revision, after);
    plan.with_current(&fixture.data, |_| Ok(())).unwrap();
}

#[test]
fn post_preparation_declaration_change_and_relink_cannot_dispatch() {
    let fixture = Fixture::new(1);
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    let mut changed = fixture.manifest.clone();
    changed.services[0].command.push("changed".into());
    fixture.write(&changed);
    assert_eq!(
        plan.with_current::<()>(&fixture.data, |_| panic!("changed declaration executed"))
            .unwrap_err()
            .code,
        "REVIEW_REQUIRED"
    );
    fixture.write(&fixture.manifest);
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    let moved = fixture._temp.path().join("moved");
    fs::rename(&fixture.root, &moved).unwrap();
    apphost::relink(&fixture.data, fixture.id, &moved).unwrap();
    assert_eq!(
        plan.with_current::<()>(&fixture.data, |_| panic!("old root executed"))
            .unwrap_err()
            .code,
        "REVIEW_REQUIRED"
    );
}

#[test]
fn snapshot_cannot_cross_data_directories_and_callback_failure_releases_lock() {
    let fixture = Fixture::new(1);
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    let other = fixture._temp.path().join("other-data");
    fs::create_dir(&other).unwrap();
    fs::copy(
        fixture.data.join("registry.json"),
        other.join("registry.json"),
    )
    .unwrap();
    assert_eq!(
        plan.with_current::<()>(&other, |_| panic!("cross-directory snapshot"))
            .unwrap_err()
            .code,
        "CONFLICT"
    );
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    let error = plan
        .with_current::<()>(&fixture.data, |_| {
            Err(apphost::Error::new(
                "PROCESS_ERROR",
                "fixture spawn failure",
            ))
        })
        .unwrap_err();
    assert_eq!(error.code, "PROCESS_ERROR");
    apphost::label(&fixture.data, fixture.id, "lock released").unwrap();
    assert!(!fixture.root.join("launched").exists());
}

#[test]
fn cwd_symlink_change_is_detected_even_when_it_stays_inside_the_project() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(1);
    fs::create_dir(fixture.root.join("a")).unwrap();
    fs::create_dir(fixture.root.join("b")).unwrap();
    symlink("a", fixture.root.join("work")).unwrap();
    let mut changed = fixture.manifest.clone();
    changed.services[0].cwd = "work".into();
    fixture.write(&changed);
    let mut draft = apphost::scan(&fixture.root, 0).unwrap();
    registry::annotate_launch_changes(&fixture.data, &mut draft).unwrap();
    apphost::review::review(&mut draft, &mut "y\n".as_bytes(), &mut Vec::new()).unwrap();
    apphost::import(&fixture.data, &draft).unwrap();
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    fs::remove_file(fixture.root.join("work")).unwrap();
    symlink("b", fixture.root.join("work")).unwrap();
    assert_eq!(
        plan.with_current::<()>(&fixture.data, |_| panic!("changed cwd executed"))
            .unwrap_err()
            .code,
        "STALE_LAUNCH"
    );
}

#[test]
fn unchanged_legacy_v1_record_is_not_invalidated_by_another_installation() {
    let fixture = Fixture::new(1);
    let other_root = fixture._temp.path().join("other");
    fs::create_dir(&other_root).unwrap();
    fs::write(
        other_root.join("apphost.toml"),
        fixture.manifest.to_toml().unwrap(),
    )
    .unwrap();
    let other_id = apphost::import(&fixture.data, &apphost::scan(&other_root, 0).unwrap()).unwrap()
        [0]
    .installation_id;
    let path = fixture.data.join("registry.json");
    let mut saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    saved["installations"][0]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let plan = LaunchPlan::prepare(&fixture.data, fixture.id).unwrap();
    apphost::label(&fixture.data, other_id, "other changed").unwrap();
    assert_eq!(
        registry::launch_snapshot(&fixture.data, fixture.id)
            .unwrap()
            .installation()
            .revision,
        0
    );
    plan.with_current(&fixture.data, |_| Ok(())).unwrap();
    assert_eq!(
        apphost::label(&fixture.data, fixture.id, "initialize this revision")
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn launch_lock_wait_is_bounded_and_rechecks_after_the_writer_finishes() {
    use fs2::FileExt;
    use std::time::{Duration, Instant};
    let f = Fixture::new(1);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.data.join("registry.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let plan = LaunchPlan::prepare(&f.data, f.id).unwrap();
    let start = Instant::now();
    assert_eq!(
        plan.with_current::<()>(&f.data, |_| panic!("must not dispatch under a busy lock"))
            .unwrap_err()
            .code,
        "BUSY"
    );
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert!(start.elapsed() < Duration::from_secs(4));
    let plan = LaunchPlan::prepare(&f.data, f.id).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(150));
            let path = f.data.join("registry.json");
            let mut registry: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            let revision = registry["installations"][0]["revision"].as_u64().unwrap();
            registry["installations"][0]["revision"] = serde_json::json!(revision + 1);
            fs::write(path, serde_json::to_vec(&registry).unwrap()).unwrap();
            FileExt::unlock(&lock).unwrap();
        });
        assert_eq!(
            plan.with_current::<()>(&f.data, |_| panic!("must reject the queued stale snapshot"))
                .unwrap_err()
                .code,
            "STALE_LAUNCH"
        );
    });
}
