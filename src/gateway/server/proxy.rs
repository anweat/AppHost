use super::*;
use hyper::header::{HeaderMap, HeaderName, HeaderValue};

fn strip(headers: &mut HeaderMap, request: bool) {
    let named: Vec<_> = headers
        .get_all("connection")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|v| HeaderName::from_bytes(v.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
    if request {
        let untrusted: Vec<_> = headers
            .keys()
            .filter(|k| k.as_str() == "forwarded" || k.as_str().starts_with("x-forwarded-"))
            .cloned()
            .collect();
        for name in untrusted {
            headers.remove(name);
        }
    }
}

pub(super) async fn forward(
    mut req: Request<Incoming>,
    addresses: Vec<SocketAddr>,
    slot: Arc<tokio::sync::OwnedSemaphorePermit>,
) -> std::result::Result<Response<Body>, Box<dyn std::error::Error + Send + Sync>> {
    let upgrade = req.headers().get("upgrade").is_some();
    if upgrade
        && (req.method() != hyper::Method::GET
            || req.headers().get_all("upgrade").iter().count() != 1
            || !req
                .headers()
                .get("upgrade")
                .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"))
            || !req
                .headers()
                .get_all("connection")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(','))
                .any(|v| v.trim().eq_ignore_ascii_case("upgrade")))
    {
        return Ok(page_error(400, "INVALID_UPGRADE"));
    }
    let browser_upgrade = upgrade.then(|| hyper::upgrade::on(&mut req));
    let host = req.headers()["host"].clone();
    let origins: Vec<_> = req.headers().get_all("origin").iter().cloned().collect();
    strip(req.headers_mut(), true);
    req.headers_mut().insert("host", host.clone());
    req.headers_mut().remove("origin");
    for origin in origins {
        req.headers_mut().append("origin", origin);
    }
    req.headers_mut().insert("x-forwarded-host", host);
    req.headers_mut()
        .insert("x-forwarded-proto", HeaderValue::from_static("http"));
    if upgrade {
        req.headers_mut()
            .insert("connection", HeaderValue::from_static("Upgrade"));
        req.headers_mut()
            .insert("upgrade", HeaderValue::from_static("websocket"));
    }
    let mut stream = None;
    for address in addresses {
        if let Ok(Ok(socket)) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect(address),
        )
        .await
        {
            stream = Some(socket);
            break;
        }
    }
    let stream = stream.ok_or("Declared upstream is unavailable")?;
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_buf_size(16384)
        .handshake(TokioIo::new(stream))
        .await?;
    let connection_slot = slot.clone();
    tokio::spawn(async move {
        let _slot = connection_slot;
        let _ = connection.with_upgrades().await;
    });
    let mut res =
        tokio::time::timeout(Duration::from_secs(120), sender.send_request(req)).await??;
    if res.status() == hyper::StatusCode::SWITCHING_PROTOCOLS {
        let Some(browser_upgrade) = browser_upgrade else {
            return Err("Unexpected upstream upgrade".into());
        };
        if !res
            .headers()
            .get("upgrade")
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"))
        {
            return Err("Invalid upstream upgrade".into());
        }
        let upstream_upgrade = hyper::upgrade::on(&mut res);
        strip(res.headers_mut(), false);
        res.headers_mut()
            .insert("connection", HeaderValue::from_static("Upgrade"));
        res.headers_mut()
            .insert("upgrade", HeaderValue::from_static("websocket"));
        tokio::spawn(async move {
            let _slot = slot;
            if let Ok(Ok((browser, upstream))) =
                tokio::time::timeout(Duration::from_secs(5), async {
                    Ok::<_, hyper::Error>((browser_upgrade.await?, upstream_upgrade.await?))
                })
                .await
            {
                let _ = tokio::io::copy_bidirectional(
                    &mut TokioIo::new(browser),
                    &mut TokioIo::new(upstream),
                )
                .await;
            }
        });
    } else {
        strip(res.headers_mut(), false);
    }
    Ok(res.map(BodyExt::boxed))
}
