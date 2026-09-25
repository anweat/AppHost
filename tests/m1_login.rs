#![cfg(target_os = "macos")]
use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn cli_login_configuration_is_explicit_json_and_does_not_start_daemon() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    let call = |command: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_apphost"))
            .env("HOME", t.path())
            .env_remove("APPHOST_ACTOR")
            .arg("--data-dir")
            .arg(&data)
            .args(["login", command])
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
    assert_eq!(call("show")["enabled"], false);
    assert!(!data.exists());
    let enabled = call("enable");
    assert_eq!(enabled["enabled"], true);
    assert_eq!(enabled["activation"], "next_login");
    assert!(!data.join("daemon.json").exists());
    assert!(!data.join("runtime").exists());
    let path = enabled["path"].as_str().unwrap();
    assert!(fs::read_to_string(path).unwrap().contains("daemon"));
    assert_eq!(call("disable")["enabled"], false);
    assert!(!std::path::Path::new(path).exists());
    let facts = apphost::journal::read(&data, None, 100).unwrap();
    assert_eq!(facts["events"].as_array().unwrap().len(), 2);
    assert_eq!(facts["events"][0]["action"], "login-enable");
    assert_eq!(facts["events"][1]["action"], "login-disable");
}
