//! One bounded continuation for an explicitly selected managed port owner.
//! Never hold two installation locks while waiting for another worker.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOwner {
    pub installation_id: Uuid,
    pub instance_id: Uuid,
    pub original_operation_id: Uuid,
    pub stop_operation_id: Uuid,
    pub target_revision: u64,
    pub expires_at_ms: u128,
    pub ports: Vec<u16>,
    pub stopped: bool,
}
pub(super) fn owner_changed() -> Error {
    Error::new(
        "OWNER_CHANGED",
        "Selected port owner or its operation has changed",
    )
}
impl PendingOwner {
    pub(super) fn validate(&self, r: &Record) -> Result<()> {
        if r.instance_id != self.instance_id
            || r.operation_id != self.original_operation_id
            || r.management_uncertain
            || !matches!(r.status.state(), State::Running | State::Failed)
        {
            return Err(owner_changed());
        }
        Ok(())
    }
}
pub(super) fn pending(r: &Record) -> bool {
    r.pending_owner.is_some()
        && !r.operation_done
        && !r.collector_attempted
        && r.status.state() == State::Starting
        && r.services.iter().all(|s| !s.spawn_attempted)
}
pub(super) fn phase(r: &Record) -> Option<&'static str> {
    r.pending_owner.as_ref().map(|p| {
        if r.operation_done {
            "complete"
        } else if p.stopped {
            "starting_target"
        } else {
            "stopping_owner"
        }
    })
}
pub(super) fn merge(r: &Record, owner: Option<(Uuid, Uuid)>) -> Result<Value> {
    if let Some((id, instance)) = owner
        && !r
            .pending_owner
            .as_ref()
            .is_some_and(|p| p.installation_id == id && p.instance_id == instance)
    {
        return Err(Error::new(
            "BUSY",
            "Target already has another accepted operation",
        ));
    }
    Ok(receipt(r))
}
pub(super) fn prepare(
    manager: &Manager,
    target: Uuid,
    owner: Uuid,
    instance: Uuid,
    plan: &LaunchPlan,
) -> Result<PendingOwner> {
    if target == owner {
        return Err(Error::new("INVALID_PARAMS", "Target and owner must differ"));
    }
    let handle = manager
        .handles
        .lock()
        .unwrap()
        .get(&owner)
        .cloned()
        .ok_or_else(owner_changed)?;
    let record = handle.record.lock().unwrap().clone();
    let mut intent = PendingOwner {
        installation_id: owner,
        instance_id: instance,
        original_operation_id: record.operation_id,
        // Publish this ID before requesting Stop. Recovery can distinguish our
        // accepted stop from a later user operation without stopping twice.
        stop_operation_id: Uuid::new_v4(),
        target_revision: plan.revision(),
        expires_at_ms: now()
            + record
                .services
                .iter()
                .map(|s| s.stop_timeout_ms as u128 + 2000)
                .sum::<u128>()
            + 10000,
        ports: Vec::new(),
        stopped: false,
    };
    intent.validate(&record)?;
    for service in plan.manifest().services.iter().filter(|s| s.port.is_some()) {
        let port = service.port.unwrap();
        for source in &record.services {
            if let Some(mut group) = source.group.clone() {
                let members = group.refresh()?;
                if !members.is_empty() && Group::port_listening(&members, port)? {
                    intent.ports.push(port);
                    break;
                }
            }
        }
    }
    if intent.ports.is_empty() {
        return Err(Error::new(
            "OWNER_NOT_PROVEN",
            "Selected instance does not own a conflicting listener",
        ));
    }
    Ok(intent)
}

pub(super) fn wait_for_owner(manager: &Arc<Manager>, h: &Handle) -> Result<Option<LaunchPlan>> {
    let (intent, id, epoch, actor) = {
        let r = h.record.lock().unwrap();
        (
            r.pending_owner.clone().unwrap(),
            r.installation_id,
            r.operation_epoch,
            r.audit.as_ref().map_or(Actor::System, |a| a.actor),
        )
    };
    let canceled = || {
        h.control.load(Ordering::Acquire) != 0
            || epoch != manager.stop_epoch.load(Ordering::Acquire)
    };
    if canceled() {
        return Ok(None);
    }
    // Clamp wall-clock recovery to the maximum supported stop budget, and use
    // monotonic time for the actual wait. No retry can extend the saved deadline.
    let remaining = intent.expires_at_ms.saturating_sub(now()).min(1_994_000) as u64;
    if remaining == 0 {
        return Err(Error::new(
            "OWNER_STOP_TIMEOUT",
            "Managed owner stop deadline expired",
        ));
    }
    let deadline = Instant::now() + Duration::from_millis(remaining);
    manager.request_stop(intent.installation_id, false, None, actor, Some(&intent))?;
    loop {
        if canceled() {
            return Ok(None);
        }
        let owner = manager
            .handles
            .lock()
            .unwrap()
            .get(&intent.installation_id)
            .cloned()
            .ok_or_else(owner_changed)?;
        let r = owner.record.lock().unwrap();
        if r.instance_id != intent.instance_id || r.operation_id != intent.stop_operation_id {
            return Err(owner_changed());
        }
        if r.operation_done {
            if r.status.state() != State::Stopped
                || r.operation_error.is_some()
                || r.management_uncertain
                || r.collector_alive
                || r.services.iter().any(|s| s.alive)
            {
                return Err(Error::new(
                    "OWNER_STOP_FAILED",
                    "Managed owner did not stop successfully",
                ));
            }
            break;
        }
        drop(r);
        if Instant::now() >= deadline {
            return Err(Error::new(
                "OWNER_STOP_TIMEOUT",
                "Managed owner stop deadline expired",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
    if canceled() {
        return Ok(None);
    }
    let plan = LaunchPlan::prepare(&manager.data, id)?;
    if plan.revision() != intent.target_revision {
        return Err(Error::new(
            "STALE_LAUNCH",
            "Target registration changed while stopping the owner",
        ));
    }
    // Admission/restart/StopAll use the same short manager gate. Read only one
    // installation at a time; never wait for a stop with this gate held.
    let handles = manager.handles.lock().unwrap();
    if canceled() {
        return Ok(None);
    }
    for service in &plan.manifest().services {
        if let Some(port) = service.port {
            for (other, handle) in handles.iter().filter(|(other, _)| **other != id) {
                let record = handle.record.lock().unwrap();
                if record.services.iter().any(|s| {
                    s.port == Some(port) && (s.alive || record.status.state() == State::Starting)
                }) {
                    return Err(Error::new("PORT_IN_USE", "Another managed installation now reserves the port")
                        .details(json!({"installation_id":other,"instance_id":record.instance_id,"port":port})));
                }
            }
            port_available(port)?;
        }
    }
    let mut r = h.record.lock().unwrap();
    if canceled() {
        return Ok(None);
    }
    r.pending_owner.as_mut().unwrap().stopped = true;
    r.launch_context = Some(plan.context());
    save_record(&manager.data, &mut r, &manager.events)?;
    Ok(Some(plan))
}
