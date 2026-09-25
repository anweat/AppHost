use super::*;

/// Only the public facts needed by the gateway's loading page. In particular,
/// operation_error may contain source paths and must not cross the HTTP boundary.
#[derive(Debug, Serialize)]
pub(crate) struct Observation {
    pub operation_id: Uuid,
    pub state: State,
    pub ready: bool,
    pub done: bool,
    pub cause: Option<String>,
}

impl Manager {
    // The HTTP adapter must first validate loopback, Host and navigation metadata.
    // Keep this out of the management RPC method table; a browser never selects
    // arbitrary installation IDs or invokes other lifecycle operations.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn browser_wake(self: &Arc<Self>, id: Uuid) -> Result<Observation> {
        let accepted = self.start_inner(id, Actor::System, "browser-wake", None)?;
        let op = serde_json::from_value(accepted["operation_id"].clone())
            .map_err(|_| invalid("Missing browser wake operation identity"))?;
        self.browser_observe(id, op)
    }

    pub(crate) fn browser_observe(&self, id: Uuid, op: Uuid) -> Result<Observation> {
        let handles = self.handles.lock().unwrap();
        let h = handles.get(&id).ok_or_else(expired)?;
        let r = h.record.lock().unwrap();
        if r.operation_id != op {
            return Err(expired());
        }
        Ok(Observation {
            operation_id: op,
            state: r.status.state(),
            ready: r.status.state() == State::Running
                && !r.management_uncertain
                && r.services.iter().all(|s| s.alive && s.ready),
            done: r.operation_done,
            cause: r.status.failure.clone(),
        })
    }

    pub(super) fn browser_existing(
        &self,
        handles: &BTreeMap<Uuid, Arc<Handle>>,
        id: Uuid,
        epoch: u64,
    ) -> Result<Option<Value>> {
        // Check the epoch even after resume-wake: unpausing cannot revive a
        // request that was preparing when Stop All cancelled it.
        if epoch != self.stop_epoch.load(Ordering::Acquire) {
            return Err(Error::new(
                "START_CANCELLED",
                "StopAll cancelled this pending wake",
            ));
        }
        if self.paused.load(Ordering::Acquire) {
            return Err(Error::new("WAKE_PAUSED", "Automatic wake is paused"));
        }
        let Some(h) = handles.get(&id) else {
            return Ok(None);
        };
        let r = h.record.lock().unwrap();
        // Stop retains the failure cause. Only a new explicit start/restart
        // replaces that record; resuming wake alone is never a retry.
        if r.status.state() == State::Failed
            || r.status
                .failure
                .as_deref()
                .is_some_and(|code| code != "START_CANCELLED")
            || (matches!(r.status.state(), State::Starting | State::Running)
                && h.working.load(Ordering::Acquire))
        {
            return Ok(Some(receipt(&r)));
        }
        Ok(None)
    }
}

