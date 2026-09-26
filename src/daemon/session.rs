//! Transport-independent request dispatch, subscriptions and log streaming.
use super::protocol;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
fn read_frame(stream: &mut impl crate::platform::LocalStream, maximum: usize) -> Result<Vec<u8>> {
    stream.read_frame(maximum)
}
fn write_frame(stream: &mut impl crate::platform::LocalStream, bytes: &[u8]) -> Result<()> {
    stream.write_frame(bytes)
}
pub fn handle(
    mut stream: impl crate::platform::LocalStream,
    data_dir: &Path,
    hello: &Value,
    manager: &Arc<crate::runtime::lifecycle::Manager>,
    subscriptions: &Arc<AtomicUsize>,
) -> Result<()> {
    stream.authenticate()?;
    let request =
        read_frame(&mut stream, protocol::MAX_REQUEST).and_then(|bytes| protocol::decode(&bytes));
    if let Ok(request) = &request
        && request.version == protocol::VERSION
        && matches!(
            request.method.as_str(),
            "subscribe" | "logs-follow" | "condition-watch"
        )
    {
        let result = if request.method == "condition-watch" {
            watch_condition(&mut stream, manager, request, subscriptions)
        } else if request.method == "logs-follow" {
            follow_logs(&mut stream, manager, request, subscriptions)
        } else {
            subscribe(&mut stream, manager, request, subscriptions)
        };
        if let Err(error) = result {
            let response = protocol::response(Some(&request.request_id), Err(error));
            return write_frame(
                &mut stream,
                &protocol::encode(&response, protocol::MAX_RESPONSE)?,
            );
        }
        return Ok(());
    }
    let (id, result) = match &request {
        Ok(request) => (
            Some(request.request_id.as_str()),
            if request.version != protocol::VERSION
                || matches!(request.method.as_str(), "hello" | "check-launch")
            {
                protocol::dispatch(data_dir, request, hello)
            } else {
                manager.dispatch(request)
            },
        ),
        Err(_) => (None, Err(request.err().unwrap())),
    };
    let response = protocol::response(id, result);
    let bytes = protocol::encode(&response, protocol::MAX_RESPONSE).or_else(|error| {
        protocol::encode(&protocol::response(id, Err(error)), protocol::MAX_RESPONSE)
    })?;
    write_frame(&mut stream, &bytes)
}

fn watch_condition(
    stream: &mut impl crate::platform::LocalStream,
    manager: &Arc<crate::runtime::lifecycle::Manager>,
    request: &protocol::Request,
    subscriptions: &Arc<AtomicUsize>,
) -> Result<()> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        installation_id: Uuid,
        failure_id: Uuid,
        duration_secs: Option<u64>,
    }
    let p: Params = serde_json::from_value(request.params.clone()).map_err(|_| {
        Error::new(
            "INVALID_PARAMS",
            "Invalid condition subscription parameters",
        )
    })?;
    let seconds = p.duration_secs.unwrap_or(60);
    if !(1..=60).contains(&seconds) {
        return Err(Error::new(
            "INVALID_PARAMS",
            "Condition duration must be 1..60 seconds",
        ));
    }
    subscriptions
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 8).then_some(n + 1)
        })
        .map_err(|_| {
            Error::new(
                "SUBSCRIPTION_LIMIT",
                "At most eight observation subscriptions",
            )
        })?;
    let _slot = Slot(subscriptions.clone());
    let until = Instant::now() + Duration::from_secs(seconds);
    loop {
        let mut value = manager.port_condition(p.installation_id, p.failure_id)?;
        if Instant::now() >= until && value["complete"] != true {
            value["state"] = json!("expired");
            value["complete"] = json!(true);
        }
        let complete = value["complete"] == true;
        write_frame(
            stream,
            &protocol::encode(
                &protocol::response(
                    Some(&request.request_id),
                    Ok(json!({"kind":"condition","data":value})),
                ),
                protocol::MAX_RESPONSE,
            )?,
        )?;
        if complete {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn follow_logs(
    stream: &mut impl crate::platform::LocalStream,
    manager: &Arc<crate::runtime::lifecycle::Manager>,
    request: &protocol::Request,
    slots: &Arc<AtomicUsize>,
) -> Result<()> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        installation_id: Uuid,
        instance_id: Uuid,
        after: Option<u64>,
        limit: Option<usize>,
    }
    let params: Params = serde_json::from_value(request.params.clone())
        .map_err(|_| Error::new("INVALID_PARAMS", "Invalid log subscription parameters"))?;
    slots
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 8).then_some(n + 1)
        })
        .map_err(|_| {
            Error::new(
                "SUBSCRIPTION_LIMIT",
                "At most eight observation subscriptions",
            )
        })?;
    let _slot = Slot(slots.clone());
    let mut after = params.after;
    loop {
        let value = manager.logs_after(
            params.installation_id,
            params.instance_id,
            after,
            params.limit.unwrap_or(65536),
        )?;
        after = value["next_cursor"].as_u64();
        let complete = value["complete"] == true;
        let result = json!({"kind":"logs","data":value});
        write_frame(
            stream,
            &protocol::encode(
                &protocol::response(Some(&request.request_id), Ok(result)),
                protocol::MAX_RESPONSE,
            )?,
        )?;
        if complete {
            return Ok(());
        }
        // Only explicit log readers pay this scan cost; no permanent watcher.
        thread::sleep(Duration::from_millis(500));
    }
}

