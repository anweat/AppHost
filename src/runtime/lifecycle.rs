//! Bounded per-installation workers. A worker owns its Child handles until the
//! groups are empty; RPC threads only submit intent and read short snapshots.
use super::{InstanceStatus, LaunchPlan, State, collector, retention};
use crate::{
    Error, Result,
    process::{self, Group},
    storage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

mod worker;
use worker::{Live, run};

const MAX_ACTIVE: usize = 32;
const MAX_RECORDS: usize = 1024;
fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn invalid(message: &str) -> Error {
    Error::new("RUNTIME_INVALID", message)
}
fn encode<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    let b = serde_json::to_vec(v).map_err(|e| invalid(&e.to_string()))?;
    if b.len() > storage::MAX_BYTES as usize {
        return Err(Error::new("RUNTIME_LIMIT", "Runtime record exceeds 1 MiB"));
    }
    Ok(b)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRecord {
    pub name: String,
    pub key: String,
    pub port: Option<u16>,
    pub startup_timeout_ms: u64,
    pub stop_timeout_ms: u64,
    pub spawn_attempted: bool,
    pub group: Option<Group>,
    pub alive: bool,
    pub ready: bool,
    pub exit: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema_version: u32,
    pub installation_id: Uuid,
    pub instance_id: Uuid,
    pub operation_id: Uuid,
    #[serde(default)]
    pub operation_epoch: u64,
    pub operation_done: bool,
    pub operation_error: Option<Value>,
    pub operation_result: Option<Value>,
    pub pending_mutation: Option<Mutation>,
    pub restart_after_stop: bool,
    pub status: InstanceStatus,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
    pub services: Vec<ServiceRecord>,
    pub collector_attempted: bool,
    pub collector: Option<Group>,
    pub collector_alive: bool,
    pub management_uncertain: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "path", rename_all = "kebab-case")]
