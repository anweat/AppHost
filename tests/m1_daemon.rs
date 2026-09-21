#![cfg(target_os = "macos")]
use apphost::daemon;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt, symlink},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

fn cli(data: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apphost"));
    command.arg("--data-dir").arg(data).arg("daemon");
    command
}
struct Daemon {
    child: Child,
    data: PathBuf,
    hello: Value,
}
impl Daemon {
    fn start(data: &Path) -> Self {
        let mut child = cli(data)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let hello = loop {
            if let Ok(hello) = daemon::ping(data) {
                break hello;
            }
            if child.try_wait().unwrap().is_some() {
                let mut error = String::new();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut error)
                    .unwrap();
                panic!("daemon startup failed: {error}");
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("daemon startup timed out");
            }
            thread::sleep(Duration::from_millis(10));
        };
        Self {
            child,
            data: data.to_owned(),
            hello,
        }
    }
    fn socket(&self) -> PathBuf {
        self.data.join("daemon.sock")
    }
    fn crash(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn temp() -> TempDir {
    tempfile::Builder::new()
        .prefix("ah-rpc-")
        .tempdir_in("/tmp")
        .unwrap()
}
fn private(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn exchange(socket: &Path, bytes: &[u8]) -> Value {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    stream.write_all(bytes).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    serde_json::from_slice(&response).unwrap()
}

#[test]
fn offline_queries_do_not_create_files_or_start_a_daemon() {
    let temp = temp();
    let data = temp.path().join("absent");
    assert_eq!(daemon::ping(&data).unwrap_err().code, "DAEMON_UNAVAILABLE");
    assert_eq!(
        daemon::check_launch(&data, uuid::Uuid::new_v4())
            .unwrap_err()
            .code,
        "DAEMON_UNAVAILABLE"
    );
    assert!(!data.exists());
    let output = cli(&data).arg("ping").output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "DAEMON_UNAVAILABLE"
    );
    assert!(!data.exists());
}

#[test]
fn singleton_uses_canonical_data_directory_and_reclaims_only_stale_socket() {
    let temp = temp();
    let data = temp.path().join("data");
    let mut daemon = Daemon::start(&data);
    assert_eq!(daemon.hello["pid"], daemon.child.id());
    assert_eq!(fs::metadata(daemon.socket()).unwrap().mode() & 0o777, 0o600);
    let inode = fs::metadata(daemon.socket()).unwrap().ino();
    let alias = temp.path().join("alias");
    symlink(&data, &alias).unwrap();
    let output = cli(&alias).arg("serve").output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "DAEMON_ALREADY_RUNNING"
    );
    assert_eq!(fs::metadata(daemon.socket()).unwrap().ino(), inode);
    assert_eq!(
        daemon::ping(&alias).unwrap()["daemon_id"],
        daemon.hello["daemon_id"]
    );
    daemon.crash();
    assert!(daemon.socket().exists());
    assert_eq!(daemon::ping(&data).unwrap_err().code, "DAEMON_UNAVAILABLE");
    let restarted = Daemon::start(&data);
    assert_ne!(restarted.hello["daemon_id"], daemon.hello["daemon_id"]);
    let second = Daemon::start(&temp.path().join("second"));
    assert_ne!(second.hello["daemon_id"], restarted.hello["daemon_id"]);
}

#[test]
fn occupied_paths_and_shared_directory_are_preserved() {
    let temp = temp();
    for kind in ["file", "symlink", "listener"] {
        let data = temp.path().join(kind);
        private(&data);
        let path = data.join("daemon.sock");
        let listener = match kind {
            "file" => {
                fs::write(&path, b"keep").unwrap();
                None
            }
            "symlink" => {
                symlink(temp.path().join("missing"), &path).unwrap();
                None
            }
            _ => Some(UnixListener::bind(&path).unwrap()),
        };
        let before = fs::symlink_metadata(&path).unwrap();
        let output = cli(&data).arg("serve").output().unwrap();
        assert!(!output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
            "CONFLICT"
        );
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), before.ino());
        drop(listener);
    }
    let data = temp.path().join("shared");
    private(&data);
    fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        daemon::serve(&data).unwrap_err().code,
        "DATA_DIR_PERMISSIONS"
    );
    assert_eq!(fs::metadata(&data).unwrap().mode() & 0o777, 0o755);
    assert_eq!(fs::read_dir(&data).unwrap().count(), 0);
}

#[test]
fn malformed_versions_params_and_large_frames_never_dispatch() {
    let temp = temp();
    let daemon = Daemon::start(&temp.path().join("data"));
    let mut request = json!({"version":2,"request_id":"test","method":"check-launch","params":{}});
    let send = |value: &Value| {
        let mut bytes = serde_json::to_vec(value).unwrap();
        bytes.push(b'\n');
        exchange(&daemon.socket(), &bytes)
    };
    let response = send(&request);
    assert_eq!(response["request_id"], "test");
    assert_eq!(response["error"]["code"], "PROTOCOL_VERSION_UNSUPPORTED");
    request["version"] = 1.into();
    request["method"] = "unavailable-method".into();
    assert_eq!(send(&request)["error"]["code"], "METHOD_NOT_FOUND");
    request["method"] = "hello".into();
    request["params"] = json!({"unexpected":true});
    assert_eq!(send(&request)["error"]["code"], "INVALID_PARAMS");
    assert_eq!(
        exchange(&daemon.socket(), b"{bad\n")["error"]["code"],
        "INVALID_REQUEST"
    );
    assert_eq!(
        exchange(&daemon.socket(), &vec![b'x'; 65537])["error"]["code"],
        "MESSAGE_TOO_LARGE"
    );
    assert!(daemon::ping(&daemon.data).is_ok());
    assert!(!daemon.data.join("registry.json").exists());
}

