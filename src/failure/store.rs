use super::*;
use fs2::FileExt;
use std::{fs, path::PathBuf};

const AGE_MS: u128 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub last_success_at_ms: Option<u128>,
    pub last_failure: Option<Reference>,
    #[serde(default)]
    pub last_started_at_ms: Option<u128>,
    #[serde(default)]
    pub last_finished_at_ms: Option<u128>,
    #[serde(default)]
    pub last_result: Option<String>,
    #[serde(default)]
    pub last_run: Option<RunObservation>,
    #[serde(default)]
    pub last_success_context: Option<LaunchContext>,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub failure_id: Uuid,
    pub at_ms: u128,
    pub code: String,
}

fn directory(data: &Path, id: Uuid) -> PathBuf {
    data.join("failures").join(id.to_string())
}
fn check_directory(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)
        .map_err(|e| storage::io_error(path, e))?
        .is_dir()
    {
        return Err(invalid("Failure storage must be a real directory"));
    }
    storage::check_private_directory(path)
}
fn writer(data: &Path, id: Uuid) -> Result<(PathBuf, fs::File)> {
    storage::private_directory(data)?;
    let root = data.join("failures");
    storage::private_directory(&root)?;
    check_directory(&root)?;
    let dir = directory(data, id);
    storage::private_directory(&dir)?;
    check_directory(&dir)?;
    let path = dir.join(".lock");
    let mut opts = fs::OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = opts.open(&path).map_err(|e| storage::io_error(&path, e))?;
    if !lock
        .metadata()
        .map_err(|e| storage::io_error(&path, e))?
        .is_file()
    {
        return Err(invalid("Invalid failure lock"));
    }
    // Tail capture and lifecycle history can write the same installation at once.
    // Absorb short contention without retrying an entire evidence publication.
    let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            // Windows lock contention uses ERROR_LOCK_VIOLATION, not WouldBlock.
            Err(e) if storage::lock_contended(&e) => {
                if std::time::Instant::now() >= until {
                    return Err(Error::new("BUSY", "Failure evidence writer is busy"));
                }
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
            Err(e) => return Err(storage::io_error(&path, e)),
        }
    }
    Ok((dir, lock))
}
fn history(dir: &Path) -> Result<History> {
    match storage::read(&dir.join("history.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid failure history")),
        Err(e) if e.code == "NOT_FOUND" => Ok(History::default()),
        Err(e) => Err(e),
    }
}

/// The small history file remains useful after detailed evidence is expired.
pub fn read_history(data: &Path, id: Uuid) -> Result<History> {
    let dir = directory(data, id);
    match fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(History::default()),
        Err(e) => return Err(storage::io_error(&dir, e)),
        Ok(_) => {}
    }
    check_directory(&data.join("failures"))?;
    check_directory(&dir)?;
    history(&dir)
}

