//! Bounded JSONL operation facts shared by offline CLI writes and the daemon.
use crate::{Error, Result, storage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const SEGMENT_BYTES: usize = 1024 * 1024;
const MAX_SEGMENTS: usize = 8;
const AGE_MS: u128 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Actor {
    #[default]
    Human,
    Agent,
    System,
}
impl Actor {
    pub fn from_environment() -> Result<Self> {
        match std::env::var("APPHOST_ACTOR") {
            Ok(s) => match s.as_str() {
                "human" => Ok(Self::Human),
                "agent" => Ok(Self::Agent),
                "system" => Ok(Self::System),
                _ => Err(Error::new(
                    "INVALID_ACTOR",
                    "APPHOST_ACTOR must be human, agent or system",
                )),
            },
            Err(std::env::VarError::NotPresent) => Ok(Self::Human),
            Err(_) => Err(Error::new(
                "INVALID_ACTOR",
                "APPHOST_ACTOR must be valid UTF-8",
            )),
        }
    }
    pub fn is_human(&self) -> bool {
        *self == Self::Human
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Accepted,
    Succeeded,
    Failed,
    Canceled,
    Observed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub schema_version: u32,
    pub event_id: String,
    pub at_ms: u128,
    pub action: String,
    pub actor: Actor,
    pub installation_id: Option<Uuid>,
    pub instance_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub outcome: Outcome,
    pub code: Option<String>,
}
pub(crate) fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
impl Event {
    pub fn new(action: &str, actor: Actor, outcome: Outcome) -> Self {
        Self {
            schema_version: 1,
            event_id: Uuid::new_v4().to_string(),
            at_ms: now(),
            action: action.into(),
            actor,
            installation_id: None,
            instance_id: None,
            operation_id: None,
            outcome,
            code: None,
        }
    }
    pub(crate) fn valid(&self) -> bool {
        fn bounded(s: &str, n: usize) -> bool {
            !s.is_empty()
                && s.len() <= n
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
        }
        self.schema_version == 1
            // Public JSON results must also fit serde_json::Value's integers.
            && self.at_ms <= u64::MAX as u128
            && bounded(&self.event_id, 128)
            && bounded(&self.action, 64)
            && self.code.as_ref().is_none_or(|s| bounded(s, 64))
    }
}
fn invalid(message: &str) -> Error {
    Error::new("JOURNAL_INVALID", message)
}
fn directory(data: &Path, create: bool) -> Result<Option<PathBuf>> {
    let dir = data.join("journal");
    if create {
        storage::private_directory(data)?;
        storage::private_directory(&dir)?;
    }
    match fs::symlink_metadata(&dir) {
        Ok(m) if m.is_dir() => {
            storage::check_private_directory(&dir)?;
            Ok(Some(dir))
        }
        Ok(_) => Err(invalid("Journal directory must not be a link or file")),
        Err(e) if !create && e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(storage::io_error(&dir, e)),
    }
}
fn lock(dir: &Path, write: bool) -> Result<File> {
    let path = dir.join(".lock");
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(write)
        .create(write)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(&path)
        .map_err(|e| storage::io_error(&path, e))?;
    if !file
        .metadata()
        .map_err(|e| storage::io_error(&path, e))?
        .is_file()
    {
        return Err(invalid("Journal lock must be a regular file"));
    }
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        let result = if write {
            fs2::FileExt::try_lock_exclusive(&file)
        } else {
            fs2::FileExt::try_lock_shared(&file)
        };
        match result {
            Ok(()) => return Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(Error::new("BUSY", "Journal writer is busy"));
            }
            Err(e) => return Err(storage::io_error(&path, e)),
        }
    }
}
struct Segment {
    number: u64,
    path: PathBuf,
    bytes: usize,
    events: Vec<Event>,
}
fn segments(dir: &Path) -> Result<Vec<Segment>> {
    let mut out = vec![];
    for (count, entry) in fs::read_dir(dir)
        .map_err(|e| storage::io_error(dir, e))?
        .enumerate()
    {
        if count >= 256 {
            return Err(Error::new(
                "JOURNAL_LIMIT",
                "Too many journal directory entries",
            ));
        }
        let entry = entry.map_err(|e| storage::io_error(dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(number) = name
            .strip_suffix(".jsonl")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| name == format!("{n}.jsonl"))
        else {
            continue;
        };
        if out.len() > MAX_SEGMENTS {
            return Err(Error::new("JOURNAL_LIMIT", "Too many journal segments"));
        }
        let bytes = storage::read(&entry.path())?;
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            return Err(invalid(
                "Journal has an unfinished tail; preserve the file before repairing it",
            ));
        }
        let mut events = vec![];
        for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
            if line.len() > 4096 {
                return Err(invalid("Journal line exceeds 4096 bytes"));
            }
            let event: Event =
                serde_json::from_slice(line).map_err(|_| invalid("Invalid journal event"))?;
            if !event.valid() {
                return Err(invalid("Invalid journal event version or fields"));
            }
            events.push(event);
        }
        out.push(Segment {
            number,
            path: entry.path(),
            bytes: bytes.len(),
            events,
        });
    }
    out.sort_by_key(|s| s.number);
    Ok(out)
}