fn expired() -> Error {
    Error::new(
        "OPERATION_EXPIRED",
        "The observed operation has been replaced",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _temp: tempfile::TempDir,
        data: PathBuf,
        root: PathBuf,
        id: Uuid,
        manager: Arc<Manager>,
    }
    impl Fixture {
        fn new(command: &str, port: Option<u16>) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let data = temp.path().join("data");
            let root = temp.path().join("app");
            fs::create_dir(&root).unwrap();
            let manifest: crate::Manifest = serde_json::from_value(json!({
                "schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture",
                "services":[{"name":"service","command":[command,"1"],"port":port}]
            }))
            .unwrap();
            fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
            let id =
                crate::import(&data, &crate::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
            let manager = Manager::open(&data).unwrap();
            Self {
                _temp: temp,
                data,
                root,
                id,
                manager,
            }
        }
        fn record(&self) -> Record {
            self.manager.handles.lock().unwrap()[&self.id]
                .record
                .lock()
                .unwrap()
                .clone()
        }
    }

    #[test]
    fn failed_preflight_is_sticky_across_refresh_stop_resume_and_recovery() {
        let mut f = Fixture::new("apphost-fixture-command-that-does-not-exist", None);
        let failed = f.manager.browser_wake(f.id).unwrap();
        assert_eq!(failed.state, State::Failed);
        assert_eq!(failed.cause.as_deref(), Some("INVALID_COMMAND"));
        assert!(failed.done && !failed.ready);
        let record = f.record();
        assert!(!record.collector_attempted);
        assert!(record.services.iter().all(|s| !s.spawn_attempted));
        assert_eq!(record.audit.as_ref().unwrap().action, "browser-wake");
        assert_eq!(record.audit.as_ref().unwrap().actor, Actor::System);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(evidence) = crate::failure::read(&f.data, f.id, record.failure_id)
                && !evidence["failure"].is_null()
            {
                assert_eq!(evidence["failure"]["phase"], "preflight");
                assert_eq!(evidence["failure"]["collecting"], false);
                assert!(evidence["failure"]["instance_id"].is_null());
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            f.manager.browser_wake(f.id).unwrap().operation_id,
            failed.operation_id
        );
        // No worker is active for a preflight failure. Reopen from durable state.
        f.manager = Manager::open(&f.data).unwrap();
        assert_eq!(
            f.manager.browser_wake(f.id).unwrap().operation_id,
            failed.operation_id
        );
        f.manager.stop(f.id, false, Actor::Human).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while f.manager.handles.lock().unwrap()[&f.id]
            .working
            .load(Ordering::Acquire)
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(f.record().status.state(), State::Stopped);
        f.manager.resume_wake().unwrap();
        assert_eq!(
            f.manager.browser_wake(f.id).unwrap().cause.as_deref(),
            Some("INVALID_COMMAND")
        );
        assert_eq!(f.record().instance_id, record.instance_id);
        // Explicit retry really reaches preflight, rather than merging the old
        // failure. CLI keeps its original synchronous error contract.
        assert_eq!(
            f.manager.start(f.id, Actor::Human).unwrap_err().code,
            "INVALID_COMMAND"
        );
        let path = f.root.join("apphost.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("apphost-fixture-command-that-does-not-exist", "/bin/sleep"),
        )
        .unwrap();
        let mut draft = crate::scan(&f.root, 0).unwrap();
        crate::registry::annotate_launch_changes(&f.data, &mut draft).unwrap();
        crate::review::review(&mut draft, &mut "y\n".as_bytes(), &mut Vec::new()).unwrap();
        crate::import(&f.data, &draft).unwrap();
        let retry = f.manager.start(f.id, Actor::Human).unwrap();
        assert_ne!(retry["instance_id"], json!(record.instance_id));
        assert_ne!(retry["operation_id"], json!(failed.operation_id));
        assert_eq!(
            f.manager
                .browser_observe(f.id, failed.operation_id)
                .unwrap_err()
                .code,
            "OPERATION_EXPIRED"
        );
        f.manager.stop(f.id, false, Actor::Human).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while f.manager.handles.lock().unwrap()[&f.id]
            .working
            .load(Ordering::Acquire)
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn concurrent_navigation_retains_one_port_failure_after_the_port_is_freed() {
        let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
        let f = Fixture::new("/bin/sleep", Some(occupied.local_addr().unwrap().port()));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let m = f.manager.clone();
                let b = barrier.clone();
                let id = f.id;
                thread::spawn(move || {
                    b.wait();
                    m.browser_wake(id).unwrap()
                })
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(
            results
                .iter()
                .all(|r| r.operation_id == results[0].operation_id
                    && r.cause.as_deref() == Some("PORT_IN_USE"))
        );
        drop(occupied);
        assert_eq!(
            f.manager.browser_wake(f.id).unwrap().operation_id,
            results[0].operation_id
        );
        assert!(!f.record().collector_attempted);
        let events = journal::read(&f.data, None, 100).unwrap();
        let wake_events: Vec<_> = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["action"] == "browser-wake")
            .collect();
        assert_eq!(wake_events.len(), 2);
        assert_eq!(wake_events[0]["outcome"], "accepted");
        assert_eq!(wake_events[1]["outcome"], "failed");
    }

    #[test]
    fn stop_all_cancels_preparing_wake_even_after_resume() {
        let f = Fixture::new("/bin/sleep", None);
        let epoch = f.manager.stop_epoch.load(Ordering::Acquire);
        f.manager.stop_all(Actor::Human).unwrap();
        assert_eq!(
            f.manager.browser_wake(f.id).unwrap_err().code,
            "WAKE_PAUSED"
        );
        f.manager.resume_wake().unwrap();
        let handles = f.manager.handles.lock().unwrap();
        assert_eq!(
            f.manager
                .browser_existing(&handles, f.id, epoch)
                .unwrap_err()
                .code,
            "START_CANCELLED"
        );
        assert!(handles.is_empty());
    }

    #[test]
    fn unconfirmed_or_detached_sources_do_not_create_runtime_attempts() {
        let f = Fixture::new("/bin/sleep", None);
        let path = f.root.join("apphost.toml");
        let original = fs::read_to_string(&path).unwrap();
        fs::write(&path, original.replace("/bin/sleep", "/bin/echo")).unwrap();
        assert_eq!(
            f.manager.browser_wake(f.id).unwrap_err().code,
            "REVIEW_REQUIRED"
        );
        fs::write(&path, original).unwrap();
        crate::registry::detach(&f.data, f.id).unwrap();
        assert_eq!(f.manager.browser_wake(f.id).unwrap_err().code, "DETACHED");
        assert!(f.manager.handles.lock().unwrap().is_empty());
    }

    #[test]
    fn observation_is_read_only_redacted_and_bound_to_one_operation() {
        let f = Fixture::new("apphost-fixture-command-that-does-not-exist", None);
        assert_eq!(
            f.manager
                .browser_observe(f.id, Uuid::new_v4())
                .unwrap_err()
                .code,
            "OPERATION_EXPIRED"
        );
        assert!(f.manager.handles.lock().unwrap().is_empty());
        let failed = f.manager.browser_wake(f.id).unwrap();
        let public = serde_json::to_value(
            f.manager
                .browser_observe(f.id, failed.operation_id)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(public.as_object().unwrap().len(), 5);
        assert!(!public.to_string().contains(f.root.to_str().unwrap()));
        let handles = f.manager.handles.lock().unwrap();
        let h = &handles[&f.id];
        let mut r = h.record.lock().unwrap();
        r.operation_id = Uuid::new_v4();
        r.status = InstanceStatus::default();
        h.working.store(true, Ordering::Release);
        let new_op = r.operation_id;
        drop(r);
        drop(handles);
        assert_eq!(
            f.manager
                .browser_observe(f.id, failed.operation_id)
                .unwrap_err()
                .code,
            "OPERATION_EXPIRED"
        );
        assert_eq!(f.manager.browser_wake(f.id).unwrap().operation_id, new_op);
    }
}