#[test]
fn daemon_preflight_matches_local_trust_checks_without_running_the_service() {
    let temp = temp();
    let data = temp.path().join("data");
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let manifest: apphost::Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":uuid::Uuid::new_v4(),"name":"fixture",
        "services":[{"name":"main","command":["/bin/sh","-c","touch MUST_NOT_RUN"],"env":{"SECRET":"fixture-secret"}}]})).unwrap();
    fs::write(project.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
    let id =
        apphost::import(&data, &apphost::scan(&project, 0).unwrap()).unwrap()[0].installation_id;
    let _daemon = Daemon::start(&data);
    let local = apphost::runtime::LaunchPlan::prepare(&data, id).unwrap();
    let remote = daemon::check_launch(&data, id).unwrap();
    assert_eq!(remote, serde_json::to_value(local.summary()).unwrap());
    assert!(!remote.to_string().contains("fixture-secret"));
    assert!(!project.join("MUST_NOT_RUN").exists());
    assert!(!data.join("instances").exists());
    apphost::detach(&data, id).unwrap();
    assert_eq!(
        daemon::check_launch(&data, id).unwrap_err().code,
        "DETACHED"
    );
}

#[test]
fn incomplete_clients_are_bounded_and_other_clients_remain_responsive() {
    let temp = temp();
    let daemon = Daemon::start(&temp.path().join("data"));
    let mut idle = UnixStream::connect(daemon.socket()).unwrap();
    idle.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
    idle.write_all(b"{").unwrap();
    let start = Instant::now();
    assert!(daemon::ping(&daemon.data).is_ok());
    assert!(start.elapsed() < Duration::from_secs(1));
    let mut bytes = Vec::new();
    idle.read_to_end(&mut bytes).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["error"]["code"],
        "RPC_TIMEOUT"
    );
    assert!(start.elapsed() < Duration::from_secs(4));
    assert!(daemon::ping(&daemon.data).is_ok());
}

#[test]
fn connection_limit_releases_slots_and_trickling_does_not_extend_deadline() {
    let temp = temp();
    let daemon = Daemon::start(&temp.path().join("data"));
    let sockets: Vec<_> = (0..32)
        .map(|_| UnixStream::connect(daemon.socket()).unwrap())
        .collect();
    // Excess connections allow a just-finishing handshake worker to release its
    // slot; the later ping still follows a full set of idle admitted clients.
    assert_eq!(
        daemon::ping(&daemon.data).unwrap_err().code,
        "DAEMON_UNAVAILABLE"
    );
    drop(sockets);
    let deadline = Instant::now() + Duration::from_secs(4);
    while daemon::ping(&daemon.data).is_err() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let mut stream = UnixStream::connect(daemon.socket()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let started = Instant::now();
    let drip = thread::spawn(move || {
        for _ in 0..20 {
            if writer.write_all(b" ").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
    });
    let mut response = Vec::new();
    // Closing with unread request bytes can reset the socket after the error frame.
    let _ = stream.read_to_end(&mut response);
    assert_eq!(
        serde_json::from_slice::<Value>(&response).unwrap()["error"]["code"],
        "RPC_TIMEOUT"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    drip.join().unwrap();
    assert!(daemon::ping(&daemon.data).is_ok());
}

#[test]
fn excessive_socket_path_and_symlink_lock_fail_without_overwriting() {
    let temp = temp();
    let data = temp.path().join("x".repeat(100));
    private(&data);
    assert_eq!(
        daemon::serve(&data).unwrap_err().code,
        "SOCKET_PATH_TOO_LONG"
    );
    assert!(!data.join("daemon.lock").exists());
    let data = temp.path().join("linked-lock");
    private(&data);
    let target = temp.path().join("target");
    fs::write(&target, b"preserve").unwrap();
    symlink(&target, data.join("daemon.lock")).unwrap();
    assert!(daemon::serve(&data).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"preserve");
    assert!(!data.join("daemon.sock").exists());
}

#[test]
fn client_rejects_a_handshake_for_another_data_directory() {
    let temp = temp();
    let data = temp.path().join("data");
    private(&data);
    let listener = UnixListener::bind(data.join("daemon.sock")).unwrap();
    let fake = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            if byte[0] == b'\n' {
                break;
            }
            request.push(byte[0]);
        }
        let request: Value = serde_json::from_slice(&request).unwrap();
        let response = json!({"version":1,"request_id":request["request_id"],"result":{
            "protocol_version":1,"data_dir":"/another/data-directory","daemon_id":uuid::Uuid::new_v4(),
            "pid":std::process::id(),"capabilities":["hello"]}});
        let mut bytes = serde_json::to_vec(&response).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).unwrap();
    });
    assert_eq!(daemon::ping(&data).unwrap_err().code, "PROTOCOL_ERROR");
    fake.join().unwrap();
}
