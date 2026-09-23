use super::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

pub(crate) struct Incident {
    pub failure: Failure,
    pub services: Vec<(String, String)>,
    pub log_incomplete: bool,
    pub recovered: bool,
}
enum Message {
    Incident(Box<Incident>),
    Success(Uuid, u128),
}
pub(crate) struct Recorder {
    sender: mpsc::SyncSender<Message>,
    warnings: Arc<Mutex<Vec<Value>>>,
}
fn now() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn warn(warnings: &Mutex<Vec<Value>>, id: Uuid, error: &Error) {
    let mut warnings = warnings.lock().unwrap_or_else(|e| e.into_inner());
    if warnings.len() == 32 {
        warnings.remove(0);
    }
    warnings.push(json!({"code":"FAILURE_CAPTURE_INCOMPLETE","level":"warning","title":"Failure evidence could not be fully saved",
        "evidence":{"installation_id":id,"error_code":error.code},"actions":[],"clears_when":"next daemon session"}));
}
impl Recorder {
    pub fn open(data: PathBuf) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(64);
        let warnings = Arc::new(Mutex::new(vec![]));
        let errors = warnings.clone();
        thread::Builder::new()
            .name("apphost-failures".into())
            .spawn(move || {
                let mut pending: Vec<(Instant, Incident)> = vec![];
                loop {
                    let timeout = pending
                        .iter()
                        .map(|(at, _)| at.saturating_duration_since(Instant::now()))
                        .min();
                    let message = match timeout {
                        Some(timeout) => receiver.recv_timeout(timeout),
                        None => receiver
                            .recv()
                            .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                    };
                    match message {
                        Ok(Message::Success(id, at)) => {
                            if let Err(e) = store::success(&data, id, at) {
                                warn(&errors, id, &e);
                            }
                        }
                        Ok(Message::Incident(mut job)) => {
                            let id = job.failure.installation_id;
                            if job.recovered
                                && let Ok(saved) =
                                    super::read(&data, id, Some(job.failure.failure_id))
                                && let Ok(failure) =
                                    serde_json::from_value::<Failure>(saved["failure"].clone())
                            {
                                job.failure = failure;
                            }
                            let deadline = job.failure.at_ms.saturating_add(10000);
                            job.failure.collecting = job.failure.instance_id.is_some()
                                && deadline > now()
                                && pending.len() < 32;
                            if job.recovered {
                                job.failure.gaps.push("daemon_observation_gap".into());
                            }
                            if !job.failure.collecting
                                && deadline > now()
                                && job.failure.instance_id.is_some()
                            {
                                job.failure.gaps.push("capture_capacity_reached".into());
                            }
                            collect(&data, &mut job);
                            if let Err(e) = store::save(&data, &job.failure, now()) {
                                warn(&errors, id, &e);
                            }
                            if job.failure.collecting {
                                pending.push((
                                    Instant::now()
                                        + Duration::from_millis(
                                            deadline.saturating_sub(now()).min(10000) as u64,
                                        ),
                                    *job,
                                ));
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    let mut i = 0;
                    while i < pending.len() {
                        if pending[i].0 > Instant::now() {
                            i += 1;
                            continue;
                        }
                        let (_, mut job) = pending.remove(i);
                        job.failure.collecting = false;
                        collect(&data, &mut job);
                        if let Err(e) = store::save(&data, &job.failure, now()) {
                            warn(&errors, job.failure.installation_id, &e);
                        }
                    }
                }
            })
            .map_err(|e| Error::new("RUNTIME_LIMIT", e.to_string()))?;
        Ok(Self { sender, warnings })
    }
    pub fn incident(&self, job: Incident) {
        let id = job.failure.installation_id;
        self.send(Message::Incident(Box::new(job)), id);
    }
    pub fn success(&self, id: Uuid, at: u128) {
        self.send(Message::Success(id, at), id);
    }
    fn send(&self, message: Message, id: Uuid) {
        if self.sender.try_send(message).is_err() {
            warn(
                &self.warnings,
                id,
                &Error::new("CAPTURE_BUSY", "Failure recorder capacity reached"),
            );
        }
    }
    pub fn warnings(&self) -> Vec<Value> {
        self.warnings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

fn sanitize(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(s) => *s = redact(s, secrets),
        Value::Array(a) => a.iter_mut().for_each(|v| sanitize(v, secrets)),
        Value::Object(o) => o.values_mut().for_each(|v| sanitize(v, secrets)),
        _ => {}
    }
}
fn collect(data: &Path, job: &mut Incident) {
    let secrets =
        match crate::registry::confirmed_environment_values(data, job.failure.installation_id) {
            Ok(values) => values,
            Err(_) => {
                job.failure
                    .gaps
                    .push("redaction_environment_unavailable".into());
                vec![]
            }
        };
    sanitize(&mut job.failure.details, &secrets);
    let Some(instance) = job.failure.instance_id else {
        return;
    };
    let root = data
        .join("instances")
        .join(instance.to_string())
        .join("logs");
    // The collector may fail after the first snapshot. Re-read its last durable
    // observation without taking an installation lock or changing runtime state.
    let record_path = data
        .join("runtime")
        .join(format!("{}.json", job.failure.installation_id));
    if let Ok(bytes) = storage::read(&record_path)
        && let Ok(record) = serde_json::from_slice::<Value>(&bytes)
        && record["instance_id"] == json!(instance)
        && record["status"]["log_incomplete"] == true
    {
        job.log_incomplete = true;
    }
    let to = job.failure.at_ms.saturating_add(10000).min(now());
    match crate::logs::read_tail(
        &root,
        job.failure.at_ms.saturating_sub(30000),
        to,
        64 * 1024,
    ) {
        Ok(window) => {
            // Assemble raw blocks per stream before matching/redacting: an error or
            // credential can cross a 4096-byte collector boundary.
            let mut streams: BTreeMap<(String, String), (u64, u128, Vec<u8>)> = BTreeMap::new();
            for chunk in &window.chunks {
                let stream = if chunk.stream == crate::logs::Stream::Stderr {
                    "stderr"
                } else {
                    "stdout"
                };
                let name = job
                    .services
                    .iter()
                    .find(|(_, key)| *key == chunk.service_key)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| chunk.service_key.clone());
                let entry = streams.entry((name, stream.into())).or_insert((
                    chunk.sequence,
                    chunk.observed_ms,
                    vec![],
                ));
                entry.2.extend_from_slice(&chunk.bytes);
            }
            let mut stderr = String::new();
            let mut output = vec![];
            for ((service, stream), (sequence, observed_ms, bytes)) in streams {
                let text = String::from_utf8_lossy(&bytes);
                if stream == "stderr" && job.failure.service.as_ref().is_none_or(|s| *s == service)
                {
                    stderr.push_str(&text);
                    stderr.push('\n');
                }
                output.push(Output {
                    snapshot: if job.failure.collecting {
                        "initial"
                    } else {
                        "tail"
                    }
                    .into(),
                    sequence,
                    observed_ms,
                    service,
                    stream,
                    text: redact(&text, &secrets),
                });
            }
            // stdout floods must not erase stderr evidence from the same window.
            match crate::logs::read_stderr_tail(
                &root,
                job.failure.at_ms.saturating_sub(30000),
                to,
                32 * 1024,
                job.failure.service.as_ref().and_then(|name| {
                    job.services
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, k)| k.as_str())
                }),
            ) {
                Ok(tail) => {
                    let mut bytes = vec![];
                    for c in tail.chunks {
                        let matching = job.failure.service.as_ref().is_none_or(|name| {
                            job.services
                                .iter()
                                .any(|(n, k)| n == name && *k == c.service_key)
                        });
                        if matching {
                            bytes.extend(c.bytes);
                        }
                    }
                    stderr = String::from_utf8_lossy(&bytes).into_owned();
                    if tail.truncated {
                        job.failure.gaps.push("stderr_tail_truncated".into());
                    }
                }
                Err(_) => job.failure.gaps.push("stderr_tail_unavailable".into()),
            }
            let mut evidence = classify(&job.failure.cause_code, &job.failure.phase, &stderr);
            evidence.excerpt = evidence.excerpt.map(|s| redact(&s, &secrets));
            if evidence.inferred || !job.failure.evidence.inferred {
                job.failure.evidence = evidence;
            }
            job.failure.stderr_tail = redact(
                &stderr
                    .chars()
                    .rev()
                    .take(8192)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>(),
                &secrets,
            );
            // Keep the immediate snapshot even if rotation removes its blocks.
            // These are labelled snapshots, not a fabricated duplicate-free log.
            if job.failure.output.is_empty() {
                job.failure.output = output;
            } else if !job.failure.collecting {
                job.failure.output.retain(|o| o.snapshot == "initial");
                job.failure.output.extend(output);
            }
            job.failure.window = json!({"from_ms":job.failure.at_ms.saturating_sub(30000),"to_ms":job.failure.at_ms.saturating_add(10000),
                "observed_through_ms":to,"prefix_evicted":window.prefix_evicted,"incomplete_tail":window.incomplete_tail,
                "truncated":window.truncated,"sequence_gap":window.sequence_gap,"clock_reversed":window.clock_reversed,"warnings":window.warnings});
            if window.prefix_evicted
                || window.incomplete_tail
                || window.truncated
                || window.sequence_gap
                || window.clock_reversed
            {
                job.failure
                    .gaps
                    .push("bounded_log_window_incomplete".into());
            }
        }
        Err(_) => job.failure.gaps.push("log_window_unavailable".into()),
    }
    if job.log_incomplete {
        job.failure.gaps.push("collector_observation_gap".into());
    }
    job.failure.gaps.sort();
    job.failure.gaps.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::{LogWriter, Stream};
    #[test]
    fn async_tail_finishes_and_keeps_split_stderr_after_stdout_flood() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        storage::private_directory(&data).unwrap();
        let id = Uuid::new_v4();
        let instance = Uuid::new_v4();
        let at = now().saturating_sub(9800);
        let root = data
            .join("instances")
            .join(instance.to_string())
            .join("logs");
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        let mut writer = LogWriter::create(root.clone(), 1024 * 1024, 2).unwrap();
        writer
            .append("s0", Stream::Stderr, b"ModuleNotFound")
            .unwrap();
        writer
            .append(
                "s0",
                Stream::Stderr,
                b"Error: no module 'gradio'\nAuthorization: Bearer split-private-token\n",
            )
            .unwrap();
        for _ in 0..40 {
            writer
                .append("s0", Stream::Stdout, &vec![b'x'; 4096])
                .unwrap();
            writer
                .append("s1", Stream::Stderr, &vec![b'y'; 4096])
                .unwrap();
        }
        drop(writer);
        let recorder = Recorder::open(data.clone()).unwrap();
        let mut failure = Failure::new(id, "SERVICE_EXITED", at);
        failure.instance_id = Some(instance);
        failure.service = Some("web".into());
        failure.phase = "startup".into();
        recorder.incident(Incident {
            failure,
            services: vec![("web".into(), "s0".into())],
            log_incomplete: false,
            recovered: false,
        });
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(value) = super::super::read(&data, id, None)
                && value["failure"]["collecting"] == false
                && !value["failure"]["gaps"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("tail_capture_not_completed"))
            {
                assert_eq!(value["failure"]["evidence"]["code"], "DEPENDENCY_MISSING");
                assert!(!value.to_string().contains("split-private-token"));
                assert!(
                    !value["failure"]["gaps"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("tail_capture_not_completed"))
                );
                break;
            }
            assert!(Instant::now() < end, "{:?}", recorder.warnings());
            thread::sleep(Duration::from_millis(10));
        }
        std::fs::remove_dir_all(data.join("instances")).unwrap();
        assert_eq!(
            super::super::read(&data, id, None).unwrap()["failure"]["evidence"]["code"],
            "DEPENDENCY_MISSING"
        );
    }
}
