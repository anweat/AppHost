#![cfg(target_os = "macos")]
use apphost::{Manifest, daemon};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[test]
fn service_fixture() {
    if std::env::var("APPHOST_LIFECYCLE_FIXTURE").is_err() {
        return;
    }
    if std::env::var("FIXTURE_IGNORE_TERM").is_ok() {
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
        }
    }
    let _listener = std::env::var("FIXTURE_PORT")
        .ok()
        .map(|p| TcpListener::bind(format!("127.0.0.1:{p}")).unwrap());
    let until = std::env::var("FIXTURE_EXIT_MS")
        .ok()
        .map(|s| s.parse::<u64>().unwrap())
        .unwrap_or(30000);
    if std::env::var("FIXTURE_CLOSE_OUTPUT").is_ok() {
        unsafe {
            libc::close(1);
            libc::close(2);
        }
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(until) {
        let _ = std::io::stdout().write_all(b"output\x00\xff\n");
        let _ = std::io::stderr().write_all(b"error\n");
        thread::sleep(Duration::from_millis(50));
    }
}
// Keep unrelated kernel/process fixtures from exhausting each other's short
// RPC deadlines. Tests of concurrent clients and instances create that load
// deliberately inside one fixture.
static FIXTURE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Fixture {
    _gate: std::sync::MutexGuard<'static, ()>,
    temp: tempfile::TempDir,
    data: PathBuf,
    daemon: Option<Child>,
    ids: Vec<Uuid>,
}
impl Fixture {
    fn new() -> Self {
        let gate = FIXTURE_GATE.lock().unwrap_or_else(|e| e.into_inner());
        let temp = tempfile::Builder::new()
            .prefix("ahl-")
            .tempdir_in("/tmp")
            .unwrap();
        let data = temp.path().join("d");
        Self {
            _gate: gate,
            temp,
            data,
            daemon: None,
            ids: vec![],
        }
    }
    fn register(&mut self, env: Value, port: Option<u16>, second: bool) -> Uuid {
        let root = self.temp.path().join(Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let mut env = env.as_object().unwrap().clone();
        env.insert("APPHOST_LIFECYCLE_FIXTURE".into(), json!("1"));
        let service = json!({"name":"fixture/unsafe/path","command":[std::env::current_exe().unwrap(),"--exact","service_fixture","--nocapture"],"env":env,"port":port,"startup_timeout_ms":3000,"stop_timeout_ms":1000});
        let mut services = vec![service];
        if second {
            services.push(json!({"name":"survivor","command":[std::env::current_exe().unwrap(),"--exact","service_fixture","--nocapture"],"env":{"APPHOST_LIFECYCLE_FIXTURE":"1"},"startup_timeout_ms":3000,"stop_timeout_ms":1000}));
        }
        let manifest:Manifest=serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture","services":services})).unwrap();
        fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id = apphost::import(&self.data, &apphost::scan(&root, 0).unwrap()).unwrap()[0]
            .installation_id;
        self.ids.push(id);
        id
    }
    fn boot(&mut self) {
        self.daemon = Some(
            Command::new(env!("CARGO_BIN_EXE_apphost"))
                .arg("--data-dir")
                .arg(&self.data)
                .args(["daemon", "serve"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait(|| daemon::ping(&self.data).is_ok());
    }
    fn rpc(&self, method: &str, params: Value) -> Value {
        daemon::call(&self.data, method, params.clone(), false)
            .unwrap_or_else(|e| panic!("{method} {params}: {e}"))
    }
    fn state(&self, id: Uuid) -> Value {
        self.rpc("status", json!({"installation_id":id}))["instances"][0].clone()
    }
    fn until(&self, id: Uuid, state: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let r = self.state(id);
            if r["status"]["state"] == state && (state == "STARTING" || r["operation_done"] == true)
            {
                return r;
            }
            assert!(Instant::now() < deadline, "Expected {state}, got {r}");
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn crash(&mut self) {
        let mut child = self.daemon.take().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if self.daemon.is_some() {
            let _ = daemon::call(&self.data, "stop-all", json!({}), false);
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if daemon::call(&self.data, "status", json!({}), false).is_ok_and(|v| {
                    v["instances"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|r| r["status"]["state"] == "STOPPED")
                }) {
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn wait(mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(6);
    while !f() {
        assert!(Instant::now() < end, "condition timed out");
        thread::sleep(Duration::from_millis(20));
    }
}
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn gone(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) != 0 }
}
#[test]
fn full_start_logs_restart_stop_and_offline_history() {
    let mut f = Fixture::new();
    let port = free_port();
    let id = f.register(json!({"FIXTURE_PORT":port.to_string()}), Some(port), false);
    f.boot();
    let start = f.rpc("start", json!({"installation_id":id}));
    let running = f.until(id, "RUNNING");
    assert_eq!(running["instance_id"], start["instance_id"]);
    assert_eq!(
        f.rpc("start", json!({"installation_id":id}))["instance_id"],
        running["instance_id"]
    );
    let logs=f.rpc("logs",json!({"installation_id":id,"instance_id":running["instance_id"],"from_ms":0,"to_ms":u64::MAX,"limit":65536}));
    assert!(!logs["window"]["chunks"].as_array().unwrap().is_empty());
    assert_eq!(logs["services"][0]["key"], "s0");
    let pid = running["services"][0]["group"]["group"].as_u64().unwrap() as u32;
    f.rpc("restart", json!({"installation_id":id}));
    let restarted = f.until(id, "RUNNING");
    assert_ne!(running["instance_id"], restarted["instance_id"]);
    wait(|| gone(pid));
    f.rpc("stop", json!({"installation_id":id}));
    f.until(id, "STOPPED");
    f.crash();
    let history = daemon::status(&f.data, Some(id)).unwrap();
    assert_eq!(history["live"], false);
    assert_eq!(history["instances"][0]["status"]["state"], "STOPPED");
}
#[test]
fn daemon_crash_preserves_services_and_collector_then_reconciles() {
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let old = f.until(id, "RUNNING");
    let pid = old["services"][0]["group"]["group"].as_u64().unwrap() as u32;
    f.crash();
    assert!(!gone(pid));
    thread::sleep(Duration::from_millis(100));
    f.boot();
    let restored = f.until(id, "RUNNING");
    assert_eq!(restored["instance_id"], old["instance_id"]);
    assert_eq!(restored["status"]["log_incomplete"], false);
    f.rpc("stop", json!({"installation_id":id}));
    f.until(id, "STOPPED");
    wait(|| gone(pid));
}
#[test]
fn collector_failure_preserves_service_until_explicit_stop() {
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let old = f.until(id, "RUNNING");
    let collector = old["collector"]["group"].as_u64().unwrap() as i32;
    assert_eq!(unsafe { libc::kill(collector, libc::SIGKILL) }, 0);
    let failed = f.until(id, "FAILED");
    assert_eq!(failed["status"]["log_incomplete"], true);
    assert_eq!(failed["services"][0]["alive"], true);
    assert_eq!(
        daemon::call(&f.data, "start", json!({"installation_id":id}), false)
            .unwrap_err()
            .code,
        "RESTART_REQUIRED"
    );
    f.rpc("stop", json!({"installation_id":id}));
    let stopped = f.until(id, "STOPPED");
    assert_eq!(stopped["status"]["log_incomplete"], true);
}
#[test]
fn foreign_port_is_rejected_before_any_spawn() {
    let mut f = Fixture::new();
    let foreign = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = foreign.local_addr().unwrap().port();
    let id = f.register(json!({}), Some(port), false);
    f.boot();
    assert_eq!(
        daemon::call(&f.data, "start", json!({"installation_id":id}), false)
            .unwrap_err()
            .code,
        "PORT_IN_USE"
    );
    assert_eq!(f.rpc("status", json!({}))["instances"], json!([]));
    assert!(foreign.local_addr().is_ok());
}

#[test]
fn partial_service_failure_preserves_other_service_and_stop_all_cancels_restart() {
    let mut f = Fixture::new();
    let id = f.register(json!({"FIXTURE_EXIT_MS":"500"}), None, true);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let failed = f.until(id, "FAILED");
    assert_eq!(failed["services"][1]["alive"], true);
    assert_eq!(failed["status"]["failure"], "SERVICE_EXITED");
    f.rpc("restart", json!({"installation_id":id}));
    f.rpc("stop-all", json!({}));
    let stopped = f.until(id, "STOPPED");
    thread::sleep(Duration::from_millis(250));
    assert_eq!(f.state(id)["instance_id"], stopped["instance_id"]);
    assert_eq!(f.rpc("status", json!({}))["automatic_wake_paused"], true);
    f.crash();
    f.boot();
    assert_eq!(f.rpc("status", json!({}))["automatic_wake_paused"], true);
}
#[test]
fn stop_cancels_starting_and_escalates_only_after_grace() {
    let mut f = Fixture::new();
    let id = f.register(json!({"FIXTURE_IGNORE_TERM":"1"}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    wait(|| f.state(id)["services"][0]["alive"] == true);
    thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    f.rpc("stop", json!({"installation_id":id}));
    let stopped = f.until(id, "STOPPED");
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(stopped["services"][0]["alive"], false);
}
#[test]
fn missing_listener_times_out_without_killing_the_service() {
    let mut f = Fixture::new();
    let id = f.register(json!({}), Some(free_port()), false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let failed = f.until(id, "FAILED");
    assert_eq!(failed["status"]["failure"], "READINESS_TIMEOUT");
    assert_eq!(failed["services"][0]["alive"], true);
}
#[test]
fn detach_waits_for_service_stop_before_writing_registration() {
    let mut f = Fixture::new();
    let id = f.register(json!({"FIXTURE_IGNORE_TERM":"1"}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    f.until(id, "RUNNING");
    f.rpc("detach", json!({"installation_id":id}));
    let pending: Value =
        serde_json::from_slice(&fs::read(f.data.join("registry.json")).unwrap()).unwrap();
    assert_eq!(pending["installations"][0]["detached"], false);
    let stopped = f.until(id, "STOPPED");
    assert_eq!(stopped["operation_result"]["detached"], true);
    assert_eq!(
        daemon::call(&f.data, "start", json!({"installation_id":id}), false)
            .unwrap_err()
            .code,
        "DETACHED"
    );
}
#[test]
fn saved_identity_mismatch_is_not_adopted_or_killed() {
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let original = f.until(id, "RUNNING");
    let pid = original["services"][0]["group"]["group"].as_u64().unwrap() as u32;
    f.crash();
    let path = f.data.join("runtime").join(format!("{id}.json"));
    let mut tampered = original.clone();
    tampered["services"][0]["group"]["known"][0]["started_seconds"] = json!(1);
    fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    f.boot();
    let failed = f.until(id, "FAILED");
    assert_eq!(failed["management_uncertain"], true);
    assert!(!gone(pid));
    f.rpc("stop", json!({"installation_id":id}));
    let stopped = f.until(id, "FAILED");
    assert!(stopped["operation_error"].is_object());
    assert!(!gone(pid));
    // Restore the trusted fixture checkpoint only for teardown; the product
    // never repairs or replaces an uncertain identity automatically.
    f.crash();
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    f.boot();
}
#[test]
fn missing_spawn_checkpoint_blocks_stop_success_and_restart() {
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let original = f.until(id, "RUNNING");
    f.crash();
    let path = f.data.join("runtime").join(format!("{id}.json"));
    let mut gap = original.clone();
    gap["services"][0]["group"] = Value::Null;
    fs::write(&path, serde_json::to_vec(&gap).unwrap()).unwrap();
    f.boot();
    assert_eq!(f.until(id, "FAILED")["management_uncertain"], true);
    f.rpc("restart", json!({"installation_id":id}));
    let failed = f.until(id, "FAILED");
    assert_eq!(failed["instance_id"], original["instance_id"]);
    f.crash();
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    f.boot();
}

#[test]
fn archived_logs_remain_addressable_and_expired_archives_are_removed() {
    let mut f = Fixture::new();
    let port = free_port();
    let id = f.register(json!({"FIXTURE_PORT":port.to_string()}), Some(port), false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let old = f.until(id, "RUNNING");
    f.rpc("restart", json!({"installation_id":id}));
    f.until(id, "RUNNING");
    let params = json!({"installation_id":id,"instance_id":old["instance_id"],"from_ms":0,"to_ms":u64::MAX,"limit":65536});
    assert!(
        !f.rpc("logs", params)["window"]["chunks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let dir = f
        .data
        .join("instances")
        .join(old["instance_id"].as_str().unwrap());
    let path = dir.join("record.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["created_at_ms"] = json!(0);
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    f.rpc("restart", json!({"installation_id":id}));
    f.until(id, "RUNNING");
    assert!(!dir.exists());
}
#[test]
fn listener_from_another_process_cannot_satisfy_readiness_after_preflight() {
    let mut f = Fixture::new();
    let port = free_port();
    let id = f.register(json!({}), Some(port), false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    wait(|| f.state(id)["services"][0]["alive"] == true);
    let _foreign = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let failed = f.until(id, "FAILED");
    assert_eq!(failed["status"]["failure"], "READINESS_TIMEOUT");
    assert_eq!(failed["services"][0]["alive"], true);
}
#[test]
fn simultaneous_starts_merge_and_disconnect_does_not_cancel() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        os::unix::net::UnixStream,
    };
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    let mut stream = UnixStream::connect(f.data.join("daemon.sock")).unwrap();
    writeln!(stream,"{}",json!({"version":1,"request_id":"disconnect","method":"start","params":{"installation_id":id}})).unwrap();
    // An acknowledgement proves peer verification and request acceptance.
    // Closing before acceptance may correctly fail getpeereid on macOS.
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).unwrap();
    assert!(
        serde_json::from_str::<Value>(&response).unwrap()["result"]["operation_id"].is_string()
    );
    drop(reader);
    wait(|| f.state(id).is_object());
    let joins: Vec<_> = (0..6)
        .map(|_| {
            let data = f.data.clone();
            thread::spawn(move || {
                daemon::call(&data, "start", json!({"installation_id":id}), false).unwrap()
            })
        })
        .collect();
    let running = f.until(id, "RUNNING");
    for j in joins {
        assert_eq!(j.join().unwrap()["instance_id"], running["instance_id"]);
    }
    // Keep an incomplete reader connected while the independent Stop RPC runs.
    let mut slow = UnixStream::connect(f.data.join("daemon.sock")).unwrap();
    slow.write_all(b"{").unwrap();
    f.rpc("stop", json!({"installation_id":id}));
    f.until(id, "STOPPED");
    let mut discard = Vec::new();
    let _ = slow.read_to_end(&mut discard);
}
#[test]
fn cli_explicit_start_bootstraps_daemon_and_read_only_status_does_not() {
    let mut f = Fixture::new();
    let port = free_port();
    let id = f.register(json!({"FIXTURE_PORT":port.to_string()}), Some(port), false);
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_apphost"))
            .arg("--data-dir")
            .arg(&f.data)
            .args(args)
            .output()
            .unwrap()
    };
    let offline = cli(&["status"]);
    assert!(offline.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&offline.stdout).unwrap()["live"],
        false
    );
    assert!(!f.data.join("daemon.sock").exists());
    let start = cli(&["start", &id.to_string()]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let pid = daemon::ping(&f.data).unwrap()["pid"].as_u64().unwrap() as i32;
    struct Cleanup(i32);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            unsafe {
                libc::kill(self.0, libc::SIGKILL);
            }
        }
    }
    let _cleanup = Cleanup(pid);
    f.until(id, "RUNNING");
    assert!(cli(&["stop-all"]).status.success());
    f.until(id, "STOPPED");
}

#[test]
fn service_limit_and_sixty_four_collector_streams_are_bounded() {
    let mut f = Fixture::new();
    f.boot();
    for count in [32, 33] {
        let root = f.temp.path().join(format!("services-{count}"));
        fs::create_dir(&root).unwrap();
        let services:Vec<_>=(0..count).map(|i|json!({"name":format!("service-{i}"),"command":["/bin/sleep","30"],"stop_timeout_ms":1000})).collect();
        let manifest:Manifest=serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"stream boundary","services":services})).unwrap();
        fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id =
            apphost::import(&f.data, &apphost::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        if count == 33 {
            assert_eq!(
                daemon::call(&f.data, "start", json!({"installation_id":id}), false)
                    .unwrap_err()
                    .code,
                "RUNTIME_LIMIT"
            );
            assert!(f.state(id).is_null());
        } else {
            f.rpc("start", json!({"installation_id":id}));
            let running = f.until(id, "RUNNING");
            assert_eq!(running["services"].as_array().unwrap().len(), 32);
            f.rpc("stop", json!({"installation_id":id}));
            f.until(id, "STOPPED");
        }
    }
}
#[test]
fn stopping_intent_survives_daemon_crash() {
    let mut f = Fixture::new();
    let id = f.register(json!({"FIXTURE_IGNORE_TERM":"1"}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    f.until(id, "RUNNING");
    f.rpc("stop", json!({"installation_id":id}));
    f.crash();
    f.boot();
    f.until(id, "STOPPED");
}
#[test]
fn immediate_exit_is_observed_without_losing_process_group_identity() {
    let mut f = Fixture::new();
    let id = f.register(json!({"FIXTURE_EXIT_MS":"0"}), None, true);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let failed = f.until(id, "FAILED");
    assert_eq!(failed["management_uncertain"], false);
    assert_eq!(failed["services"][1]["alive"], true);
    f.rpc("stop", json!({"installation_id":id}));
    f.until(id, "STOPPED");
}

#[test]
fn normal_output_eof_does_not_fail_a_live_service() {
    let mut f = Fixture::new();
    let id = f.register(json!({"FIXTURE_CLOSE_OUTPUT":"1"}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let running = f.until(id, "RUNNING");
    assert_eq!(running["collector_alive"], false);
    assert_eq!(running["status"]["log_incomplete"], false);
    f.crash();
    f.boot();
    f.until(id, "RUNNING");
    f.rpc("stop", json!({"installation_id":id}));
    assert_eq!(f.until(id, "STOPPED")["status"]["log_incomplete"], false);
}
#[test]
fn unpublished_runtime_temporary_file_is_not_a_snapshot() {
    let mut f = Fixture::new();
    f.boot();
    f.crash();
    let orphan = f.data.join("runtime/.apphost-publish-interrupted");
    fs::write(&orphan, b"incomplete write").unwrap();
    f.boot();
    assert!(!orphan.exists());
    assert_eq!(f.rpc("status", json!({}))["instances"], json!([]));
}

#[test]
fn batch_detach_preflights_unknown_ids_before_stopping_any_service() {
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    f.until(id, "RUNNING");
    let output = Command::new(env!("CARGO_BIN_EXE_apphost"))
        .arg("--data-dir")
        .arg(&f.data)
        .args(["detach", &id.to_string(), &Uuid::new_v4().to_string()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "NOT_FOUND"
    );
    assert_eq!(f.state(id)["status"]["state"], "RUNNING");
}

#[test]
fn runtime_write_failure_preserves_child_handles_for_later_stop_and_reap() {
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let running = f.until(id, "RUNNING");
    let pid = running["services"][0]["group"]["group"].as_u64().unwrap() as u32;
    let backup = f.data.join("runtime-unavailable");
    fs::rename(f.data.join("runtime"), &backup).unwrap();
    assert_eq!(
        unsafe {
            libc::kill(
                running["collector"]["group"].as_u64().unwrap() as i32,
                libc::SIGKILL,
            )
        },
        0
    );
    wait(|| f.state(id)["operation_error"]["code"] == "NOT_FOUND");
    fs::rename(backup, f.data.join("runtime")).unwrap();
    f.rpc("stop", json!({"installation_id":id}));
    f.until(id, "STOPPED");
    wait(|| gone(pid));
}

#[test]
fn stop_all_global_intent_survives_a_crash_before_individual_records_are_updated() {
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    f.rpc("start", json!({"installation_id":id}));
    let original = f.until(id, "RUNNING");
    f.crash();
    // The global publish is the first durable step of StopAll. Per-instance
    // restart intent deliberately remains from the previous generation.
    fs::write(
        f.data.join("wake.json"),
        serde_json::to_vec(&json!({"schema_version":1,"paused":true,"stop_epoch":1})).unwrap(),
    )
    .unwrap();
    let mut pending = original.clone();
    pending["status"]["state"] = json!("STOPPING");
    pending["restart_after_stop"] = json!(true);
    pending["operation_done"] = json!(false);
    fs::write(
        f.data.join("runtime").join(format!("{id}.json")),
        serde_json::to_vec(&pending).unwrap(),
    )
    .unwrap();
    f.boot();
    let stopped = f.until(id, "STOPPED");
    assert_eq!(stopped["instance_id"], original["instance_id"]);
    assert_eq!(stopped["restart_after_stop"], false);
    // A later explicit Start is still allowed without unpausing automatic wake.
    f.rpc("start", json!({"installation_id":id}));
    let started = f.until(id, "RUNNING");
    assert_ne!(started["instance_id"], original["instance_id"]);
    assert_eq!(f.rpc("status", json!({}))["automatic_wake_paused"], true);
}

#[test]
fn different_installations_start_concurrently() {
    let mut f = Fixture::new();
    let ids: Vec<_> = (0..4).map(|_| f.register(json!({}), None, true)).collect();
    f.boot();
    let barrier = std::sync::Barrier::new(ids.len());
    thread::scope(|scope| {
        let barrier = &barrier;
        let f = &f;
        for id in &ids {
            scope.spawn(move || {
                barrier.wait();
                f.rpc("start", json!({"installation_id":id}));
            });
        }
    });
    for id in ids {
        f.until(id, "RUNNING");
    }
}

#[test]
fn wildcard_listeners_are_rejected_before_spawn() {
    let mut f = Fixture::new();
    for host in ["0.0.0.0:0", "[::]:0"] {
        let foreign = TcpListener::bind(host).unwrap();
        let port = foreign.local_addr().unwrap().port();
        let id = f.register(json!({}), Some(port), false);
        if f.daemon.is_none() {
            f.boot();
        }
        let error =
            daemon::call(&f.data, "start", json!({"installation_id":id}), false).unwrap_err();
        assert_eq!(error.code, "PORT_IN_USE");
        assert_eq!(f.rpc("status", json!({}))["instances"], json!([]));
    }
}

#[test]
fn start_waits_for_external_registry_writer() {
    use fs2::FileExt;
    let mut f = Fixture::new();
    let id = f.register(json!({}), None, false);
    f.boot();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.data.join("registry.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    f.rpc("start", json!({"installation_id":id}));
    thread::sleep(Duration::from_millis(350));
    FileExt::unlock(&lock).unwrap();
    f.until(id, "RUNNING");
}
