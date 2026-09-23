//! Process ownership, readiness and explicit stop execution for one installation.
use super::*;

pub(super) struct Live {
    groups: Vec<Option<Group>>,
    children: Vec<Option<Child>>,
    collector: Option<Group>,
    collector_child: Option<Child>,
}
impl Live {
    pub(super) fn empty() -> Self {
        Self {
            groups: vec![],
            children: vec![],
            collector: None,
            collector_child: None,
        }
    }
    pub(super) fn from_record(r: &Record) -> Self {
        Self {
            groups: r.services.iter().map(|s| s.group.clone()).collect(),
            children: (0..r.services.len()).map(|_| None).collect(),
            collector: r.collector.clone(),
            collector_child: None,
        }
    }
    pub(super) fn any(&self) -> bool {
        self.groups.iter().any(Option::is_some) || self.collector.is_some()
    }
    pub(super) fn reconcile(&mut self, data: &Path, r: &mut Record) {
        r.management_uncertain = false;
        for (i, group) in self.groups.iter_mut().enumerate() {
            match group.as_mut().map(Group::reconcile) {
                Some(Ok(members)) => {
                    r.services[i].alive = !members.is_empty();
                    if members.is_empty() {
                        *group = None;
                        r.services[i].ready = false;
                    }
                }
                Some(Err(_)) => {
                    r.management_uncertain = true;
                    r.status.fail("MANAGEMENT_UNCERTAIN");
                }
                None => {
                    if r.services[i].spawn_attempted && r.services[i].exit.is_none() {
                        r.management_uncertain = true;
                        r.status.fail("MANAGEMENT_UNCERTAIN");
                    }
                }
            }
        }
        match self.collector.as_mut().map(Group::reconcile) {
            Some(Ok(members)) => {
                r.collector_alive = !members.is_empty();
                if members.is_empty() {
                    self.collector = None;
                }
            }
            Some(Err(_)) => {
                r.management_uncertain = true;
                r.status.fail("MANAGEMENT_UNCERTAIN");
            }
            None if r.collector_attempted && r.collector_alive => {
                r.management_uncertain = true;
                r.status.fail("MANAGEMENT_UNCERTAIN");
            }
            _ => {}
        }
        // Readiness is always observed again, never trusted from disk.
        if r.status.state() == State::Running {
            r.status.state = State::Starting;
        }
        if !r.collector_alive
            && r.services.iter().any(|s| s.alive)
            && !log_path(data, r).with_extension("complete").is_file()
        {
            r.status.collector_exited(false);
        }
        if r.services.iter().any(|s| s.alive) && r.services.iter().any(|s| !s.alive) {
            r.status.fail("DAEMON_INTERRUPTED");
        }
    }
}
fn lock_dispatch(lock: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    // This gate contains no mutable data. A panic in one installation must not
    // poison dispatch for every other installation; its own record stays separate.
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
fn spawn(manager: &Manager, h: &Handle, plan: LaunchPlan, live: &mut Live) -> Result<()> {
    if h.control.load(Ordering::Acquire) != 0 {
        return Ok(());
    }
    let data = &manager.data;
    let (root, count) = {
        let r = h.record.lock().unwrap();
        (log_path(data, &r), r.services.len())
    };
    let (id, instance) = {
        let r = h.record.lock().unwrap();
        (r.installation_id, r.instance_id)
    };
    let warnings = retention::prune(data, id, instance)?;
    h.record.lock().unwrap().warnings = warnings;
    storage::private_directory(root.parent().unwrap())?;
    {
        let mut r = h.record.lock().unwrap();
        r.collector_attempted = true;
        r.collector_alive = true;
        save_record(data, &mut r)?;
    }
    let pipes = collector::spawn(&root, count);
    let mut pipes = match pipes {
        Ok(p) => p,
        Err(e) => {
            let mut r = h.record.lock().unwrap();
            r.collector_attempted = false;
            r.collector_alive = false;
            return Err(e);
        }
    };
    live.collector_child = Some(pipes.child);
    live.collector = Some(Group::capture_spawned(
        live.collector_child.as_ref().unwrap(),
    )?);
    {
        let mut r = h.record.lock().unwrap();
        r.collector = live.collector.clone();
        save_record(data, &mut r)?;
    }
    collector::ready(live.collector_child.as_mut().unwrap())?;
    live.groups = (0..count).map(|_| None).collect();
    live.children = (0..count).map(|_| None).collect();
    plan.with_current(data, |services| {
        // Preparation, cleanup, collector readiness and registry-lock waiting
        // are complete. Only spawn and its identity checkpoints hold this gate.
        let _dispatch = lock_dispatch(&manager.launch_dispatch);
        for (i, (service, (out, err))) in services
            .into_iter()
            .zip(pipes.outputs.drain(..))
            .enumerate()
        {
            let mut r = h.record.lock().unwrap();
            if h.control.load(Ordering::Acquire) != 0 {
                break;
            }
            r.services[i].spawn_attempted = true;
            save_record(data, &mut r)?;
            let child = process::spawn_group(
                service
                    .into_command()
                    .stdin(Stdio::null())
                    .stdout(out)
                    .stderr(err),
            );
            let child = match child {
                Ok(c) => c,
                Err(e) => {
                    r.services[i].spawn_attempted = false;
                    if r.status.failure.is_none() {
                        r.failure_service = Some(r.services[i].name.clone());
                    }
                    return Err(e);
                }
            };
            live.children[i] = Some(child);
            live.groups[i] = Some(Group::capture_spawned(live.children[i].as_ref().unwrap())?);
            r.services[i].group = live.groups[i].clone();
            r.services[i].alive = true;
            save_record(data, &mut r)?;
        }
        Ok(())
    })
}
fn stop_groups(data: &Path, h: &Handle, live: &mut Live) -> Result<bool> {
    let mut errors = Vec::new();
    for i in 0..live.groups.len() {
        if let Some(group) = &mut live.groups[i] {
            let grace = h.record.lock().unwrap().services[i].stop_timeout_ms;
            match group.stop(Duration::from_millis(grace)) {
                Ok(_) => {
                    if let Some(mut child) = live.children[i].take() {
                        let _ = child.wait();
                    }
                    let checkpoint = group.clone();
                    live.groups[i] = None;
                    let mut r = h.record.lock().unwrap();
                    r.services[i].alive = false;
                    r.services[i].ready = false;
                    r.services[i].group = Some(checkpoint);
                    r.services[i]
                        .exit
                        .get_or_insert(json!({"reason":"explicit_stop"}));
                }
                Err(e) => errors.push(json!(e)),
            }
        }
    }
    if let Some(group) = &mut live.collector {
        // Give EOF a bounded chance to flush before explicitly stopping the
        // collector. No wait/reap occurs while a group is still nonempty.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !group.refresh()?.is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if !group.refresh()?.is_empty() {
            h.record.lock().unwrap().status.log_incomplete = true;
        }
        match group.stop(Duration::from_secs(1)) {
            Ok(_) => {
                if let Some(mut child) = live.collector_child.take() {
                    let _ = child.wait();
                }
                let mut r = h.record.lock().unwrap();
                r.collector = Some(group.clone());
                r.collector_alive = false;
                if !log_path(data, &r).with_extension("complete").is_file() {
                    r.status.log_incomplete = true;
                }
                live.collector = None;
            }
            Err(e) => errors.push(json!(e)),
        }
    }
    let mut r = h.record.lock().unwrap();
    // Missing spawn identities cannot be cleared by a successful stop of the
    // other groups. They continue to block restart rather than risk duplication.
    if errors.is_empty() && !r.management_uncertain {
        r.status.finish_stop(true);
        r.operation_done = false;
    } else {
        r.status.state = State::Failed;
        r.operation_done = true;
        r.operation_error = Some(
            json!({"code":"STOP_FAILED","groups":errors,"management_uncertain":r.management_uncertain}),
        );
    }
    save_record(data, &mut r)?;
    Ok(r.status.state() == State::Stopped)
}
pub(super) fn run(
    manager: &Arc<Manager>,
    h: &Arc<Handle>,
    mut plan: Option<LaunchPlan>,
    live: &mut Live,
) -> Result<()> {
    let mut started = Instant::now();
    let mut misses: [Option<Instant>; 32] = [None; 32];
    let mut next_observation = Instant::now();
    loop {
        if let Some(prepared) = plan.take()
            && h.control.load(Ordering::Acquire) == 0
        {
            let result = spawn(manager, h, prepared, live);
            started = Instant::now();
            if let Err(error) = result {
                let mut r = h.record.lock().unwrap();
                r.status.fail(&error.code);
                r.operation_done = true;
                r.operation_error = Some(json!(error));
                if error.code == "MANAGEMENT_UNCERTAIN" {
                    r.management_uncertain = true;
                }
                save_record(&manager.data, &mut r)?;
            }
        }
        let control = h.control.load(Ordering::Acquire);
        if control != 0 {
            let epoch = manager.stop_epoch.load(Ordering::Acquire);
            let stopped = stop_groups(&manager.data, h, live)?;
            let restart_still_requested = h.control.swap(0, Ordering::AcqRel) == 2;
            if stopped {
                let mut r = h.record.lock().unwrap();
                if let Some(mutation) = r.pending_mutation.take() {
                    match mutation.apply(&manager.data, r.installation_id) {
                        Ok(result) => r.operation_result = Some(result),
                        Err(error) => r.operation_error = Some(json!(error)),
                    }
                }
                r.operation_done = !(control == 2 && restart_still_requested);
                save_record(&manager.data, &mut r)?;
            } else {
                let mut r = h.record.lock().unwrap();
                r.pending_mutation = None;
                r.restart_after_stop = false;
            }
            if control == 2 && restart_still_requested && stopped {
                let (id, op) = {
                    let r = h.record.lock().unwrap();
                    (r.installation_id, r.operation_id)
                };
                let prepared = LaunchPlan::prepare(&manager.data, id)?;
                let handles = manager.handles.lock().unwrap();
                if prepared.manifest().services.len() > 32 {
                    return Err(Error::new(
                        "RUNTIME_LIMIT",
                        "At most 32 services per instance",
                    ));
                }
                if epoch != manager.stop_epoch.load(Ordering::Acquire)
                    || h.control.load(Ordering::Acquire) != 0
                {
                    return Err(Error::new(
                        "START_CANCELLED",
                        "StopAll cancelled the restart",
                    ));
                }
                for service in &prepared.manifest().services {
                    if let Some(port) = service.port {
                        for (other, handle) in handles.iter() {
                            if *other != id {
                                let record = handle.record.lock().unwrap();
                                if record.services.iter().any(|s| {
                                    s.port == Some(port)
                                        && (s.alive || record.status.state() == State::Starting)
                                }) {
                                    return Err(Error::new(
                                        "PORT_IN_USE",
                                        "Port reserved by another managed installation",
                                    ));
                                }
                            }
                        }
                    }
                }
                for s in &prepared.manifest().services {
                    if let Some(port) = s.port {
                        port_available(port)?;
                    }
                }
                {
                    let mut r = h.record.lock().unwrap();
                    retention::archive(&manager.data, &r)?;
                    let operation = r.audit.take();
                    let pending = std::mem::take(&mut r.journal_pending);
                    *r = new_record(id, op, &prepared, epoch);
                    r.audit = operation;
                    r.journal_pending = pending;
                    save_record(&manager.data, &mut r)?;
                }
                drop(handles);
                *live = Live::empty();
                plan = Some(prepared);
                started = Instant::now();
                misses.fill(None);
                next_observation = Instant::now();
                continue;
            }
            if stopped || !live.any() {
                return Ok(());
            }
        }
        // Keep control latency short without scanning process FDs or serializing
        // records ten times a second once startup has finished.
        if Instant::now() < next_observation {
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let before = encode(&*h.record.lock().unwrap())?;
        let mut all_ready = true;
        for (i, miss) in misses.iter_mut().enumerate().take(live.groups.len()) {
            let Some(group) = &mut live.groups[i] else {
                all_ready = false;
                continue;
            };
            let members = match group.refresh() {
                Ok(m) => m,
                Err(e) => {
                    let mut r = h.record.lock().unwrap();
                    r.management_uncertain = true;
                    r.status.fail(&e.code);
                    r.operation_error = Some(json!(e));
                    all_ready = false;
                    continue;
                }
            };
            let mut exit = None;
            if let Some(child) = &live.children[i]
                && let Some(status) = process::observe_exit(child)?
            {
                use std::os::unix::process::ExitStatusExt;
                exit = Some(json!({"code":status.code(),"signal":status.signal()}));
            }
            let (port, starting) = {
                let r = h.record.lock().unwrap();
                (r.services[i].port, r.status.state() == State::Starting)
            };
            let ready = if members.is_empty() {
                false
            } else if let Some(port) = port {
                let observed = if starting {
                    group.port_ready(port)
                } else {
                    Group::port_listening(&members, port)
                };
                match observed {
                    Ok(v) => v,
                    Err(e) => {
                        let mut r = h.record.lock().unwrap();
                        r.management_uncertain = true;
                        r.status.fail(&e.code);
                        false
                    }
                }
            } else {
                started.elapsed() >= Duration::from_secs(2)
            };
            let mut r = h.record.lock().unwrap();
            let was_running = r.status.state() == State::Running;
            if ready {
                *miss = None;
            } else if was_running {
                miss.get_or_insert_with(Instant::now);
            }
            r.services[i].alive = !members.is_empty();
            r.services[i].ready = ready;
            r.services[i].group = Some(group.clone());
            if let Some(exit) = exit {
                if r.status.failure.is_none() {
                    r.failure_service = Some(r.services[i].name.clone());
                }
                r.services[i].exit = Some(exit);
                r.status.fail("SERVICE_EXITED");
            }
            if members.is_empty() {
                if r.status.failure.is_none() {
                    r.failure_service = Some(r.services[i].name.clone());
                }
                r.services[i]
                    .exit
                    .get_or_insert(json!({"reason":"exit_status_unavailable"}));
                r.status.fail("SERVICE_EXITED");
                live.groups[i] = None;
                if let Some(mut child) = live.children[i].take() {
                    let _ = child.wait();
                }
            }
            if !ready {
                all_ready = false;
                if was_running
                    && miss.is_some_and(|since| since.elapsed() >= Duration::from_secs(10))
                {
                    if r.status.failure.is_none() {
                        r.failure_service = Some(r.services[i].name.clone());
                    }
                    r.status.fail("PORT_NOT_READY");
                } else if r.status.state() == State::Starting
                    && started.elapsed() >= Duration::from_millis(r.services[i].startup_timeout_ms)
                {
                    if r.status.failure.is_none() {
                        r.failure_service = Some(r.services[i].name.clone());
                    }
                    r.status.fail("READINESS_TIMEOUT");
                }
            }
        }
        if let Some(group) = &mut live.collector {
            match group.refresh() {
                Ok(members) => {
                    let mut r = h.record.lock().unwrap();
                    r.collector = Some(group.clone());
                    r.collector_alive = !members.is_empty();
                    if members.is_empty() {
                        let completed = log_path(&manager.data, &r)
                            .with_extension("complete")
                            .is_file();
                        r.status.collector_exited(completed);
                        if let Some(mut child) = live.collector_child.take() {
                            let _ = child.wait();
                        }
                        live.collector = None;
                    }
                }
                Err(e) => {
                    let mut r = h.record.lock().unwrap();
                    r.management_uncertain = true;
                    r.status.collector_exited(false);
                    r.operation_error = Some(json!(e));
                }
            }
        }
        {
            let mut r = h.record.lock().unwrap();
            if all_ready
                && (r.collector_alive
                    || log_path(&manager.data, &r)
                        .with_extension("complete")
                        .is_file())
            {
                r.status.all_ready();
                if r.status.state() == State::Running && r.reached_running_at_ms.is_none() {
                    let at = now();
                    r.reached_running_at_ms = Some(at);
                    manager.failures.success(r.installation_id, at);
                }
            }
            if matches!(r.status.state(), State::Running | State::Failed) {
                r.operation_done = true;
            }
            if encode(&*r)? != before {
                r.updated_at_ms = now();
                save_record(&manager.data, &mut r)?;
            }
        }
        manager.capture_failure(h, false);
        if !live.any() {
            return Ok(());
        }
        let interval = if h.record.lock().unwrap().status.state() == State::Starting {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        };
        next_observation = Instant::now() + interval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisoned_dispatch_gate_does_not_block_other_installations() {
        let gate = Mutex::new(());
        let panic = std::panic::catch_unwind(|| {
            let _guard = gate.lock().unwrap();
            panic!("simulated installation panic");
        });
        assert!(panic.is_err());
        let _guard = lock_dispatch(&gate);
    }

    #[test]
    fn busy_dispatch_gate_does_not_block_collector_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        let data = temp.path().join("data");
        let manifest: crate::Manifest = serde_json::from_value(json!({
            "schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture",
            "services":[{"name":"service","command":["/bin/sleep","1"]}]
        }))
        .unwrap();
        fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id = crate::import(&data, &crate::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        let plan = LaunchPlan::prepare(&data, id).unwrap();
        let manager = Manager::open(&data).unwrap();
        let h = Arc::new(Handle {
            record: Mutex::new(new_record(id, Uuid::new_v4(), &plan, 0)),
            control: AtomicU8::new(0),
            working: AtomicBool::new(false),
            parked: Mutex::new(None),
        });
        let gate = manager.launch_dispatch.lock().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let other = manager.clone();
        let thread = thread::spawn(move || {
            let mut live = Live::empty();
            // The library test executable cannot serve the hidden collector command.
            // Its preparation failure must be returned even while dispatch is busy.
            let result = spawn(&other, &h, plan, &mut live);
            sender.send(result.is_err()).unwrap();
            h.record.lock().unwrap().status.request_stop();
            stop_groups(&other.data, &h, &mut live).unwrap();
        });
        let prepared = receiver.recv_timeout(Duration::from_secs(3));
        drop(gate);
        thread.join().unwrap();
        assert!(prepared.unwrap());
    }
}
