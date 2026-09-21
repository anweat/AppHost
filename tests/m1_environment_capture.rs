#![cfg(target_os = "macos")]
use apphost::environment::{self, SYSTEM_PATH};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn shell(root: &Path, body: &str) -> PathBuf {
    let path = root.join("fixture-shell");
    // This controlled non-login /bin/sh fixture accepts the capture command as
    // $3. It never reads the real user's .zprofile, .bash_profile or other scripts.
    fs::write(&path, format!("#!/bin/sh\n/bin/sleep 0.05\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[test]
fn explicit_capture_filters_noise_records_provenance_and_manual_save_clears_it() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let shell = shell(
        temp.path(),
        "PATH=/usr/bin:/bin\nprintf 'startup noise\\n'\nprintf 'stderr noise\\n' >&2\n/bin/sh -c \"$3\"\nprintf 'logout noise\\n'",
    );
    let captured = environment::capture_path(&data, &shell).unwrap();
    assert_eq!(captured.path, "/usr/bin:/bin");
    let record = captured.capture.unwrap();
    assert_eq!(record.shell, shell);
    assert!(record.captured_at_ms > 0);
    assert_eq!(environment::load(&data).unwrap().path, "/usr/bin:/bin");
    environment::save_path(&data, SYSTEM_PATH.into()).unwrap();
    assert!(environment::load(&data).unwrap().capture.is_none());
}

#[test]
fn invalid_output_exit_and_output_flood_leave_settings_intact_without_leaking_output() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    environment::save_path(&data, SYSTEM_PATH.into()).unwrap();
    let original = fs::read(data.join("environment.json")).unwrap();
    for (body, expected) in [
        (
            "printf 'private-output-do-not-leak' >&2\nexit 4",
            "ENV_CAPTURE_FAILED",
        ),
        ("printf 'no marker'", "ENV_CAPTURE_FAILED"),
        (
            "PATH=/bin::/usr/bin /bin/sh -c \"$3\"",
            "ENVIRONMENT_INVALID",
        ),
        (
            "/usr/bin/yes private-output-do-not-leak >&2",
            "ENV_CAPTURE_LIMIT",
        ),
    ] {
        let executable = shell(temp.path(), body);
        let error = environment::capture_path(&data, &executable).unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.to_string().contains("private-output-do-not-leak"));
        assert_eq!(fs::read(data.join("environment.json")).unwrap(), original);
    }
}

#[test]
fn timeout_is_bounded_and_terminates_verified_shell_group() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let executable = shell(temp.path(), "/bin/sleep 20");
    let started = Instant::now();
    let error = environment::capture_path(&data, &executable).unwrap_err();
    assert_eq!(error.code, "ENV_CAPTURE_TIMEOUT");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!data.exists());
}

#[test]
fn config_changed_during_capture_is_not_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    environment::save_path(&data, SYSTEM_PATH.into()).unwrap();
    let ready = temp.path().join("ready");
    let quoted = format!("'{}'", ready.to_str().unwrap().replace('\'', "'\\''"));
    let executable = shell(
        temp.path(),
        &format!("printf ready > {quoted}\n/bin/sleep 0.3\n/bin/sh -c \"$3\""),
    );
    let capture_data = data.clone();
    let worker = thread::spawn(move || environment::capture_path(&capture_data, &executable));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready.exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    environment::save_path(&data, "/bin".into()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err().code, "CONFLICT");
    assert_eq!(environment::load(&data).unwrap().path, "/bin");
}

#[test]
fn cli_requires_explicit_execution_flag_and_captures_only_when_requested() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let executable = shell(temp.path(), "/bin/sh -c \"$3\"");
    let cli = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_apphost"));
        command
            .arg("--data-dir")
            .arg(&data)
            .args(["environment", "capture", "--shell"])
            .arg(&executable);
        command
    };
    assert_eq!(cli().output().unwrap().status.code(), Some(2));
    assert!(!data.exists());
    let output = cli().arg("--yes").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(saved["path"], SYSTEM_PATH);
    assert!(saved["capture"]["captured_at_ms"].as_u64().unwrap() > 0);
}