fn subscribe<S: crate::platform::LocalStream>(
    stream: &mut S,
    manager: &Arc<crate::runtime::lifecycle::Manager>,
    request: &protocol::Request,
    slots: &Arc<AtomicUsize>,
) -> Result<()> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        cursor: Option<crate::events::Cursor>,
        installation_id: Option<Uuid>,
    }
    let params: Params = serde_json::from_value(request.params.clone())
        .map_err(|_| Error::new("INVALID_PARAMS", "Invalid subscription parameters"))?;
    slots
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 8).then_some(n + 1)
        })
        .map_err(|_| {
            Error::new(
                "SUBSCRIPTION_LIMIT",
                "At most eight observation subscriptions",
            )
        })?;
    let _slot = Slot(slots.clone());
    let send = |stream: &mut S, value| {
        write_frame(
            stream,
            &protocol::encode(
                &protocol::response(Some(&request.request_id), Ok(value)),
                protocol::MAX_RESPONSE,
            )?,
        )
    };
    let mut cursor = match params.cursor {
        Some(cursor) => {
            manager.events.after(&cursor, Duration::ZERO)?;
            send(stream, json!({"kind":"subscribed","cursor":cursor}))?;
            cursor
        }
        None => {
            let snapshot = manager.status(params.installation_id)?;
            let cursor = serde_json::from_value(snapshot["cursor"].clone())
                .map_err(|_| Error::new("PROTOCOL_ERROR", "Missing snapshot cursor"))?;
            send(
                stream,
                json!({"kind":"snapshot","snapshot":snapshot,"cursor":cursor}),
            )?;
            cursor
        }
    };
    loop {
        let events = manager.events.after(&cursor, Duration::from_secs(30))?;
        if events.is_empty() {
            send(stream, json!({"kind":"heartbeat","cursor":cursor}))?;
        }
        let had_events = !events.is_empty();
        let mut sent = false;
        for event in events {
            cursor = serde_json::from_value(event["cursor"].clone())
                .map_err(|_| Error::new("PROTOCOL_ERROR", "Invalid event cursor"))?;
            if params.installation_id.is_none()
                || event["installation_id"].is_null()
                || event["installation_id"] == json!(params.installation_id)
            {
                send(stream, event)?;
                sent = true;
            }
        }
        if had_events && !sent {
            send(stream, json!({"kind":"heartbeat","cursor":cursor}))?;
        }
    }
}

pub(super) struct Slot(pub(super) Arc<AtomicUsize>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct FakeLocalStream {
        authenticated: bool,
        request: Vec<u8>,
        output: Arc<Mutex<Vec<u8>>>,
    }
    impl crate::platform::LocalStream for FakeLocalStream {
        fn authenticate(&self) -> Result<()> {
            if self.authenticated {
                Ok(())
            } else {
                Err(Error::new("PEER_REJECTED", "test peer"))
            }
        }
        fn read_frame(&mut self, maximum: usize) -> Result<Vec<u8>> {
            assert!(self.request.len() <= maximum);
            Ok(self.request.clone())
        }
        fn write_frame(&mut self, bytes: &[u8]) -> Result<()> {
            self.output.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        }
    }
    #[test]
    fn common_dispatch_checks_peer_then_persists_stop_all_and_recovery() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let manager = crate::runtime::lifecycle::Manager::open(&data).unwrap();
        let slots = Arc::new(AtomicUsize::new(0));
        let output = Arc::new(Mutex::new(vec![]));
        let request = serde_json::to_vec(
            &json!({"version":1,"request_id":"fake","method":"stop-all","params":{}}),
        )
        .unwrap();
        let stream = |authenticated| FakeLocalStream {
            authenticated,
            request: request.clone(),
            output: output.clone(),
        };
        assert_eq!(
            handle(stream(false), &data, &json!({}), &manager, &slots)
                .unwrap_err()
                .code,
            "PEER_REJECTED"
        );
        assert!(
            !manager.status(None).unwrap()["automatic_wake_paused"]
                .as_bool()
                .unwrap()
        );
        assert!(output.lock().unwrap().is_empty());
        handle(stream(true), &data, &json!({}), &manager, &slots).unwrap();
        let response: Value = serde_json::from_slice(&output.lock().unwrap()).unwrap();
        assert_eq!(response["request_id"], "fake");
        assert_eq!(manager.status(None).unwrap()["automatic_wake_paused"], true);
        drop(manager);
        let recovered = crate::runtime::lifecycle::Manager::open(&data).unwrap();
        assert_eq!(
            recovered.status(None).unwrap()["automatic_wake_paused"],
            true
        );
        assert_eq!(slots.load(Ordering::Relaxed), 0);
    }
}