pub enum Mutation {
    Detach,
    Relink(PathBuf),
}
impl Mutation {
    fn apply(&self, data: &Path, id: Uuid) -> Result<Value> {
        match self {
            Self::Detach => Ok(json!(crate::registry::detach(data, id)?)),
            Self::Relink(path) => Ok(json!(crate::registry::relink(data, id, path)?)),
        }
    }
}
struct Handle {
    record: Mutex<Record>,
    // 0: observe, 1: explicit stop, 2: full restart. Durable intent lives in Record.
    control: AtomicU8,
    working: AtomicBool,
    // Preserve Child ownership across a worker IO failure, so a later explicit
    // stop can reap each leader after proving its whole group empty.
    parked: Mutex<Option<Live>>,
}
fn record_path(data: &Path, id: Uuid) -> PathBuf {
    data.join("runtime").join(format!("{id}.json"))
}
fn log_path(data: &Path, r: &Record) -> PathBuf {
    data.join("instances")
        .join(r.instance_id.to_string())
        .join("logs")
}
fn save_record(data: &Path, r: &Record) -> Result<()> {
    storage::publish(&record_path(data, r.installation_id), &encode(r)?, true)
}
fn new_record(id: Uuid, op: Uuid, plan: &LaunchPlan, epoch: u64) -> Record {
    Record {
        schema_version: 1,
        installation_id: id,
        instance_id: Uuid::new_v4(),
        operation_id: op,
        operation_epoch: epoch,
        operation_done: false,
        operation_error: None,
        operation_result: None,
        pending_mutation: None,
        restart_after_stop: false,
        status: InstanceStatus::default(),
        created_at_ms: now(),
        updated_at_ms: now(),
        services: plan
            .manifest()
            .services
            .iter()
            .enumerate()
            .map(|(i, s)| ServiceRecord {
                name: s.name.clone(),
                key: format!("s{i}"),
                port: s.port,
                startup_timeout_ms: s.startup_timeout_ms(),
                stop_timeout_ms: s.stop_timeout_ms(),
                spawn_attempted: false,
                group: None,
                alive: false,
                ready: false,
                exit: None,
            })
            .collect(),
        collector_attempted: false,
        collector: None,
        collector_alive: false,
        management_uncertain: false,
    }
}
fn load_records(data: &Path) -> Result<Vec<Record>> {
    let root = data.join("runtime");
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(storage::io_error(&root, e)),
    };
    let mut records = Vec::new();
    let mut ignored = 0;
    for (count, e) in entries.enumerate() {
        if count >= 4096 {
            return Err(invalid("Too many runtime directory entries"));
        }
        let e = e.map_err(|e| storage::io_error(&root, e))?;
        // Only canonical installation filenames carry runtime authority. Never
        // interpret Finder metadata or unrelated user files as snapshots.
        let name = e.file_name();
        let valid = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_some_and(|id| name == format!("{id}.json").as_str());
        if !valid {
            ignored += 1;
            continue;
        }
        let r: Record = serde_json::from_slice(&storage::read(&e.path())?).map_err(|error| {
            invalid(&format!(
                "Invalid runtime record {}: {error}",
                e.path().display()
            ))
        })?;
        if r.schema_version != 1
            || e.file_name() != format!("{}.json", r.installation_id).as_str()
            || r.services.is_empty()
            || r.services.len() > 32
            || r.services.iter().enumerate().any(|(i, s)| {
                s.key != format!("s{i}")
                    || !(1000..=600000).contains(&s.startup_timeout_ms)
                    || !(1000..=60000).contains(&s.stop_timeout_ms)
                    || s.port == Some(0)
            })
        {
            return Err(invalid(
                "Invalid runtime identity, version, service count or timeouts",
            ));
        }
        records.push(r);
    }
    if ignored > 0 {
        eprintln!(
            "AppHost: ignored {ignored} unrelated entries in {}",
            root.display()
        );
    }
    if records.len() > MAX_RECORDS {
        return Err(invalid("Too many runtime records"));
    }
    Ok(records)
}
pub fn history(data: &Path, id: Option<Uuid>) -> Result<Value> {
    let records: Vec<_> = load_records(data)?
        .into_iter()
        .filter(|r| id.is_none_or(|id| r.installation_id == id))
        .collect();
    Ok(json!({"live":false,"instances":records}))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wake {
    schema_version: u32,
    paused: bool,
    stop_epoch: u64,
}
fn wake(data: &Path) -> Result<Option<Wake>> {
    let bytes = match storage::read(&data.join("wake.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.code == "NOT_FOUND" => return Ok(None),
        Err(error) => return Err(error),
    };
    // Preserve the early boolean-only preference without resetting later epochs.
    if let Ok(paused) = serde_json::from_slice::<bool>(&bytes) {
        return Ok(Some(Wake {
            schema_version: 1,
            paused,
            stop_epoch: 0,
        }));
    }
    let saved: Wake =
        serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid automatic wake preference"))?;
    if saved.schema_version != 1 {
        return Err(invalid("Unsupported automatic wake preference version"));
    }
    Ok(Some(saved))
}

pub(crate) struct Manager {
    data: PathBuf,
    handles: Mutex<BTreeMap<Uuid, Arc<Handle>>>,
    launch_dispatch: Mutex<()>,
    paused: AtomicBool,
    stop_epoch: AtomicU64,
}
impl Manager {
    pub fn open(data: &Path) -> Result<Arc<Self>> {
        storage::private_directory(&data.join("runtime"))?;
        // The daemon singleton lock excludes any other runtime publisher.
        // Remove only our identifiable, unpublished regular temporary files.
        for (count, entry) in fs::read_dir(data.join("runtime"))
            .map_err(|e| storage::io_error(data, e))?
            .enumerate()
        {
            if count >= 4096 {
                return Err(invalid("Too many runtime directory entries"));
            }
            let entry = entry.map_err(|e| storage::io_error(data, e))?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".apphost-publish-")
            {
                if !entry
                    .file_type()
                    .map_err(|e| storage::io_error(&entry.path(), e))?
                    .is_file()
                {
                    return Err(invalid("Invalid unpublished runtime file"));
                }
                fs::remove_file(entry.path()).map_err(|e| storage::io_error(&entry.path(), e))?;
            }
        }
        let mut records = load_records(data)?;
        let saved_wake = wake(data)?;
        let missing_wake = saved_wake.is_none();
        let wake = match saved_wake {
            Some(wake) => wake,
            None => Wake {
                schema_version: 1,
                paused: !records.is_empty(),
                stop_epoch: match records.iter().map(|r| r.operation_epoch).max() {
                    Some(epoch) => epoch
                        .checked_add(1)
                        .ok_or_else(|| invalid("Cancellation epoch exhausted"))?,
                    None => 0,
                },
            },
        };
        if missing_wake {
            // Missing global intent is not a fresh StopAll. Normalize every record
            // FIRST, retaining normal live services and cancelling only restart
            // continuations. Publish wake last: a crash midway leaves it absent,
            // so another recovery safely repeats from the largest saved epoch.
            for r in &mut records {
                r.operation_epoch = wake.stop_epoch;
                if r.restart_after_stop {
                    r.restart_after_stop = false;
                    r.operation_error = Some(json!(Error::new(
                        "START_CANCELLED",
                        "Missing wake preference cancelled pending restart"
                    )));
                    if r.status.state() == State::Stopped && r.pending_mutation.is_none() {
                        r.operation_done = true;
                    }
                }
                save_record(data, r)?;
            }
            storage::publish(&data.join("wake.json"), &encode(&wake)?, true)?;
        }
        let manager = Arc::new(Self {
            data: data.to_owned(),
            handles: Mutex::new(BTreeMap::new()),
            launch_dispatch: Mutex::new(()),
            paused: AtomicBool::new(wake.paused),
            stop_epoch: AtomicU64::new(wake.stop_epoch),
        });
        let mut recovered = Vec::new();
        for mut r in records {
            let mut live = Live::from_record(&r);
            // Reconcile all identities before advertising the management endpoint.
            live.reconcile(&manager.data, &mut r);
            if r.operation_epoch > wake.stop_epoch {
                return Err(invalid(
                    "Runtime operation is newer than the saved cancellation epoch",
                ));
            }
            if r.operation_epoch < wake.stop_epoch {
                // StopAll may have crashed after publishing its global intent
                // but before updating any per-installation record.
                r.operation_epoch = wake.stop_epoch;
                r.restart_after_stop = false;
                if live.any() || r.management_uncertain || r.status.state() != State::Stopped {
                    r.operation_id = Uuid::new_v4();
                    r.operation_done = false;
                    r.status.request_stop();
                }
            }
            if r.status.state() != State::Stopped
                && r.status.state() != State::Stopping
                && !live.any()
                && !r.management_uncertain
            {
                // A dead failed instance is still evidence requiring attention.
                if r.status.state() != State::Failed {
                    r.status.state = State::Stopped;
                }
                r.operation_done = true;
            }
            save_record(data, &r)?;
            let id = r.installation_id;
            let stopping = r.status.state() == State::Stopping
                || (!r.operation_done && (r.pending_mutation.is_some() || r.restart_after_stop));
            let control = if stopping {
                if r.restart_after_stop { 2 } else { 1 }
            } else {
                0
            };
            let handle = Arc::new(Handle {
                record: Mutex::new(r),
                control: AtomicU8::new(control),
                working: AtomicBool::new(false),
                parked: Mutex::new(None),
            });
            manager.handles.lock().unwrap().insert(id, handle.clone());
            if live.any() || stopping {
                recovered.push((handle, live));
            }
        }
        if recovered.len() > MAX_ACTIVE {
            return Err(Error::new(
                "RUNTIME_LIMIT",
                "Too many unfinished instances to recover",
            ));
        }
        for (handle, live) in recovered {
            manager.worker(handle, None, live)?;
        }
        Ok(manager)
    }
    fn worker(
        self: &Arc<Self>,
        h: Arc<Handle>,
        plan: Option<LaunchPlan>,
        mut live: Live,
    ) -> Result<()> {
        h.working.store(true, Ordering::Release);
        let manager = self.clone();
        let failed = h.clone();
        thread::Builder::new()
            .name("apphost-instance".into())
            .spawn(move || {
                let mut plan = plan;
                loop {
                    if let Err(error) = run(&manager, &h, plan.take(), &mut live) {
                        let mut r = h.record.lock().unwrap();
                        r.status.fail(&error.code);
                        if r.status.state() == State::Stopping {
                            r.status.state = State::Failed;
                        }
                        r.operation_error = Some(json!(error));
                        r.operation_done = true;
                        if matches!(error.code.as_str(), "IO_ERROR" | "MANAGEMENT_UNCERTAIN") {
                            r.management_uncertain = true;
                        }
                        let _ = save_record(&manager.data, &r);
                    }
                    let r = h.record.lock().unwrap();
                    // Close the worker/Stop admission race under the same mutex.
                    if h.control.load(Ordering::Acquire) != 0 {
                        drop(r);
                        continue;
                    }
                    *h.parked.lock().unwrap() = Some(live);
                    h.working.store(false, Ordering::Release);
                    break;
                }
            })
            .map_err(|e| {
                failed.working.store(false, Ordering::Release);
                Error::new("RUNTIME_LIMIT", e.to_string())
            })?;
        Ok(())
    }
    pub fn status(&self, id: Option<Uuid>) -> Result<Value> {
        let handles = self.handles.lock().unwrap();
        let records: Vec<_> = handles
            .iter()
            .filter(|(key, _)| id.is_none_or(|id| id == **key))
            .map(|(_, h)| h.record.lock().unwrap().clone())
            .collect();
        Ok(
            json!({"live":true,"automatic_wake_paused":self.paused.load(Ordering::Acquire),"instances":records}),
        )
    }
    pub fn start(self: &Arc<Self>, id: Uuid) -> Result<Value> {
        let epoch = self.stop_epoch.load(Ordering::Acquire);
        if let Some(h) = self.handles.lock().unwrap().get(&id) {
            let r = h.record.lock().unwrap();
            if matches!(r.status.state(), State::Starting | State::Running)
                && h.working.load(Ordering::Acquire)
            {
                return Ok(receipt(&r));
            }
        }
        // Preparation may touch a slow source directory. It never holds the
        // manager mutex or delays another installation's Stop request.
        let plan = LaunchPlan::prepare(&self.data, id)?;
        if plan.manifest().services.len() > 32 {
            return Err(Error::new(
                "RUNTIME_LIMIT",
                "At most 32 services per instance",
            ));
        }
        let mut handles = self.handles.lock().unwrap();
        if epoch != self.stop_epoch.load(Ordering::Acquire) {
            return Err(Error::new(
                "START_CANCELLED",
                "StopAll cancelled this pending start",
            ));
        }
        if let Some(h) = handles.get(&id) {
            let r = h.record.lock().unwrap();
            if matches!(r.status.state(), State::Starting | State::Running)
                && h.working.load(Ordering::Acquire)
            {
                return Ok(receipt(&r));
            }
            if r.management_uncertain || r.services.iter().any(|s| s.alive) || r.collector_alive {
                return Err(Error::new(
                    "RESTART_REQUIRED",
                    "Stop or restart the existing instance before starting again",
                ));
            }
            if h.working.load(Ordering::Acquire) {
                return Err(Error::new(
                    "BUSY",
                    "The previous operation is still completing",
                ));
            }
        }
        if handles
            .values()
            .filter(|h| h.working.load(Ordering::Acquire))
            .count()
            >= MAX_ACTIVE
            || (!handles.contains_key(&id) && handles.len() >= MAX_RECORDS)
        {
            return Err(Error::new("RUNTIME_LIMIT", "Runtime capacity reached"));
        }
        for service in &plan.manifest().services {
            if let Some(port) = service.port {
                for h in handles.values() {
                    let r = h.record.lock().unwrap();
                    if r.services.iter().any(|s| {
                        s.port == Some(port) && (s.alive || r.status.state() == State::Starting)
                    }) {
                        return Err(Error::new(
                            "PORT_IN_USE",
                            "Port reserved by an AppHost instance",
                        )
                        .details(json!({"port":port,"installation_id":r.installation_id})));
                    }
                }
                port_available(port)?;
            }
        }
        if let Some(old) = handles.get(&id) {
            retention::archive(&self.data, &old.record.lock().unwrap())?;
        }
        let record = new_record(id, Uuid::new_v4(), &plan, epoch);
        save_record(&self.data, &record)?;
        let result = receipt(&record);
        let h = Arc::new(Handle {
            record: Mutex::new(record),
            control: AtomicU8::new(0),
            working: AtomicBool::new(false),
            parked: Mutex::new(None),
        });
        handles.insert(id, h.clone());
        self.worker(h, Some(plan), Live::empty())?;
        Ok(result)
    }
    pub fn stop(self: &Arc<Self>, id: Uuid, restart: bool) -> Result<Value> {
        self.request_stop(id, restart, None)
    }
    pub fn mutate(self: &Arc<Self>, id: Uuid, mutation: Mutation) -> Result<Value> {
        let handles = self.handles.lock().unwrap();
        if !handles.contains_key(&id) {
            return mutation.apply(&self.data, id);
        }
        drop(handles);
        self.request_stop(id, false, Some(mutation))
    }
    fn request_stop(
        self: &Arc<Self>,
        id: Uuid,
        restart: bool,
        mutation: Option<Mutation>,
    ) -> Result<Value> {
        let h = self.handles.lock().unwrap().get(&id).cloned();
        let Some(h) = h else {
            return if restart {
                self.start(id)
            } else {
                Ok(json!({"installation_id":id,"state":"STOPPED","operation_done":true}))
            };
        };
        let mut r = h.record.lock().unwrap();
        if r.status.state() == State::Stopping && h.working.load(Ordering::Acquire) {
            if mutation.is_some() {
                return Err(Error::new(
                    "BUSY",
                    "Stop is already in progress; retry the registration change after it completes",
                ));
            }
            return if (h.control.load(Ordering::Acquire) == 2) == restart {
                Ok(receipt(&r))
            } else {
                Err(Error::new(
                    "BUSY",
                    "Another stop or restart is already accepted",
                ))
            };
        }
        if !restart && mutation.is_none() && r.status.state() == State::Stopped && r.operation_done
        {
            return Ok(receipt(&r));
        }
        r.operation_id = Uuid::new_v4();
        r.operation_epoch = self.stop_epoch.load(Ordering::Acquire);
        r.operation_done = false;
        r.operation_error = None;
        r.operation_result = None;
        r.status.request_stop();
        r.pending_mutation = mutation;
        r.restart_after_stop = restart;
        save_record(&self.data, &r)?;
        h.control
            .store(if restart { 2 } else { 1 }, Ordering::Release);
        let result = receipt(&r);
        if !h.working.load(Ordering::Acquire) {
            let mut live = h
                .parked
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Live::from_record(&r));
            live.reconcile(&self.data, &mut r);
            self.worker(h.clone(), None, live)?;
        }
        drop(r);
        Ok(result)
    }
    pub fn stop_all(self: &Arc<Self>) -> Result<Value> {
        // Persist before cancellation. Later explicit Start remains allowed;
        // only automatic wake is paused by this preference.
        let ids: Vec<_> = {
            let handles = self.handles.lock().unwrap();
            let epoch = self
                .stop_epoch
                .load(Ordering::Acquire)
                .checked_add(1)
                .ok_or_else(|| invalid("Cancellation epoch exhausted"))?;
            storage::publish(
                &self.data.join("wake.json"),
                &encode(&Wake {
                    schema_version: 1,
                    paused: true,
                    stop_epoch: epoch,
                })?,
                true,
            )?;
            self.paused.store(true, Ordering::Release);
            self.stop_epoch.store(epoch, Ordering::Release);
            for h in handles.values() {
                let mut r = h.record.lock().unwrap();
                // StopAll cancels the launch half of an accepted Restart.
                if h.control.load(Ordering::Acquire) == 2 {
                    r.restart_after_stop = false;
                    save_record(&self.data, &r)?;
                    h.control.store(1, Ordering::Release);
                }
            }
            handles.keys().copied().collect()
        };
        let results: Vec<_> = ids
            .into_iter()
            .map(|id| match self.stop(id, false) {
                Ok(v) => v,
                Err(e) => json!({"installation_id":id,"error":e}),
            })
            .collect();
        Ok(json!({"automatic_wake_paused":true,"results":results}))
    }
    pub fn logs(
        &self,
        id: Uuid,
        instance: Uuid,
        from: u128,
        to: u128,
        limit: usize,
    ) -> Result<Value> {
        let r = self
            .handles
            .lock()
            .unwrap()
            .get(&id)
            .map(|h| h.record.lock().unwrap().clone())
            .ok_or_else(|| Error::new("NOT_FOUND", "No runtime instance"))?;
        let r = if r.instance_id == instance {
            r
        } else {
            retention::read(&self.data, id, instance)?
        };
        // JSON byte arrays can expand sixfold. Keep the wire response under 1 MiB.
        if !(1..=128 * 1024).contains(&limit) {
            return Err(Error::new(
                "INVALID_PARAMS",
                "Log byte limit must be 1..131072",
            ));
        }
        let window = crate::logs::read_window(&log_path(&self.data, &r), from, to, limit)
            .map_err(|e| storage::io_error(&log_path(&self.data, &r), e))?;
        Ok(
            json!({"instance_id":instance,"log_incomplete":r.status.log_incomplete,"services":r.services.iter().map(|s|json!({"name":s.name,"key":s.key})).collect::<Vec<_>>(),"window":window}),
        )
    }
}
fn receipt(r: &Record) -> Value {
    json!({"installation_id":r.installation_id,"instance_id":r.instance_id,"operation_id":r.operation_id,"operation_done":r.operation_done,"state":r.status.state()})
}
fn port_available(port: u16) -> Result<()> {
    // macOS permits a reusable wildcard bind beside a specific-address listener
    // (and vice versa). Check both addresses in both families. Keep REUSEADDR:
    // a socket without it also rejects a closed server's harmless TIME_WAIT.
    for host in ["0.0.0.0", "127.0.0.1", "[::]", "[::1]"] {
        TcpListener::bind(format!("{host}:{port}")).map_err(|e| {
            Error::new(
                if e.kind() == std::io::ErrorKind::AddrInUse { "PORT_IN_USE" } else { "PORT_CHECK_FAILED" },
                "Declared port is unavailable; no process was stopped",
            ).details(json!({"port":port,"address":host,"reason":e.to_string(),"ownership":process::port_owners(port)}))
        })?;
    }
    Ok(())
}