pub(crate) fn observe_run(data: &Path, id: Uuid, run: &RunObservation) -> Result<()> {
    let (dir, _lock) = writer(data, id)?;
    let mut h = history(&dir)?;
    let before = encode(&h)?;
    // Reconciliation of an old instance cannot erase a newer success baseline.
    if h.last_started_at_ms
        .is_none_or(|at| run.started_at_ms >= at)
    {
        h.last_started_at_ms = Some(run.started_at_ms);
        h.last_run = Some(run.clone());
        if let Some(result) = &run.result {
            h.last_result = Some(result.clone());
        }
    }
    if let Some(at) = run.finished_at_ms {
        h.last_finished_at_ms = Some(h.last_finished_at_ms.unwrap_or(0).max(at));
    }
    if let Some(at) = run.success_at_ms
        && h.last_success_at_ms.is_none_or(|saved| at >= saved)
    {
        h.last_success_at_ms = Some(at);
        h.last_success_context = run.context.clone();
    }
    let after = encode(&h)?;
    if before != after {
        storage::publish(&dir.join("history.json"), &after, true)?;
    }
    Ok(())
}
fn records(dir: &Path, id: Uuid) -> Result<Vec<Failure>> {
    let mut out = vec![];
    let mut budget = 24 * 1024 * 1024;
    for (n, entry) in fs::read_dir(dir)
        .map_err(|e| storage::io_error(dir, e))?
        .enumerate()
    {
        if n >= 4096 {
            return Err(Error::new(
                "FAILURE_LIMIT",
                "Too many failure directory entries",
            ));
        }
        let entry = entry.map_err(|e| storage::io_error(dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(fid) = name
            .strip_suffix(".json")
            .and_then(|s| Uuid::parse_str(s).ok())
            .filter(|u| name == format!("{u}.json"))
        else {
            continue;
        };
        let bytes = storage::read_budgeted(&entry.path(), &mut budget)?;
        let f: Failure =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid failure record"))?;
        if f.schema_version != 1 || f.installation_id != id || f.failure_id != fid {
            return Err(invalid("Failure identity or version mismatch"));
        }
        out.push(f);
    }
    out.sort_by_key(|f| std::cmp::Reverse((f.at_ms, f.failure_id)));
    Ok(out)
}
pub(super) fn save(data: &Path, failure: &Failure, now: u128) -> Result<()> {
    let mut bounded = failure.clone();
    if serde_json::to_vec(&bounded.details)
        .map_err(|e| invalid(&e.to_string()))?
        .len()
        > 64 * 1024
    {
        bounded.details = json!({"truncated":true});
        bounded.gaps.push("evidence_byte_limit".into());
    }
    let bytes = loop {
        match encode(&bounded) {
            Ok(bytes) => break bytes,
            Err(e)
                if e.code == "FAILURE_LIMIT"
                    && bounded.output.iter().any(|o| !o.text.is_empty()) =>
            {
                if !bounded.gaps.iter().any(|g| g == "evidence_byte_limit") {
                    bounded.gaps.push("evidence_byte_limit".into());
                }
                for output in &mut bounded.output {
                    let mut end = output.text.len() / 2;
                    while !output.text.is_char_boundary(end) {
                        end -= 1;
                    }
                    output.text.truncate(end);
                }
            }
            Err(e) => return Err(e),
        }
    };
    let (dir, _lock) = writer(data, failure.installation_id)?;
    let records = records(&dir, failure.installation_id)?;
    if now.saturating_sub(failure.at_ms) > AGE_MS
        || records
            .iter()
            .filter(|r| r.failure_id != failure.failure_id && r.at_ms > failure.at_ms)
            .count()
            >= 20
    {
        // A delayed tail must not resurrect evidence already evicted by newer failures.
        return Ok(());
    }
    // Prune only authoritative records belonging to this installation. Unknown
    // files remain untouched; a symlink bearing a valid record name is an error.
    let mut kept = 0;
    for old in records
        .iter()
        .filter(|old| old.failure_id != failure.failure_id)
    {
        if kept >= 19 || now.saturating_sub(old.at_ms) > AGE_MS {
            let path = dir.join(format!("{}.json", old.failure_id));
            fs::remove_file(&path).map_err(|e| storage::io_error(&path, e))?;
        } else {
            kept += 1;
        }
    }
    storage::publish(
        &dir.join(format!("{}.json", failure.failure_id)),
        &bytes,
        true,
    )?;
    let mut h = history(&dir)?;
    if h.last_failure
        .as_ref()
        .is_none_or(|last| last.at_ms <= failure.at_ms)
    {
        h.last_failure = Some(Reference {
            failure_id: failure.failure_id,
            at_ms: failure.at_ms,
            code: failure.evidence.code.clone(),
        });
        storage::publish(&dir.join("history.json"), &encode(&h)?, true)?;
    }
    Ok(())
}
#[cfg(test)]
pub(super) fn success(data: &Path, id: Uuid, at: u128) -> Result<()> {
    let (dir, _lock) = writer(data, id)?;
    let mut h = history(&dir)?;
    h.last_success_at_ms = Some(h.last_success_at_ms.unwrap_or(0).max(at));
    storage::publish(&dir.join("history.json"), &encode(&h)?, true)
}
/// Offline-safe: no daemon start and no file creation or retention side effects.
pub fn read(data: &Path, id: Uuid, failure_id: Option<Uuid>) -> Result<Value> {
    let dir = directory(data, id);
    check_directory(&data.join("failures"))?;
    check_directory(&dir)?;
    let h = history(&dir)?;
    let records = records(&dir, id)?;
    let selected = failure_id.or_else(|| h.last_failure.as_ref().map(|r| r.failure_id));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut f = records
        .iter()
        .find(|f| Some(f.failure_id) == selected && now.saturating_sub(f.at_ms) <= AGE_MS)
        .cloned();
    if let Some(f) = &mut f
        && f.collecting
        && now > f.at_ms.saturating_add(10000)
    {
        // A crashed recorder cannot leave the offline UI "collecting" forever.
        f.collecting = false;
        f.gaps.push("tail_capture_not_completed".into());
    }
    let consecutive = f.as_ref().map(|f| {
        records
            .iter()
            .filter(|r| r.at_ms <= f.at_ms && h.last_success_at_ms.is_none_or(|at| r.at_ms > at))
            .take_while(|r| r.evidence.code == f.evidence.code && r.service == f.service)
            .count()
    });
    Ok(
        json!({"installation_id":id,"failure":f,"history":h,"consecutive_same_cause":consecutive,"count_limited_by_retention":consecutive==Some(20),
        "records":records.iter().map(|f| json!({"failure_id":f.failure_id,"at_ms":f.at_ms,"code":f.evidence.code})).collect::<Vec<_>>(),
        "details_expired":selected.is_some()&&f.is_none()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writer_waits_for_short_contention_but_remains_bounded() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let id = Uuid::new_v4();
        let (_, lock) = writer(&data, id).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            drop(lock);
        });
        let (_, acquired) = writer(&data, id).unwrap();
        release.join().unwrap();
        let start = std::time::Instant::now();
        assert_eq!(writer(&data, id).unwrap_err().code, "BUSY");
        assert!(start.elapsed() >= std::time::Duration::from_millis(190));
        drop(acquired);
    }
    #[test]
    fn history_keeps_new_success_context_and_reads_missing_without_writes() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let id = Uuid::new_v4();
        assert!(read_history(&data, id).unwrap().last_run.is_none());
        assert!(!data.exists());
        let context = LaunchContext {
            branch: Some("main".into()),
            launch_sha256: "a".repeat(64),
            manifest_source: "project".into(),
            confirmed_actor: None,
            environment_sha256: "b".repeat(64),
        };
        let recent = RunObservation {
            instance_id: Uuid::new_v4(),
            started_at_ms: 200,
            success_at_ms: Some(220),
            finished_at_ms: Some(250),
            result: Some("stopped".into()),
            context: Some(context.clone()),
        };
        observe_run(&data, id, &recent).unwrap();
        let old = RunObservation {
            instance_id: Uuid::new_v4(),
            started_at_ms: 100,
            success_at_ms: Some(110),
            finished_at_ms: Some(150),
            result: Some("failed".into()),
            context: None,
        };
        observe_run(&data, id, &old).unwrap();
        let saved = read_history(&data, id).unwrap();
        assert_eq!(saved.last_run, Some(recent));
        assert_eq!(saved.last_success_context, Some(context));
        assert_eq!(saved.last_result.as_deref(), Some("stopped"));
        assert_eq!(saved.last_finished_at_ms, Some(250));
    }
    #[test]
    fn abandoned_tail_becomes_an_explicit_gap_and_escaped_output_stays_bounded() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let id = Uuid::new_v4();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let mut f = Failure::new(id, "UNKNOWN", now - 11000);
        f.collecting = true;
        f.output.push(Output {
            snapshot: "initial".into(),
            sequence: 0,
            observed_ms: now - 11000,
            service: "web".into(),
            stream: "stderr".into(),
            text: "\0".repeat(200000),
        });
        save(&data, &f, now).unwrap();
        let value = read(&data, id, None).unwrap();
        assert_eq!(value["failure"]["collecting"], false);
        assert!(
            value["failure"]["gaps"]
                .as_array()
                .unwrap()
                .contains(&json!("tail_capture_not_completed"))
        );
        assert!(serde_json::to_vec(&value).unwrap().len() < 1024 * 1024);
    }

    #[cfg(unix)]
    #[test]
    fn valid_record_name_symlinks_are_rejected_and_unrelated_entries_are_preserved() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let id = Uuid::new_v4();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let f = Failure::new(id, "UNKNOWN", now);
        save(&data, &f, now).unwrap();
        let dir = directory(&data, id);
        fs::write(dir.join(".DS_Store"), b"unrelated").unwrap();
        assert!(read(&data, id, None).is_ok());
        let path = dir.join(format!("{}.json", f.failure_id));
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(dir.join(".DS_Store"), &path).unwrap();
        assert!(read(&data, id, None).is_err());
        assert_eq!(fs::read(dir.join(".DS_Store")).unwrap(), b"unrelated");
    }
    #[test]
    fn history_survives_retention_and_old_tail_does_not_replace_newer_failure() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let id = Uuid::new_v4();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let mut first = Failure::new(id, "INVALID_COMMAND", now - AGE_MS - 1);
        save(&data, &first, first.at_ms).unwrap();
        for i in 0..22 {
            save(&data, &Failure::new(id, "PORT_IN_USE", now + i), now + i).unwrap();
        }
        let dir = directory(&data, id);
        assert_eq!(records(&dir, id).unwrap().len(), 20);
        assert!(
            read(&data, id, Some(first.failure_id)).unwrap()["details_expired"]
                .as_bool()
                .unwrap()
        );
        first.at_ms = now - 1;
        save(&data, &first, now + 22).unwrap();
        assert_eq!(
            read(&data, id, None).unwrap()["failure"]["cause_code"],
            "PORT_IN_USE"
        );
        success(&data, id, now + 100).unwrap();
        assert_eq!(read(&data, id, None).unwrap()["consecutive_same_cause"], 0);
    }
    #[test]
    fn missing_read_never_creates_storage_and_corruption_is_not_hidden() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("missing");
        let id = Uuid::new_v4();
        assert!(read(&data, id, None).is_err());
        assert!(!data.exists());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let f = Failure::new(id, "UNKNOWN", now);
        save(&data, &f, now).unwrap();
        fs::write(
            directory(&data, id).join(format!("{}.json", f.failure_id)),
            b"bad",
        )
        .unwrap();
        assert_eq!(read(&data, id, None).unwrap_err().code, "FAILURE_INVALID");
    }
}
