use super::*;
use crate::runtime::{
    State,
    lifecycle::{BrowserObservation, Manager},
};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    convert::Infallible,
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Semaphore, oneshot};

mod proxy;
type Body = BoxBody<Bytes, hyper::Error>;

#[derive(Default)]
pub(crate) struct Controller {
    running: Mutex<Option<Running>>,
    diagnostic: Mutex<Value>,
}
struct Running {
    port: u16,
    stop: oneshot::Sender<()>,
}
impl Controller {
    pub(crate) fn status(&self) -> Value {
        self.diagnostic.lock().unwrap().clone()
    }
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn start(&self, data: &Path, manager: Arc<Manager>) {
        let result = config_lock(data)
            .and_then(|_lock| config(data).and_then(|c| self.set_locked(data, manager, c.port)));
        if let Err(error) = result {
            *self.diagnostic.lock().unwrap() = json!({"available":false,"error":error});
        }
    }
    pub(crate) fn set(&self, data: &Path, manager: Arc<Manager>, port: u16) -> Result<Value> {
        let _lock = config_lock(data)?;
        self.set_locked(data, manager, port)
    }
    fn set_locked(&self, data: &Path, manager: Arc<Manager>, port: u16) -> Result<Value> {
        let mut running = self.running.lock().unwrap();
        if running.as_ref().is_some_and(|r| r.port == port) && self.status()["available"] == true {
            publish_config(data, port)?;
            return Ok(self.status());
        }
        if port == 0 {
            return Err(Error::new("INVALID_PORT", "Gateway port must be nonzero"));
        }
        // macOS may allow a reusable wildcard listener beside a specific
        // loopback bind. Check observed owners before binding either family.
        let owners = crate::process::port_owners(port);
        if owners["found"] == true {
            return Err(
                Error::new("GATEWAY_PORT_IN_USE", "Gateway port already has a listener")
                    .details(json!({"port":port,"ownership":owners})),
            );
        }
        let bind = |addr: SocketAddr| {
            TcpListener::bind(addr).map_err(|e| {
                Error::new(
                    if e.kind() == std::io::ErrorKind::AddrInUse {
                        "GATEWAY_PORT_IN_USE"
                    } else {
                        "GATEWAY_BIND_FAILED"
                    },
                    "Gateway listener could not be bound; choose an explicit port",
                )
                .details(json!({"port":port,"address":addr.to_string(),"reason":e.to_string()}))
            })
        };
        let listeners = [
            bind(([127, 0, 0, 1], port).into())?,
            bind(([0, 0, 0, 0, 0, 0, 0, 1], port).into())?,
        ];
        for listener in &listeners {
            listener
                .set_nonblocking(true)
                .map_err(|e| Error::new("GATEWAY_BIND_FAILED", e.to_string()))?;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(16)
            .build()
            .map_err(|e| Error::new("GATEWAY_UNAVAILABLE", e.to_string()))?;
        let data = data.to_owned();
        let server_data = data.clone();
        let (stop, quit) = oneshot::channel();
        let (start, ready) = std::sync::mpsc::sync_channel(1);
        // Allocate the thread and both listeners before publishing a new address.
        // A failed save drops `start`; no replacement server becomes active.
        std::thread::Builder::new()
            .name("apphost-gateway".into())
            .spawn(move || {
                if ready.recv().is_ok()
                    && let Err(error) =
                        runtime.block_on(serve(listeners, server_data, port, manager.clone(), quit))
                {
                    let mut status = manager.gateway.diagnostic.lock().unwrap();
                    if status["port"] == port {
                        *status = json!({"available":false,"port":port,"error":error});
                    }
                }
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .map_err(|e| Error::new("GATEWAY_UNAVAILABLE", e.to_string()))?;
        publish_config(&data, port)?;
        start
            .send(())
            .map_err(|_| Error::new("GATEWAY_UNAVAILABLE", "Gateway thread exited"))?;
        if let Some(old) = running.replace(Running { port, stop }) {
            let _ = old.stop.send(());
        }
        let status = json!({"available":true,"port":port});
        *self.diagnostic.lock().unwrap() = status.clone();
        Ok(status)
    }
}

async fn serve(
    listeners: [TcpListener; 2],
    data: PathBuf,
    port: u16,
    manager: Arc<Manager>,
    mut quit: oneshot::Receiver<()>,
) -> Result<()> {
    let [v4, v6] = listeners
        .map(|l| tokio::net::TcpListener::from_std(l).expect("registered nonblocking listener"));
    let slots = Arc::new(Semaphore::new(64));
    loop {
        let accepted = tokio::select! {
            _ = &mut quit => return Ok(()),
            result = v4.accept() => result,
            result = v6.accept() => result,
        };
        let (stream, peer) =
            accepted.map_err(|e| Error::new("GATEWAY_UNAVAILABLE", e.to_string()))?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let slot = Arc::new(slot);
        let data = data.clone();
        let manager = manager.clone();
        tokio::spawn(async move {
            let service = service_fn(move |req| {
                handle(req, data.clone(), port, manager.clone(), slot.clone())
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(5))
                .max_headers(128)
                .max_buf_size(16384)
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}

enum Decision {
    Proxy(Vec<SocketAddr>),
    Page(Option<BrowserObservation>),
    Poll(BrowserObservation),
}
fn decide(
    data: &Path,
    port: u16,
    manager: &Arc<Manager>,
    host: &str,
    path: &str,
    navigation: bool,
) -> Result<Decision> {
    let slug = host
        .strip_suffix(&format!(".localhost:{port}"))
        .or_else(|| {
            (port == 80)
                .then(|| host.strip_suffix(".localhost"))
                .flatten()
        })
        .filter(|s| !s.is_empty() && s.len() <= 63)
        .ok_or_else(|| Error::new("ROUTE_NOT_FOUND", "Unknown Host"))?;
    let installation = crate::registry::registered_slug(data, slug)?;
    let snapshot = crate::registry::launch_snapshot(data, installation.installation_id)?;
    let target = snapshot
        .manifest()
        .open_target()?
        .ok_or_else(|| Error::new("NO_BROWSER_ENTRY", "No declared browser entry"))?;
    if let Some(op) = path.strip_prefix("/__apphost/status/") {
        let op = Uuid::parse_str(op)
            .map_err(|_| Error::new("OPERATION_EXPIRED", "Invalid operation"))?;
        return manager
            .browser_observe(installation.installation_id, op)
            .map(Decision::Poll);
    }
    let mut observed = manager.browser_current(installation.installation_id)?;
    if !observed.as_ref().is_some_and(|o| o.ready) && navigation {
        observed = Some(manager.browser_wake(installation.installation_id)?);
    }
    if observed.as_ref().is_some_and(|o| o.ready) {
        if target.port == port {
            return Err(Error::new("GATEWAY_LOOP", "App port equals gateway port"));
        }
        return manager
            .browser_upstream(
                installation.installation_id,
                &target,
                &snapshot.manifest().launch_sha256(),
            )
            .map(Decision::Proxy);
    }
    Ok(Decision::Page(observed))
}

async fn handle(
    req: Request<Incoming>,
    data: PathBuf,
    port: u16,
    manager: Arc<Manager>,
    slot: Arc<tokio::sync::OwnedSemaphorePermit>,
) -> std::result::Result<Response<Body>, Infallible> {
    let hosts: Vec<_> = req.headers().get_all("host").iter().collect();
    let path = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("");
    if hosts.len() != 1
        || req.uri().scheme().is_some()
        || req.uri().authority().is_some()
        || !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('\\')
        || path.len() > 8192
        || req.method() == hyper::Method::CONNECT
    {
        return Ok(page_error(421, "INVALID_GATEWAY_REQUEST"));
    }
    let Ok(host) = hosts[0].to_str() else {
        return Ok(page_error(421, "INVALID_HOST"));
    };
    let raw: Vec<_> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str(), v.to_str().unwrap_or("\0")))
        .collect();
    let navigation = is_wake_navigation(req.method().as_str(), &raw);
    let poll = path.starts_with("/__apphost/status/");
    if poll && req.method() != hyper::Method::GET {
        return Ok(page_error(405, "METHOD_NOT_ALLOWED"));
    }
    let host = host.to_owned();
    let path = path.to_owned();
    let work_slot = slot.clone();
    let decision = tokio::task::spawn_blocking(move || {
        // Client disconnect cannot release capacity while its blocking source
        // check or accepted lifecycle submission is still running.
        let _slot = work_slot;
        decide(&data, port, &manager, &host, &path, navigation)
    })
    .await;
    let response = match decision {
        Ok(Ok(Decision::Proxy(addresses))) => proxy::forward(req, addresses, slot)
            .await
            .unwrap_or_else(|_| page_error(502, "UPSTREAM_UNAVAILABLE")),
        Ok(Ok(Decision::Poll(observation))) => response(
            200,
            "application/json",
            serde_json::to_string(&observation).unwrap(),
        ),
        Ok(Ok(Decision::Page(observation))) => page(observation),
        Ok(Err(error)) if poll => response(
            409,
            "application/json",
            json!({"error":error.code}).to_string(),
        ),
        Ok(Err(error)) => page_error(
            if error.code == "ROUTE_NOT_FOUND" {
                421
            } else {
                503
            },
            &error.code,
        ),
        Err(_) => page_error(503, "GATEWAY_UNAVAILABLE"),
    };
    Ok(response)
}

fn response(status: u16, content_type: &str, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff")
        .header("referrer-policy", "no-referrer")
        .header("x-frame-options", "DENY")
        .body(
            Full::new(Bytes::from(body))
                .map_err(|never| match never {})
                .boxed(),
        )
        .unwrap()
}
fn page_error(status: u16, code: &str) -> Response<Body> {
    // Codes originate in core, but still escape rather than turn errors into HTML.
    let safe: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(64)
        .collect();
    response(
        status,
        "text/html; charset=utf-8",
        format!(
            "<!doctype html><meta charset=utf-8><title>AppHost</title><h1>暂时无法打开</h1><p>{safe}</p><p>请在 AppHost 中查看并显式启动或重试。</p>"
        ),
    )
}
fn page(observed: Option<BrowserObservation>) -> Response<Body> {
    let Some(o) = observed else {
        return page_error(503, "APP_STOPPED");
    };
    if let Some(cause) = &o.cause {
        return page_error(503, cause);
    }
    if !matches!(o.state, State::Starting | State::Running) {
        return page_error(503, "APP_STOPPED");
    }
    let nonce = Uuid::new_v4().simple().to_string();
    let mut page = response(
        202,
        "text/html; charset=utf-8",
        format!(
            r#"<!doctype html><meta charset=utf-8><title>AppHost · 正在启动</title><h1>正在启动…</h1><p id="status">请稍候</p><script nonce="{nonce}">
const end=Date.now()+605000;async function poll(){{try{{const r=await fetch('/__apphost/status/{}',{{cache:'no-store'}});const s=await r.json();if(s.ready){{location.reload();return}}if(!r.ok||s.cause||s.state==='STOPPED'||s.state==='FAILED'||Date.now()>end){{document.getElementById('status').textContent='请在 AppHost 中查看并显式重试';return}}setTimeout(poll,1000)}}catch{{document.getElementById('status').textContent='连接中断，请在 AppHost 中查看'}}}}poll();</script>"#,
            o.operation_id
        ),
    );
    page.headers_mut().insert("content-security-policy", format!("default-src 'none'; script-src 'nonce-{nonce}'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'").parse().unwrap());
    page
}
