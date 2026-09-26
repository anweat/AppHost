//! Desktop confirmation reuses core drafts and edit guards. No persistent UI draft.
use apphost::{Draft, Error, Result, journal::Actor, registry::editing};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

#[derive(Default, Clone)]
// Keep only one bounded native-selected draft. Tokens bind indices to that exact
// scan/group result; consuming it prevents duplicate or replayed imports.
pub struct DraftBuffer(Arc<Mutex<Option<ScanResult>>>);
#[derive(Clone, serde::Serialize)]
pub struct ScanResult {
    token: Uuid,
    draft: Draft,
}
impl DraftBuffer {
    fn replace(&self, draft: Draft) -> Result<ScanResult> {
        bounded(&draft)?;
        let result = ScanResult {
            token: Uuid::new_v4(),
            draft,
        };
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(result.clone());
        Ok(result)
    }
    fn group(&self, token: Uuid, index: usize, merge: bool) -> Result<ScanResult> {
        let mut buffer = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let current = buffer
            .as_ref()
            .filter(|s| s.token == token)
            .ok_or_else(stale)?;
        let mut draft = current.draft.clone();
        if merge {
            apphost::discovery::accept_group(&mut draft, index)?;
        } else if index < draft.groups.len() {
            draft.groups.remove(index);
        } else {
            return Err(Error::new("INVALID_INPUT", "Unknown group suggestion"));
        }
        bounded(&draft)?;
        let result = ScanResult {
            token: Uuid::new_v4(),
            draft,
        };
        *buffer = Some(result.clone());
        Ok(result)
    }
    fn take(&self, token: Uuid, selected: &[usize]) -> Result<Draft> {
        let mut buffer = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let current = buffer
            .as_ref()
            .filter(|s| s.token == token)
            .ok_or_else(stale)?;
        if selected.is_empty()
            || selected.len() > current.draft.candidates.len()
            || selected
                .iter()
                .any(|i| *i >= current.draft.candidates.len())
            || !current.draft.groups.is_empty()
        {
            return Err(Error::new(
                "INVALID_INPUT",
                "Select candidates and resolve grouping before import",
            ));
        }
        let mut draft = buffer.take().expect("checked buffer").draft;
        for (index, candidate) in draft.candidates.iter_mut().enumerate() {
            candidate.selected = selected.contains(&index);
        }
        Ok(draft)
    }
}
fn stale() -> Error {
    Error::new("STALE_DRAFT", "扫描草稿已失效，请重新选择目录")
}
#[tauri::command]
pub async fn desktop_scan(
    app: tauri::AppHandle,
    buffer: tauri::State<'_, DraftBuffer>,
) -> Result<Option<ScanResult>> {
    let buffer = buffer.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(folder) = app
            .dialog()
            .file()
            .set_title("选择应用或项目集合目录")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let root = folder
            .into_path()
            .map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
        let data = apphost::paths::data_dir(None)?;
        let mut draft =
            apphost::discovery::scan_with_registry(&root, &Default::default(), Some(&data))?;
        apphost::registry::annotate_launch_changes(&data, &mut draft)?;
        buffer.replace(draft).map(Some)
    })
    .await
    .map_err(join_error)?
}

