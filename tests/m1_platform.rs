#![cfg(not(target_os = "macos"))]
#[test]
fn runtime_stubs_fail_explicitly_and_never_spawn() {
    use apphost::{environment, process};
    use std::process::Command;
    assert_eq!(
        environment::Baseline::system().unwrap_err().code,
        "UNSUPPORTED_PLATFORM"
    );
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        apphost::daemon::serve(temp.path()).unwrap_err().code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(
        apphost::daemon::ping(temp.path()).unwrap_err().code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(
        apphost::daemon::check_launch(temp.path(), uuid::Uuid::new_v4())
            .unwrap_err()
            .code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(
        environment::load(temp.path()).unwrap_err().code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(
        environment::capture_path(temp.path(), std::path::Path::new("missing-shell"))
            .unwrap_err()
            .code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(
        process::spawn_group(&mut Command::new("missing-program"))
            .unwrap_err()
            .code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(
        apphost::login::show(temp.path()).unwrap_err().code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(
        apphost::login::set(temp.path(), true).unwrap_err().code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(
        apphost::daemon::watch_condition(temp.path(), serde_json::json!({}), |_| Ok(true))
            .unwrap_err()
            .code,
        "UNSUPPORTED_PLATFORM"
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