pub fn append(data: &Path, event: &Event) -> Result<()> {
    append_at(data, event, now())
}
fn append_at(data: &Path, event: &Event, at: u128) -> Result<()> {
    if !event.valid() {
        return Err(invalid("Invalid journal event"));
    }
    let mut bytes = serde_json::to_vec(event).map_err(|e| invalid(&e.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > 4096 {
        return Err(Error::new(
            "JOURNAL_LIMIT",
            "Journal event exceeds 4096 bytes",
        ));
    }
    let dir = directory(data, true)?.unwrap();
    let _lock = lock(&dir, true)?;
    let mut files = segments(&dir)?;
    if let Some(old) = files
        .iter()
        .flat_map(|s| &s.events)
        .find(|old| old.event_id == event.event_id)
    {
        let mut repeated = event.clone();
        repeated.at_ms = old.at_ms;
        if repeated != *old {
            return Err(Error::new(
                "JOURNAL_CONFLICT",
                "Event identity already belongs to another operation fact",
            ));
        }
        return Ok(());
    }
    let next = files.last().map_or(Ok(0), |s| {
        s.number
            .checked_add(1)
            .ok_or_else(|| invalid("Journal segment counter exhausted"))
    })?;
    let mut i = 0;
    while i < files.len() {
        if files[i]
            .events
            .iter()
            .all(|e| at.saturating_sub(e.at_ms) > AGE_MS)
        {
            let old = files.remove(i);
            fs::remove_file(&old.path).map_err(|e| storage::io_error(&old.path, e))?;
        } else {
            i += 1;
        }
    }
    while files.len() > MAX_SEGMENTS {
        let old = files.remove(0);
        fs::remove_file(&old.path).map_err(|e| storage::io_error(&old.path, e))?;
    }
    if let Some(last) = files
        .last()
        .filter(|s| s.bytes + bytes.len() <= SEGMENT_BYTES)
    {
        let mut options = OpenOptions::new();
        options.append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut file = options
            .open(&last.path)
            .map_err(|e| storage::io_error(&last.path, e))?;
        if !file
            .metadata()
            .map_err(|e| storage::io_error(&last.path, e))?
            .is_file()
        {
            return Err(invalid("Journal segment must be a regular file"));
        }
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| storage::io_error(&last.path, e))?;
    } else {
        // A ninth segment can exist after a crash between publish and pruning.
        storage::publish(&dir.join(format!("{next}.jsonl")), &bytes, false)?;
        files.push(Segment {
            number: next,
            path: dir.join(format!("{next}.jsonl")),
            bytes: bytes.len(),
            events: vec![],
        });
    }
    while files.len() > MAX_SEGMENTS {
        let old = files.remove(0);
        fs::remove_file(&old.path).map_err(|e| storage::io_error(&old.path, e))?;
    }
    Ok(())
}

/// A cursor is the last event ID seen, not an offset into a mutable file.
pub fn read(data: &Path, cursor: Option<&str>, limit: usize) -> Result<Value> {
    read_at(data, cursor, limit, now())
}
fn read_at(data: &Path, cursor: Option<&str>, limit: usize, at: u128) -> Result<Value> {
    if !(1..=200).contains(&limit) || cursor.is_some_and(|c| c.is_empty() || c.len() > 128) {
        return Err(Error::new(
            "INVALID_PARAMS",
            "Journal limit must be 1..200 and cursor at most 128 bytes",
        ));
    }
    let Some(dir) = directory(data, false)? else {
        if cursor.is_some() {
            return Err(Error::new(
                "JOURNAL_CURSOR_EXPIRED",
                "Reload journal without the old cursor",
            ));
        }
        return Ok(json!({"events":[],"next_cursor":null,"has_more":false}));
    };
    let _lock = lock(&dir, false)?;
    let files = segments(&dir)?;
    let events: Vec<_> = files
        .iter()
        .flat_map(|s| &s.events)
        .filter(|e| at.saturating_sub(e.at_ms) <= AGE_MS)
        .collect();
    let start = match cursor {
        Some(id) => events
            .iter()
            .position(|e| e.event_id == id)
            .map(|p| p + 1)
            .ok_or_else(|| {
                Error::new(
                    "JOURNAL_CURSOR_EXPIRED",
                    "Journal cursor was removed or expired; reload the snapshot",
                )
            })?,
        None => 0,
    };
    let page: Vec<_> = events.iter().skip(start).take(limit).copied().collect();
    let next = page.last().map(|e| e.event_id.as_str()).or(cursor);
    Ok(json!({"events":page,"next_cursor":next,"has_more":start+page.len()<events.len()}))
}

pub fn warning(error: &Error) -> Value {
    json!({"code":"JOURNAL_WRITE_FAILED","level":"warning","title":"Operation result was not fully recorded",
    "evidence":{"error_code":error.code},"actions":[],"clears_when":"journal storage repaired"})
}

/// Preserve legacy object/array receipts. Logging trouble is not a failed command.
pub fn add_warning(value: &mut Value, warn: Value) {
    match value {
        Value::Object(object) => {
            let entry = object.entry("warnings").or_insert_with(|| json!([]));
            if let Some(warnings) = entry.as_array_mut()
                && !warnings.iter().any(|w| w["code"] == warn["code"])
            {
                warnings.push(warn);
            }
        }
        Value::Array(items) => {
            for item in items {
                add_warning(item, warn.clone());
            }
        }
        _ => {}
    }
}

/// Application-layer audit of L0 receipts, including the completed part of a batch.
/// Core registration stays independent of the daemon and of journal availability.
pub fn record_result(
    data: &Path,
    action: &str,
    actor: Actor,
    installation_id: Option<Uuid>,
    mut result: Result<Value>,
) -> Result<Value> {
    fn receipts(value: &Value) -> Vec<&Value> {
        match value {
            Value::Array(a) => a.iter().collect(),
            v => vec![v],
        }
    }
    let mut events = vec![];
    let completed = match &result {
        Ok(v) => receipts(v),
        Err(e) => e.details.get("completed").map(receipts).unwrap_or_default(),
    };
    for receipt in completed {
        if receipt["operation_id"].is_string() || receipt["journal_event_id"].is_string() {
            continue;
        }
        let mut event = Event::new(action, actor, Outcome::Succeeded);
        event.installation_id = receipt["installation_id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .or(installation_id);
        events.push(event);
    }
    if let Err(error) = &result
        && !error.details["journal_event_id"].is_string()
    {
        let mut event = Event::new(action, actor, Outcome::Failed);
        event.installation_id = installation_id;
        event.code = Some(error.code.clone());
        events.push(event);
    }
    for event in events {
        if let Err(error) = append(data, &event) {
            match &mut result {
                Ok(v) => add_warning(v, warning(&error)),
                Err(e) => {
                    if !e.details.is_object() {
                        e.details = json!({"evidence":e.details});
                    }
                    add_warning(&mut e.details, warning(&error));
                }
            }
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_rotation_and_interrupted_pruning_stay_bounded() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let mut first = Event::new("start", Actor::Human, Outcome::Accepted);
        first.at_ms = 100;
        append_at(&data, &first, 100).unwrap();
        let dir = data.join("journal");
        // Reproduce a crash after publishing the ninth segment, before pruning.
        // Newline padding fills the capacity without an expensive fixture writer.
        for n in 0..=MAX_SEGMENTS {
            let mut e = first.clone();
            e.event_id = format!("segment-{n}");
            let mut bytes = serde_json::to_vec(&e).unwrap();
            bytes.resize(SEGMENT_BYTES, b'\n');
            fs::write(dir.join(format!("{n}.jsonl")), bytes).unwrap();
        }
        let mut latest = first.clone();
        latest.event_id = "new-event".into();
        append_at(&data, &latest, 100).unwrap();
        let files = segments(&dir).unwrap();
        assert_eq!(files.len(), MAX_SEGMENTS);
        assert!(files.iter().all(|f| f.bytes <= SEGMENT_BYTES));
        assert_eq!(files.first().unwrap().number, 2);
        assert_eq!(files.last().unwrap().number, 9);
        assert_eq!(
            read_at(&data, Some("segment-0"), 1, 100).unwrap_err().code,
            "JOURNAL_CURSOR_EXPIRED"
        );
        let mut fresh = latest.clone();
        fresh.event_id = "fresh".into();
        fresh.at_ms = AGE_MS + 101;
        append_at(&data, &fresh, fresh.at_ms).unwrap();
        assert_eq!(segments(&dir).unwrap().len(), 1);
        assert_eq!(
            read_at(&data, None, 200, fresh.at_ms).unwrap()["events"],
            json!([fresh])
        );
    }

    #[test]
    fn simultaneous_writers_preserve_facts_and_duplicate_ids_cannot_change_them() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let seed = Event::new("label", Actor::Human, Outcome::Succeeded);
        append(&data, &seed).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let data = &data;
                scope.spawn(move || {
                    for _ in 0..5 {
                        let event = Event::new("label", Actor::Agent, Outcome::Succeeded);
                        let deadline = Instant::now() + Duration::from_secs(5);
                        // BUSY is a bounded admission result, not lost data. Retry
                        // the same event ID, just as the daemon's pending queue does.
                        loop {
                            match append(data, &event) {
                                Ok(()) => break,
                                Err(e) if e.code == "BUSY" && Instant::now() < deadline => {
                                    std::thread::sleep(Duration::from_millis(10));
                                }
                                Err(e) => panic!("journal append failed: {e}"),
                            }
                        }
                    }
                });
            }
        });
        assert_eq!(
            read(&data, None, 200).unwrap()["events"]
                .as_array()
                .unwrap()
                .len(),
            21
        );
        let mut duplicate = seed.clone();
        duplicate.at_ms += 1;
        append(&data, &duplicate).unwrap();
        duplicate.outcome = Outcome::Failed;
        assert_eq!(
            append(&data, &duplicate).unwrap_err().code,
            "JOURNAL_CONFLICT"
        );
        let dir = directory(&data, false).unwrap().unwrap();
        let _held = lock(&dir, true).unwrap();
        let started = Instant::now();
        assert_eq!(read(&data, None, 1).unwrap_err().code, "BUSY");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn partial_batch_results_and_warning_shapes_preserve_original_business_results() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let id = Uuid::new_v4();
        let error = Error::new("IO_ERROR", "private path and secret text")
            .details(json!({"completed":[{"installation_id":id}],"remaining":2}));
        let returned = record_result(&data, "import", Actor::Agent, None, Err(error)).unwrap_err();
        assert_eq!(returned.details["remaining"], 2);
        let page = read(&data, None, 10).unwrap();
        assert_eq!(page["events"][0]["installation_id"], json!(id));
        assert_eq!(page["events"][0]["outcome"], "succeeded");
        assert_eq!(page["events"][1]["outcome"], "failed");
        assert!(!page.to_string().contains("secret"));
        fs::write(data.join("journal/0.jsonl"), b"broken").unwrap();
        let receipts = json!([{"installation_id":id}]);
        let result = record_result(&data, "detach", Actor::Human, None, Ok(receipts)).unwrap();
        assert!(result.is_array());
        assert_eq!(result[0]["installation_id"], json!(id));
        assert_eq!(result[0]["warnings"][0]["code"], "JOURNAL_WRITE_FAILED");
    }

    #[cfg(unix)]
    #[test]
    fn journal_links_and_nonregular_segments_are_rejected_without_touching_targets() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        storage::private_directory(&data).unwrap();
        let external = t.path().join("external");
        fs::write(&external, b"untouched").unwrap();
        symlink(&external, data.join("journal")).unwrap();
        let e = Event::new("stop", Actor::Human, Outcome::Succeeded);
        assert!(append(&data, &e).is_err());
        assert!(read(&data, None, 1).is_err());
        fs::remove_file(data.join("journal")).unwrap();
        append(&data, &e).unwrap();
        fs::remove_file(data.join("journal/0.jsonl")).unwrap();
        symlink(&external, data.join("journal/0.jsonl")).unwrap();
        assert!(append(&data, &e).is_err());
        assert!(read(&data, None, 1).is_err());
        assert_eq!(fs::read(external).unwrap(), b"untouched");
    }

    #[test]
    fn pagination_dedup_and_expiry_are_explicit_and_reads_do_not_create_storage() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        assert_eq!(read_at(&data, None, 2, 100).unwrap()["events"], json!([]));
        assert!(!data.exists());
        let mut e = Event::new("start", Actor::Agent, Outcome::Accepted);
        e.at_ms = 100;
        append_at(&data, &e, 100).unwrap();
        append_at(&data, &e, 100).unwrap();
        let mut done = e.clone();
        done.event_id = Uuid::new_v4().to_string();
        done.outcome = Outcome::Succeeded;
        append_at(&data, &done, 100).unwrap();
        let page = read_at(&data, None, 1, 100).unwrap();
        assert_eq!(page["events"].as_array().unwrap().len(), 1);
        assert_eq!(page["has_more"], true);
        let next = read_at(&data, page["next_cursor"].as_str(), 1, 100).unwrap();
        assert_eq!(next["events"][0]["outcome"], "succeeded");
        assert_eq!(next["has_more"], false);
        assert_eq!(
            read_at(&data, Some(&e.event_id), 1, AGE_MS + 101)
                .unwrap_err()
                .code,
            "JOURNAL_CURSOR_EXPIRED"
        );
        assert!(read(&data, None, 201).is_err());
    }
    #[test]
    fn interrupted_tail_is_not_silently_discarded() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let e = Event::new("label", Actor::Human, Outcome::Succeeded);
        append(&data, &e).unwrap();
        let path = data.join("journal/0.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"partial\":")
            .unwrap();
        let before = fs::read(&path).unwrap();
        assert_eq!(read(&data, None, 10).unwrap_err().code, "JOURNAL_INVALID");
        assert!(append(&data, &Event::new("stop", Actor::Human, Outcome::Succeeded)).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn out_of_range_timestamps_are_errors_instead_of_json_response_panics() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let mut e = Event::new("label", Actor::Human, Outcome::Succeeded);
        e.at_ms = u128::MAX;
        assert_eq!(append(&data, &e).unwrap_err().code, "JOURNAL_INVALID");
        assert!(!data.exists());
        e.at_ms = now();
        append(&data, &e).unwrap();
        e.at_ms = u128::MAX;
        let mut bytes = serde_json::to_vec(&e).unwrap();
        bytes.push(b'\n');
        fs::write(data.join("journal/0.jsonl"), bytes).unwrap();
        assert_eq!(read(&data, None, 1).unwrap_err().code, "JOURNAL_INVALID");
    }
}
