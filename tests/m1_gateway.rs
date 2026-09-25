#![cfg(target_os = "macos")]
use apphost::{Manifest, daemon, gateway};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[test]
fn http_service_fixture() {
    let Ok(port) = std::env::var("APPHOST_GATEWAY_FIXTURE") else {
        return;
    };
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    for mut stream in listener.incoming().flatten() {
        thread::spawn(move || {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 16384 {
                let mut b = [0];
                if stream.read_exact(&mut b).is_err() {
                    return;
                }
                bytes.push(b[0]);
            }
            let head = String::from_utf8(bytes).unwrap();
            if head.starts_with("GET /ws ") {
                stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n").unwrap();
                let mut b = [0; 4];
                stream.read_exact(&mut b).unwrap();
                stream.write_all(&b).unwrap();
            } else if head.starts_with("GET /events ") {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\nD\r\ndata: first\n\n\r\n").unwrap();
                stream.flush().unwrap();
                thread::sleep(Duration::from_millis(250));
                let _ = stream.write_all(b"E\r\ndata: second\n\n\r\n0\r\n\r\n");
            } else {
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                assert!(length <= 2 * 1024 * 1024);
                let mut body = vec![0; length];
                if stream.read_exact(&mut body).is_err() {
                    return;
                }
                let body = json!({"headers":head,"length":length}).to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
        });
    }
}

fn port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
struct Fixture {
    _temp: tempfile::TempDir,
    data: PathBuf,
    root: PathBuf,
    id: Uuid,
    port: u16,
    host: String,
    daemon: Option<Child>,
}
impl Fixture {
    fn new(command: Option<&str>) -> Self {
        let temp = tempfile::Builder::new()
            .prefix("ahg-")
            .tempdir_in("/tmp")
            .unwrap();
        let data = temp.path().join("data");
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        let app_port = port();
        let gateway_port = port();
        let command = command.map(|c| vec![c.to_owned()]).unwrap_or_else(|| {
            vec![
                std::env::current_exe().unwrap().to_str().unwrap().into(),
                "--exact".into(),
                "http_service_fixture".into(),
                "--nocapture".into(),
            ]
        });
        let manifest: Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"Gateway fixture",
            "services":[{"name":"web","command":command,"port":app_port,"env":{"APPHOST_GATEWAY_FIXTURE":app_port.to_string()},"startup_timeout_ms":3000,"stop_timeout_ms":1000}]})).unwrap();
        fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let installation = apphost::import(&data, &apphost::scan(&root, 0).unwrap())
            .unwrap()
            .remove(0);
        let host = format!("{}.localhost:{gateway_port}", installation.gateway_slug());
        gateway::save_config(&data, gateway_port).unwrap();
        Self {
            _temp: temp,
            data,
            root,
            id: installation.installation_id,
            port: gateway_port,
            host,
            daemon: None,
        }
    }
    fn boot(&mut self) {
        self.daemon = Some(
            Command::new(env!("CARGO_BIN_EXE_apphost"))
                .arg("--data-dir")
                .arg(&self.data)
                .args(["daemon", "serve"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let end = Instant::now() + Duration::from_secs(5);
        while daemon::ping(&self.data).is_err() {
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn connect(&self) -> TcpStream {
        let s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s
    }
    fn request(&self, path: &str, headers: &str) -> String {
        let mut s = self.connect();
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n{headers}\r\n",
            self.host
        )
        .unwrap();
        let mut result = String::new();
        s.read_to_string(&mut result).unwrap();
        result
    }
    fn wait_state(&self, wanted: &str) -> Value {
        let end = Instant::now() + Duration::from_secs(8);
        loop {
            let status = daemon::status(&self.data, Some(self.id)).unwrap();
            if status["instances"][0]["status"]["state"] == wanted {
                return status["instances"][0].clone();
            }
            assert!(Instant::now() < end, "{status}");
            thread::sleep(Duration::from_millis(30));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = &mut self.daemon {
            let _ = daemon::call(&self.data, "stop-all", json!({}), false);
            let until = Instant::now() + Duration::from_secs(4);
            while Instant::now() < until {
                let Ok(s) = daemon::status(&self.data, None) else {
                    break;
                };
                if s["instances"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["status"]["state"] == "STOPPED")
                {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
const NAV: &str =
    "Sec-Fetch-Dest: document\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Site: none\r\n";

#[test]
fn stable_url_is_read_only_and_survives_label_detach_and_legacy_records() {
    let f = Fixture::new(None);
    fs::remove_file(f.data.join("gateway.json")).unwrap();
    let before = fs::read(f.data.join("registry.json")).unwrap();
    let original = gateway::url(&f.data, f.id).unwrap();
    assert!(original["url"].as_str().unwrap().contains(":18480/"));
    assert!(!f.data.join("gateway.json").exists());
    assert_eq!(before, fs::read(f.data.join("registry.json")).unwrap());
    apphost::label(&f.data, f.id, "Renamed 中文 label").unwrap();
    assert_eq!(original, gateway::url(&f.data, f.id).unwrap());
    apphost::detach(&f.data, f.id).unwrap();
    assert_eq!(gateway::url(&f.data, f.id).unwrap_err().code, "DETACHED");
    apphost::import(&f.data, &apphost::scan(&f.root, 0).unwrap()).unwrap();
    assert_eq!(original, gateway::url(&f.data, f.id).unwrap());
    let path = f.data.join("registry.json");
    let mut legacy: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy["installations"][0]
        .as_object_mut()
        .unwrap()
        .remove("slug");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let old = gateway::url(&f.data, f.id).unwrap();
    apphost::label(&f.data, f.id, "Another name").unwrap();
    assert_eq!(old, gateway::url(&f.data, f.id).unwrap());
    assert!(
        old["url"]
            .as_str()
            .unwrap()
            .contains(&format!("app-{}", f.id.simple()))
    );
}

#[test]
fn navigation_wakes_once_and_proxies_http_streams_uploads_and_websocket() {
    let mut f = Fixture::new(None);
    f.boot();
    assert!(f.request("/favicon.ico", "").contains("APP_STOPPED"));
    assert!(
        daemon::status(&f.data, None).unwrap()["instances"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let loading = f.request("/", NAV);
    assert!(loading.starts_with("HTTP/1.1 202"), "{loading}");
    assert!(loading.contains("content-security-policy"));
    let running = f.wait_state("RUNNING");
    let poll = f.request(
        &format!(
            "/__apphost/status/{}",
            running["operation_id"].as_str().unwrap()
        ),
        "",
    );
    assert!(poll.contains("\"ready\":true"), "{poll}");
    let content=f.request("/api?value=1", "Origin: http://example.invalid\r\nX-Forwarded-Host: attacker\r\nForwarded: for=attacker\r\n");
    assert!(content.starts_with("HTTP/1.1 200"), "{content}");
    assert!(content.contains("/api?value=1"));
    assert!(content.contains("http://example.invalid"));
    assert!(!content.contains("attacker"));
    assert!(content.contains(&f.host));
    let mut upload = f.connect();
    let body = vec![b'x'; 1024 * 1024];
    write!(
        upload,
        "POST /upload HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        f.host,
        body.len()
    )
    .unwrap();
    upload.write_all(&body).unwrap();
    let mut response = String::new();
    upload.read_to_string(&mut response).unwrap();
    assert!(response.contains("1048576"), "{response}");
    let mut events = f.connect();
    write!(
        events,
        "GET /events HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        f.host
    )
    .unwrap();
    let mut early = Vec::new();
    while !String::from_utf8_lossy(&early).contains("data: first") {
        let mut chunk = [0; 4096];
        let n = events.read(&mut chunk).unwrap();
        assert!(n > 0);
        early.extend_from_slice(&chunk[..n]);
    }
    assert!(
        !String::from_utf8_lossy(&early).contains("data: second"),
        "Stream was buffered until completion"
    );
    let mut remaining = String::new();
    events.read_to_string(&mut remaining).unwrap();
    assert!(remaining.contains("data: second"));
    let mut ws = f.connect();
    write!(ws,"GET /ws HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",f.host).unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0];
        ws.read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    assert!(String::from_utf8(head).unwrap().starts_with("HTTP/1.1 101"));
    ws.write_all(b"echo").unwrap();
    let mut echo = [0; 4];
    ws.read_exact(&mut echo).unwrap();
    assert_eq!(&echo, b"echo");
    assert_eq!(
        running["instance_id"],
        f.wait_state("RUNNING")["instance_id"]
    );
}

#[test]
fn failed_wake_and_stop_all_cannot_be_retried_by_browser_requests() {
    let mut f = Fixture::new(Some("apphost-gateway-missing-command"));
    f.boot();
    assert!(f.request("/", NAV).contains("INVALID_COMMAND"));
    let failed = f.wait_state("FAILED");
    assert!(f.request("/", NAV).contains("INVALID_COMMAND"));
    assert_eq!(
        failed["operation_id"],
        f.wait_state("FAILED")["operation_id"]
    );
    daemon::call(&f.data, "stop-all", json!({}), false).unwrap();
    f.wait_state("STOPPED");
    assert!(f.request("/", NAV).contains("WAKE_PAUSED"));
    daemon::call(&f.data, "resume-wake", json!({}), false).unwrap();
    assert!(f.request("/", NAV).contains("INVALID_COMMAND"));
    assert_eq!(
        failed["instance_id"],
        f.wait_state("STOPPED")["instance_id"]
    );
}

#[test]
fn malformed_hosts_metadata_and_status_queries_never_launch() {
    let mut f = Fixture::new(None);
    f.boot();
    for extra in [
        "",
        "Sec-Fetch-Dest: document\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Site: same-origin\r\n",
        "Sec-Fetch-Dest: document\r\nSec-Fetch-Dest: document\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Site: none\r\n",
    ] {
        assert!(f.request("/", extra).contains("APP_STOPPED"));
    }
    assert!(
        f.request(&format!("/__apphost/status/{}", Uuid::new_v4()), NAV)
            .contains("OPERATION_EXPIRED")
    );
    for request in [
        format!("GET / HTTP/1.1\r\nHost: attacker.invalid:{}\r\n", f.port),
        format!("GET / HTTP/1.1\r\nHost: {}\r\nHost: {}\r\n", f.host, f.host),
        format!("GET http://{}/ HTTP/1.1\r\nHost: {}\r\n", f.host, f.host),
        format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n", f.host, f.host),
    ] {
        let mut stream = f.connect();
        write!(stream, "{request}Connection: close\r\n{NAV}\r\n").unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        assert!(response.starts_with("HTTP/1.1 4"), "{response}");
    }
    assert!(
        daemon::status(&f.data, None).unwrap()["instances"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut stream = f.connect();
    let _ = write!(
        stream,
        "GET / HTTP/1.1\r\nHost: {}\r\nX-Large: {}\r\nConnection: close\r\n\r\n",
        f.host,
        "x".repeat(17000)
    );
    let mut rejected = String::new();
    let _ = stream.read_to_string(&mut rejected);
    assert!(rejected.is_empty() || rejected.starts_with("HTTP/1.1 4"));
}

#[test]
fn port_conflict_does_not_block_management_and_live_reconfiguration_is_atomic() {
    let mut f = Fixture::new(None);
    let occupied = TcpListener::bind(("0.0.0.0", f.port)).unwrap();
    f.boot();
    let status = daemon::status(&f.data, None).unwrap();
    assert_eq!(status["gateway"]["error"]["code"], "GATEWAY_PORT_IN_USE");
    daemon::call(&f.data, "stop-all", json!({}), false).unwrap();
    let fresh = port();
    let cli = Command::new(env!("CARGO_BIN_EXE_apphost"))
        .arg("--data-dir")
        .arg(&f.data)
        .args(["gateway", "set-port", &fresh.to_string()])
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(cli.stderr.is_empty());
    assert_eq!(gateway::config(&f.data).unwrap().port, fresh);
    let old = f.port;
    f.port = fresh;
    f.host = f.host.replace(&format!(":{old}"), &format!(":{fresh}"));
    assert!(f.request("/", NAV).contains("WAKE_PAUSED"));
    assert_eq!(
        gateway::set_port(&f.data, old).unwrap_err().code,
        "GATEWAY_PORT_IN_USE"
    );
    assert_eq!(gateway::config(&f.data).unwrap().port, fresh);
    assert!(f.request("/", NAV).contains("WAKE_PAUSED"));
    drop(occupied);
    let second = port();
    gateway::set_port(&f.data, second).unwrap();
    assert_eq!(
        daemon::status(&f.data, None).unwrap()["gateway"]["port"],
        second
    );
    let end = Instant::now() + Duration::from_secs(2);
    while TcpStream::connect(("127.0.0.1", fresh)).is_ok() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn corrupted_settings_and_foreign_slugs_fail_without_rewriting_them() {
    let f = Fixture::new(None);
    let path = f.data.join("gateway.json");
    let corrupt = br#"{"schema_version":2,"port":18480,"future":true}"#;
    fs::write(&path, corrupt).unwrap();
    assert_eq!(
        gateway::config(&f.data).unwrap_err().code,
        "GATEWAY_CONFIG_INVALID"
    );
    assert_eq!(
        gateway::save_config(&f.data, 19000).unwrap_err().code,
        "GATEWAY_CONFIG_INVALID"
    );
    assert_eq!(fs::read(&path).unwrap(), corrupt);
    let registry = f.data.join("registry.json");
    let mut contents: Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    contents["installations"][0]["slug"] = json!(format!("app-{}", Uuid::new_v4().simple()));
    fs::write(&registry, serde_json::to_vec(&contents).unwrap()).unwrap();
    assert_eq!(
        gateway::url(&f.data, f.id).unwrap_err().code,
        "REGISTRY_INVALID"
    );
}

#[test]
fn incomplete_http_clients_do_not_block_stop_all_and_expire() {
    let mut f = Fixture::new(None);
    f.boot();
    let clients: Vec<_> = (0..64)
        .map(|_| {
            let mut s = f.connect();
            s.write_all(b"GET / HTTP/1.1\r\n").unwrap();
            s
        })
        .collect();
    assert_eq!(
        daemon::call(&f.data, "stop-all", json!({}), false).unwrap()["automatic_wake_paused"],
        true
    );
    thread::sleep(Duration::from_millis(5300));
    drop(clients);
    assert!(f.request("/", NAV).contains("WAKE_PAUSED"));
}