impl Manager {
    pub fn dispatch(self: &Arc<Self>, request: &crate::daemon::protocol::Request) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Installation {
            installation_id: Uuid,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Relink {
            installation_id: Uuid,
            path: PathBuf,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Status {
            installation_id: Option<Uuid>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Logs {
            installation_id: Uuid,
            instance_id: Uuid,
            from_ms: u128,
            to_ms: u128,
            limit: usize,
        }
        fn params<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T> {
            serde_json::from_value(v.clone())
                .map_err(|_| Error::new("INVALID_PARAMS", "Invalid runtime method parameters"))
        }
        match request.method.as_str() {
            "start" => self.start(params::<Installation>(&request.params)?.installation_id),
            "stop" => self.stop(
                params::<Installation>(&request.params)?.installation_id,
                false,
            ),
            "restart" => self.stop(
                params::<Installation>(&request.params)?.installation_id,
                true,
            ),
            "detach" => self.mutate(
                params::<Installation>(&request.params)?.installation_id,
                Mutation::Detach,
            ),
            "relink" => {
                let p = params::<Relink>(&request.params)?;
                if !p.path.is_absolute() {
                    return Err(Error::new(
                        "INVALID_PARAMS",
                        "Relink requires an absolute project path",
                    ));
                }
                self.mutate(p.installation_id, Mutation::Relink(p.path))
            }
            "status" => self.status(params::<Status>(&request.params)?.installation_id),
            "stop-all" if request.params.as_object().is_some_and(|p| p.is_empty()) => {
                self.stop_all()
            }
            "stop-all" => Err(Error::new(
                "INVALID_PARAMS",
                "stop-all accepts no parameters",
            )),
            "logs" => {
                let p = params::<Logs>(&request.params)?;
                self.logs(
                    p.installation_id,
                    p.instance_id,
                    p.from_ms,
                    p.to_ms,
                    p.limit,
                )
            }
            _ => Err(Error::new(
                "METHOD_NOT_FOUND",
                "Daemon method is not available",
            )),
        }
    }
}
