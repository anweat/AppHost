#![cfg(target_os = "macos")]
use apphost::process::{Group, observe_exit, spawn_group};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Managed {
    child: Child,
    group: Option<Group>,
}
impl Managed {
    fn spawn(command: &mut Command) -> Self {
        let child = spawn_group(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit()),
        )
        .unwrap();
        let mut owned = Self { child, group: None };
        owned.group = Some(Group::capture(&mut owned.child).unwrap());
        owned
    }
}
impl Drop for Managed {
    fn drop(&mut self) {
        if let Some(group) = &mut self.group {
            let _ = group.stop(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < end, "fixture timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

fn fixture_command(mode: &str, root: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "process_fixture", "--nocapture"])
        .env("APPHOST_PROCESS_FIXTURE", mode)
        .env("APPHOST_PROCESS_ROOT", root);
    command
}

#[test]
fn process_fixture() {
    let Ok(mode) = std::env::var("APPHOST_PROCESS_FIXTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os("APPHOST_PROCESS_ROOT").unwrap());
    if mode == "leader" {
        let mut leaf = fixture_command("leaf", &root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        fs::write(root.join("leaf-pid"), leaf.id().to_string()).unwrap();
        wait_for(|| root.join("release-leader").exists());
        // Deliberately leave this child in the original group after leader exit.
        let _ = leaf.try_wait();
    } else {
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
        }
        fs::write(root.join("leaf-ready"), b"ready").unwrap();
        // A hard fixture deadline prevents a failed assertion leaving long-lived work.
        thread::sleep(Duration::from_secs(15));
    }
}

#[test]
fn independent_group_stop_is_verified_and_idempotent() {
    let mut owned = Managed::spawn(Command::new("/bin/sleep").arg("30"));
    let group = owned.group.as_mut().unwrap();
    assert_eq!(group.id(), owned.child.id());
    let live = group.refresh().unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].pid, owned.child.id());
    assert!(!group.stop(Duration::from_secs(1)).unwrap());
    owned.child.wait().unwrap();
    assert!(group.refresh().unwrap().is_empty());
    assert!(!group.stop(Duration::from_secs(1)).unwrap());
}

#[test]
fn known_descendant_survives_leader_exit_and_is_stopped_with_bounded_escalation() {
    let temp = tempfile::tempdir().unwrap();
    let mut owned = Managed::spawn(&mut fixture_command("leader", temp.path()));
    wait_for(|| temp.path().join("leaf-ready").exists());
    let group = owned.group.as_mut().unwrap();
    let members = group.refresh().unwrap();
    assert_eq!(members.len(), 2);
    let leaf: u32 = fs::read_to_string(temp.path().join("leaf-pid"))
        .unwrap()
        .parse()
        .unwrap();
    fs::write(temp.path().join("release-leader"), b"exit").unwrap();
    wait_for(|| observe_exit(&owned.child).unwrap().is_some());
    // Repeated observation must not consume the wait status or release the PID.
    assert!(observe_exit(&owned.child).unwrap().unwrap().success());
    let members = group.refresh().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].pid, leaf);
    assert!(group.stop(Duration::from_millis(50)).unwrap());
    assert!(group.refresh().unwrap().is_empty());
    assert!(owned.child.wait().unwrap().success());
}

#[test]
fn unknown_survivor_is_not_adopted_or_signalled() {
    let temp = tempfile::tempdir().unwrap();
    let mut owned = Managed::spawn(&mut fixture_command("leader", temp.path()));
    let mut unobserved = Group::capture(&mut owned.child).unwrap();
    wait_for(|| temp.path().join("leaf-ready").exists());
    // A separate verified handle is retained exclusively for fixture cleanup.
    owned.group.as_mut().unwrap().refresh().unwrap();
    fs::write(temp.path().join("release-leader"), b"exit").unwrap();
    wait_for(|| observe_exit(&owned.child).unwrap().is_some());
    let error = unobserved.stop(Duration::from_millis(50)).unwrap_err();
    assert_eq!(error.code, "MANAGEMENT_UNCERTAIN");
    assert_eq!(owned.group.as_mut().unwrap().refresh().unwrap().len(), 1);
}

#[test]
fn ordinary_child_in_supervisor_group_cannot_be_adopted() {
    let child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    let mut owned = Managed { child, group: None };
    assert_eq!(
        Group::capture(&mut owned.child).unwrap_err().code,
        "MANAGEMENT_UNCERTAIN"
    );
}

#[test]
fn invalid_grace_does_not_signal_and_finished_child_cannot_be_recaptured() {
    let mut owned = Managed::spawn(Command::new("/bin/sleep").arg("10"));
    for duration in [Duration::ZERO, Duration::from_secs(61)] {
        assert_eq!(
            owned
                .group
                .as_mut()
                .unwrap()
                .stop(duration)
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );
        assert!(owned.child.try_wait().unwrap().is_none());
    }
    owned
        .group
        .as_mut()
        .unwrap()
        .stop(Duration::from_millis(50))
        .unwrap();
    owned.child.wait().unwrap();
    assert_eq!(
        Group::capture(&mut owned.child).unwrap_err().code,
        "MANAGEMENT_UNCERTAIN"
    );
}
