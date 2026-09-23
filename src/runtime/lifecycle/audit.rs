use super::*;
use crate::journal::{self, Actor, Event, Outcome};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub action: String,
    pub actor: Actor,
    pub completion_recorded: bool,
}

pub(super) fn enqueue(r: &mut Record, event: Event) {
    if r.journal_pending
        .iter()
        .any(|e| e.event_id == event.event_id)
    {
        return;
    }
    if r.journal_pending.len() >= 32 {
        // A full queue loses facts; repairing storage cannot recover them.
        // Preserve that gap separately from the retryable write warning.
        r.warnings.retain(|w| w["code"] != "JOURNAL_INCOMPLETE");
        r.warnings.push(json!({"code":"JOURNAL_INCOMPLETE","level":"warning",
            "title":"Pending operation journal capacity was exceeded",
            "evidence":{"limit":32},"actions":[],"clears_when":"historical gap cannot be recovered"}));
        return;
    }
    r.journal_pending.push(event);
}
fn warn(r: &mut Record, warning: Value) {
    r.warnings.retain(|w| w["code"] != "JOURNAL_WRITE_FAILED");
    r.warnings.push(warning);
}
fn event(r: &Record, operation: &Operation, outcome: Outcome, phase: &str) -> Event {
    let mut e = Event::new(&operation.action, operation.actor, outcome);
    e.event_id = format!("{}:{}:{phase}", r.operation_id, operation.action);
    e.operation_id = Some(r.operation_id);
    e.installation_id = Some(r.installation_id);
    e.instance_id = Some(r.instance_id);
    e
}
pub(super) fn begin(r: &mut Record, action: &str, actor: Actor) {
    let operation = Operation {
        action: action.into(),
        actor,
        completion_recorded: false,
    };
    enqueue(r, event(r, &operation, Outcome::Accepted, "accepted"));
    r.audit = Some(operation);
}
pub(super) fn cancel(r: &mut Record) {
    if let Some(mut operation) = r.audit.clone()
        && !operation.completion_recorded
        && !r.operation_done
    {
        enqueue(r, event(r, &operation, Outcome::Canceled, "completed"));
        operation.completion_recorded = true;
        r.audit = Some(operation);
    }
}
pub(super) fn complete(r: &mut Record) {
    let Some(mut operation) = r.audit.clone() else {
        return;
    };
    if !r.operation_done || operation.completion_recorded {
        return;
    }
    let code = r
        .operation_error
        .as_ref()
        .and_then(|e| e["code"].as_str())
        .map(str::to_owned)
        .or_else(|| {
            (r.status.state() == State::Failed)
                .then(|| r.status.failure.clone())
                .flatten()
        });
    let outcome = if code.as_deref() == Some("START_CANCELLED")
        || (matches!(
            operation.action.as_str(),
            "start" | "restart" | "stop-owner-and-start"
        ) && r.status.state() == State::Stopped
            && code.is_none())
    {
        Outcome::Canceled
    } else if code.is_some() || r.status.state() == State::Failed {
        Outcome::Failed
    } else {
        Outcome::Succeeded
    };
    let mut e = event(r, &operation, outcome, "completed");
    e.code = code;
    enqueue(r, e);
    operation.completion_recorded = true;
    r.audit = Some(operation);
}
pub(super) fn flush(data: &Path, r: &mut Record) -> bool {
    if r.journal_pending.is_empty() {
        return false;
    }
    let mut changed = false;
    while let Some(e) = r.journal_pending.first() {
        match journal::append(data, e) {
            Ok(()) => {
                r.journal_pending.remove(0);
                changed = true;
            }
            Err(e) => {
                warn(r, journal::warning(&e));
                return true;
            }
        }
    }
    r.warnings.retain(|w| w["code"] != "JOURNAL_WRITE_FAILED");
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_capacity_is_bounded_and_repair_does_not_hide_lost_facts() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let root = temp.path().join("app");
        fs::create_dir(&root).unwrap();
        let manifest: crate::Manifest = serde_json::from_value(json!({
            "schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture",
            "services":[{"name":"service","command":["/bin/sleep","1"]}]
        }))
        .unwrap();
        fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id = crate::import(&data, &crate::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        let plan = LaunchPlan::prepare(&data, id).unwrap();
        let mut r = new_record(id, Uuid::new_v4(), &plan, 0);
        for _ in 0..33 {
            let mut e = Event::new("stop", Actor::Human, Outcome::Succeeded);
            e.installation_id = Some(id);
            enqueue(&mut r, e);
        }
        assert_eq!(r.journal_pending.len(), 32);
        assert_eq!(r.warnings.len(), 1);
        fs::write(data.join("journal"), b"blocked").unwrap();
        assert!(flush(&data, &mut r));
        assert_eq!(r.journal_pending.len(), 32);
        assert_eq!(r.warnings.len(), 2);
        fs::remove_file(data.join("journal")).unwrap();
        assert!(flush(&data, &mut r));
        assert!(r.journal_pending.is_empty());
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(r.warnings[0]["code"], "JOURNAL_INCOMPLETE");
        assert_eq!(
            journal::read(&data, None, 100).unwrap()["events"]
                .as_array()
                .unwrap()
                .len(),
            32
        );
    }
}