fn bounded(draft: &Draft) -> Result<()> {
    if serde_json::to_vec(draft)
        .map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?
        .len()
        > 1024 * 1024
    {
        return Err(Error::new(
            "INVALID_INPUT",
            "草稿超过 1 MiB，请选择更小的目录",
        ));
    }
    Ok(())
}
#[tauri::command]
pub async fn desktop_group(
    buffer: tauri::State<'_, DraftBuffer>,
    token: Uuid,
    index: usize,
    merge: bool,
) -> Result<ScanResult> {
    let buffer = buffer.inner().clone();
    tauri::async_runtime::spawn_blocking(move || buffer.group(token, index, merge))
        .await
        .map_err(join_error)?
}
fn receipt(value: &Value) -> Value {
    json!({"installation_id":value["installation_id"],"label":value["human_label"],
        "path":value["path"],"created_files":value["created_files"],"warnings":value["warnings"]})
}
fn import(data: &Path, mut draft: Draft) -> Result<Value> {
    bounded(&draft)?;
    // The button explicitly confirms the displayed selected declarations. Core
    // still rechecks source evidence and never overwrites existing manifests.
    for candidate in draft.candidates.iter_mut().filter(|c| c.selected) {
        if let Some(manifest) = &candidate.manifest {
            candidate.reviewed_launch_sha256 = Some(manifest.launch_sha256());
        }
    }
    let result = apphost::registry::import_as(data, &draft, Actor::Human)
        .map(|receipts| serde_json::to_value(receipts).expect("serializable receipts"));
    match apphost::journal::record_result(data, "import", Actor::Human, None, result) {
        Ok(value) => Ok(
            json!({"completed":value.as_array().into_iter().flatten().map(receipt).collect::<Vec<_>>()}),
        ),
        Err(mut error) => {
            if let Some(completed) = error
                .details
                .get_mut("completed")
                .and_then(Value::as_array_mut)
            {
                for item in completed {
                    *item = receipt(item);
                }
            }
            Err(error)
        }
    }
}
#[tauri::command]
pub async fn desktop_import(
    buffer: tauri::State<'_, DraftBuffer>,
    token: Uuid,
    selected: Vec<usize>,
) -> Result<Value> {
    let buffer = buffer.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let draft = buffer.take(token, &selected)?;
        import(&apphost::paths::data_dir(None)?, draft)
    })
    .await
    .map_err(join_error)?
}
#[tauri::command]
pub async fn desktop_manifest_read(installation_id: Uuid) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        editing::read(&apphost::paths::data_dir(None)?, installation_id)
    })
    .await
    .map_err(join_error)?
}
#[tauri::command]
pub async fn desktop_manifest_review(installation_id: Uuid, text: String) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        editing::review(
            &apphost::paths::data_dir(None)?,
            installation_id,
            Some(&text),
        )
    })
    .await
    .map_err(join_error)?
}
fn save(data: &Path, id: Uuid, request: editing::SaveRequest) -> Result<Value> {
    editing::preflight(data, id, &request)?;
    if cfg!(target_os = "macos") {
        match apphost::daemon::call(
            data,
            "manifest-save",
            json!({"installation_id":id,"edit":request}),
            false,
        ) {
            Ok(receipt) => {
                return Ok(
                    json!({"installation_id":id,"instance_id":receipt["instance_id"],"operation_id":receipt["operation_id"],"operation_done":receipt["operation_done"],"confirmed":receipt["confirmed"],"warnings":receipt["warnings"]}),
                );
            }
            Err(e) if e.code == "DAEMON_UNAVAILABLE" => {}
            Err(e) => return Err(e),
        }
    }
    // An offline GUI must not call the CLI coordinator's current_exe bootstrap.
    // A durable instance requires explicit connection before a coordinated save.
    if data
        .join("runtime")
        .join(format!("{id}.json"))
        .try_exists()
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?
    {
        return Err(Error::new(
            "DAEMON_UNAVAILABLE",
            "请先连接后台，再停止并保存启动声明",
        ));
    }
    let result = editing::save(data, id, &request, Actor::Human).map(|value| {
        json!({"installation_id":id,"confirmed":value["confirmed"],"file_written":value["file_written"]})
    });
    apphost::journal::record_result(data, "manifest-save", Actor::Human, Some(id), result)
}
#[tauri::command]
pub async fn desktop_manifest_save(
    installation_id: Uuid,
    request: editing::SaveRequest,
) -> Result<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        save(&apphost::paths::data_dir(None)?, installation_id, request)
    })
    .await
    .map_err(join_error)?
}
fn join_error(error: impl std::fmt::Display) -> Error {
    Error::new("DESKTOP_OPERATION_FAILED", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        Draft,
        String,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let root = temp.path().join("project with 'quotes'");
        std::fs::create_dir(&root).unwrap();
        let manifest: apphost::Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture","services":[{"name":"web","command":["unused"],"env":{"SECRET":"fixture-secret"}}]})).unwrap();
        let text = format!(
            "# preserve the author's formatting\n{}",
            manifest.to_toml().unwrap()
        );
        std::fs::write(root.join("apphost.toml"), &text).unwrap();
        let draft = apphost::discovery::scan_with_registry(&root, &Default::default(), Some(&data))
            .unwrap();
        (temp, data, root, draft, text)
    }
    fn request(data: &Path, id: Uuid, text: &str) -> editing::SaveRequest {
        let review = editing::review(data, id, Some(text)).unwrap();
        editing::SaveRequest {
            revision: review["revision"].as_u64().unwrap(),
            base_sha256: review["base_sha256"].as_str().unwrap().into(),
            confirmed_launch_sha256: review["launch_sha256"].as_str().unwrap().into(),
            text: Some(text.into()),
        }
    }
    fn imported_id(data: &Path, draft: Draft) -> Uuid {
        let result = import(data, draft).unwrap();
        assert!(!result.to_string().contains("fixture-secret"));
        Uuid::parse_str(result["completed"][0]["installation_id"].as_str().unwrap()).unwrap()
    }
    #[test]
    fn scan_is_read_only_import_preserves_existing_bytes_and_receipts_exclude_env() {
        let (_temp, data, root, draft, text) = fixture();
        assert!(!data.exists());
        imported_id(&data, draft);
        assert_eq!(
            std::fs::read_to_string(root.join("apphost.toml")).unwrap(),
            text
        );
    }
    #[test]
    fn stale_import_cannot_confirm_changed_source() {
        let (_temp, data, root, draft, text) = fixture();
        std::fs::write(
            root.join("apphost.toml"),
            format!("{text}\n# external edit"),
        )
        .unwrap();
        assert_eq!(import(&data, draft).unwrap_err().code, "STALE_DRAFT");
        assert!(!data.join("registry.json").exists());
    }
    #[test]
    fn offline_save_preserves_text_and_rejects_changes_since_review() {
        let (_temp, data, root, draft, text) = fixture();
        let id = imported_id(&data, draft);
        let next = format!("{text}\n# desktop edit\n");
        let edit = request(&data, id, &next);
        assert_eq!(save(&data, id, edit).unwrap()["confirmed"], true);
        assert_eq!(
            std::fs::read_to_string(root.join("apphost.toml")).unwrap(),
            next
        );
        let stale = request(&data, id, &text);
        let external = format!("{next}# written by someone else\n");
        std::fs::write(root.join("apphost.toml"), &external).unwrap();
        assert_eq!(save(&data, id, stale).unwrap_err().code, "EDIT_CONFLICT");
        assert_eq!(
            std::fs::read_to_string(root.join("apphost.toml")).unwrap(),
            external
        );
    }
    #[test]
    fn durable_runtime_requires_explicit_connection_before_save() {
        let (_temp, data, root, draft, text) = fixture();
        let id = imported_id(&data, draft);
        let runtime = data.join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::write(runtime.join(format!("{id}.json")), "{}").unwrap();
        let edit = request(&data, id, &format!("{text}\n# not saved"));
        assert_eq!(
            save(&data, id, edit).unwrap_err().code,
            "DAEMON_UNAVAILABLE"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("apphost.toml")).unwrap(),
            text
        );
    }
    #[test]
    fn native_draft_tokens_bind_selection_and_are_single_use() {
        let (_temp, _data, _root, draft, _text) = fixture();
        let buffer = DraftBuffer::default();
        let old = buffer.replace(draft.clone()).unwrap();
        let current = buffer.replace(draft).unwrap();
        assert_eq!(
            buffer.take(old.token, &[0]).unwrap_err().code,
            "STALE_DRAFT"
        );
        assert_eq!(
            buffer.take(current.token, &[999]).unwrap_err().code,
            "INVALID_INPUT"
        );
        let selected = buffer.take(current.token, &[0]).unwrap();
        assert!(selected.candidates[0].selected);
        assert_eq!(
            buffer.take(current.token, &[0]).unwrap_err().code,
            "STALE_DRAFT"
        );
    }
    #[test]
    fn grouping_decisions_must_precede_selection_and_replace_the_token() {
        let (_temp, _data, _root, mut draft, _text) = fixture();
        draft.groups.push(apphost::discovery::GroupSuggestion {
            members: vec![draft.candidates[0].path.clone()],
            candidate: draft.candidates[0].clone(),
        });
        let buffer = DraftBuffer::default();
        let first = buffer.replace(draft).unwrap();
        assert_eq!(
            buffer.take(first.token, &[0]).unwrap_err().code,
            "INVALID_INPUT"
        );
        let decided = buffer.group(first.token, 0, false).unwrap();
        assert_eq!(
            buffer.take(first.token, &[0]).unwrap_err().code,
            "STALE_DRAFT"
        );
        assert!(buffer.take(decided.token, &[0]).is_ok());
    }
}
