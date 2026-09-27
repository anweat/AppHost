//! Desktop appearance only; never a source of lifecycle state.
use crate::{Error, Result, storage};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use uuid::Uuid;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub schema_version: u32,
    pub theme: Theme,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub collapsed: BTreeSet<String>,
    #[serde(default = "yes")]
    pub show_recent: bool,
    #[serde(default)]
    pub last_used: BTreeMap<Uuid, u64>,
    #[serde(default)]
    pub search: Search,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: Theme::System,
            groups: Vec::new(),
            collapsed: BTreeSet::new(),
            show_recent: true,
            last_used: BTreeMap::new(),
            search: Search::default(),
        }
    }
}
fn yes() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    pub members: BTreeSet<Uuid>,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Enter {
    #[default]
    Primary,
    Details,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub enter: Enter,
    pub match_path: bool,
    pub show_actions: bool,
}
impl Default for Search {
    fn default() -> Self {
        Self {
            enter: Enter::Primary,
            match_path: false,
            show_actions: true,
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Edit {
    Prune,
    Search {
        search: Search,
    },
    Recent {
        enabled: bool,
    },
    CreateGroup {
        name: String,
    },
    RenameGroup {
        id: Uuid,
        name: String,
    },
    DissolveGroup {
        id: Uuid,
    },
    Move {
        installation_id: Uuid,
        group_id: Option<Uuid>,
    },
    Collapse {
        key: String,
        collapsed: bool,
    },
    Touch {
        installation_id: Uuid,
    },
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}
fn validate(value: &Preferences) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut members = BTreeSet::new();
    if value.groups.len() > 128
        || value.collapsed.len() > 256
        || value.last_used.len() > 1024
        || value.groups.iter().any(|g| {
            !valid_name(&g.name)
                || !ids.insert(g.id)
                || g.members.iter().any(|id| !members.insert(*id))
        })
        || members.len() > 1024
        || value.collapsed.iter().any(|k| k.len() > 4096)
    {
        return Err(Error::new(
            "UI_INVALID",
            "Invalid or oversized appearance preferences",
        ));
    }
    Ok(())
}
pub fn load(data: &Path) -> Result<Preferences> {
    let value: Preferences = match storage::read(&data.join("ui.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("UI_INVALID", "Invalid appearance preferences"))?,
        Err(e) if e.code == "NOT_FOUND" => return Ok(Preferences::default()),
        Err(e) => return Err(e),
    };
    if value.schema_version != 1 {
        return Err(Error::new("UI_INVALID", "Unsupported appearance version"));
    }
    validate(&value)?;
    Ok(value)
}
fn update(data: &Path, edit: impl FnOnce(&mut Preferences) -> Result<()>) -> Result<Preferences> {
    storage::private_directory(data)?;
    let path = data.join("ui.lock");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let lock = options
        .open(&path)
        .map_err(|e| storage::io_error(&path, e))?;
    if !lock
        .metadata()
        .map_err(|e| storage::io_error(&path, e))?
        .is_file()
    {
        return Err(Error::new("UI_INVALID", "Invalid UI lock"));
    }
    // A short metadata write may overlap another window or CLI operation.
    // Wait only for contention; permission and I/O failures are not BUSY.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => break,
            // Windows lock contention uses ERROR_LOCK_VIOLATION, not WouldBlock.
            Err(e) if storage::lock_contended(&e) => {
                if std::time::Instant::now() >= deadline {
                    return Err(Error::new("BUSY", "Appearance settings are being saved"));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => return Err(storage::io_error(&path, e)),
        }
    }
    let mut value = load(data)?;
    edit(&mut value)?;
    validate(&value)?;
    storage::publish(
        &data.join("ui.json"),
        &serde_json::to_vec(&value).unwrap(),
        true,
    )?;
    Ok(value)
}
pub fn save_theme(data: &Path, theme: Theme) -> Result<Preferences> {
    update(data, |v| {
        v.theme = theme;
        Ok(())
    })
}
pub fn edit(data: &Path, change: Edit) -> Result<Preferences> {
    update(data, |v| {
        let missing = || {
            Error::new(
                "UI_GROUP_MISSING",
                "Group no longer exists; refresh preferences",
            )
        };
        match change {
            Edit::Prune => {
                // Read the atomic registry under the UI lock; WebView does not
                // decide which installation identities may be removed.
                let active = crate::registry::active_ids(data)?;
                for group in &mut v.groups {
                    group.members.retain(|id| active.contains(id));
                }
                v.last_used.retain(|id, _| active.contains(id));
            }
            Edit::Search { search } => v.search = search,
            Edit::Recent { enabled } => v.show_recent = enabled,
            Edit::CreateGroup { name } => v.groups.push(Group {
                id: Uuid::new_v4(),
                name: name.trim().into(),
                members: BTreeSet::new(),
            }),
            Edit::RenameGroup { id, name } => {
                v.groups
                    .iter_mut()
                    .find(|g| g.id == id)
                    .ok_or_else(missing)?
                    .name = name.trim().into()
            }
            Edit::DissolveGroup { id } => {
                if !v.groups.iter().any(|g| g.id == id) {
                    return Err(missing());
                }
                v.groups.retain(|g| g.id != id);
                v.collapsed.remove(&id.to_string());
            }
            Edit::Move {
                installation_id,
                group_id,
            } => {
                crate::registry::registered(data, installation_id)?;
                if group_id.is_some_and(|id| !v.groups.iter().any(|g| g.id == id)) {
                    return Err(missing());
                }
                for g in &mut v.groups {
                    g.members.remove(&installation_id);
                    if Some(g.id) == group_id {
                        g.members.insert(installation_id);
                    }
                }
            }
            Edit::Collapse { key, collapsed } => {
                if collapsed {
                    v.collapsed.insert(key);
                } else {
                    v.collapsed.remove(&key);
                }
            }
            Edit::Touch { installation_id } => {
                crate::registry::registered(data, installation_id)?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                v.last_used.insert(installation_id, now);
                if v.last_used.len() > 1024
                    && let Some(id) = v
                        .last_used
                        .iter()
                        .min_by_key(|(_, at)| *at)
                        .map(|(id, _)| *id)
                {
                    v.last_used.remove(&id);
                }
            }
        }
        Ok(())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pruning_detached_references_preserves_groups_and_other_settings() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let root = t.path().join("app");
        std::fs::create_dir(&root).unwrap();
        let manifest:crate::Manifest=serde_json::from_value(serde_json::json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture","services":[{"name":"worker","command":["unused"]}]})).unwrap();
        std::fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        let id = crate::import(&data, &crate::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        let value = edit(
            &data,
            Edit::CreateGroup {
                name: "Work".into(),
            },
        )
        .unwrap();
        let group = value.groups[0].id;
        edit(
            &data,
            Edit::Move {
                installation_id: id,
                group_id: Some(group),
            },
        )
        .unwrap();
        edit(
            &data,
            Edit::Touch {
                installation_id: id,
            },
        )
        .unwrap();
        edit(
            &data,
            Edit::Collapse {
                key: group.to_string(),
                collapsed: true,
            },
        )
        .unwrap();
        assert!(
            edit(&data, Edit::Prune)
                .unwrap()
                .last_used
                .contains_key(&id)
        );
        crate::detach(&data, id).unwrap();
        let after = edit(&data, Edit::Prune).unwrap();
        assert_eq!(after.groups[0].name, "Work");
        assert!(after.groups[0].members.is_empty());
        assert!(after.last_used.is_empty());
        assert!(after.collapsed.contains(&group.to_string()));
        assert!(root.join("apphost.toml").is_file());
        assert_eq!(load(&data).unwrap(), after);
    }
    #[test]
    fn ui_writer_waits_for_short_contention_but_has_a_deadline() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        save_theme(&data, Theme::Light).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(data.join("ui.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            drop(lock);
        });
        save_theme(&data, Theme::Dark).unwrap();
        thread.join().unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(data.join("ui.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        assert_eq!(save_theme(&data, Theme::Light).unwrap_err().code, "BUSY");
        assert_eq!(load(&data).unwrap().theme, Theme::Dark);
    }
    #[test]
    fn legacy_preferences_default_new_fields_and_edits_preserve_other_choices() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        save_theme(&data, Theme::Light).unwrap();
        std::fs::write(
            data.join("ui.json"),
            r#"{"schema_version":1,"theme":"dark"}"#,
        )
        .unwrap();
        assert!(load(&data).unwrap().show_recent);
        edit(
            &data,
            Edit::CreateGroup {
                name: "Study".into(),
            },
        )
        .unwrap();
        let before = edit(&data, Edit::Recent { enabled: false }).unwrap();
        save_theme(&data, Theme::Light).unwrap();
        let after = load(&data).unwrap();
        assert_eq!(before.groups, after.groups);
        assert!(!after.show_recent);
        assert!(edit(&data, Edit::CreateGroup { name: " ".into() }).is_err());
        assert_eq!(load(&data).unwrap(), after);
        assert!(
            edit(
                &data,
                Edit::Move {
                    installation_id: Uuid::new_v4(),
                    group_id: None
                }
            )
            .is_err()
        );
        assert_eq!(load(&data).unwrap(), after);
        edit(
            &data,
            Edit::DissolveGroup {
                id: after.groups[0].id,
            },
        )
        .unwrap();
        assert!(load(&data).unwrap().groups.is_empty());
        assert!(!data.join("runtime").exists());
    }
    #[test]
    fn theme_is_read_only_until_saved_and_bad_files_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        assert_eq!(load(&data).unwrap(), Preferences::default());
        assert!(!data.exists());
        save_theme(&data, Theme::Light).unwrap();
        assert_eq!(load(&data).unwrap().theme, Theme::Light);
        let path = data.join("ui.json");
        std::fs::write(&path, "{bad}").unwrap();
        assert!(save_theme(&data, Theme::Dark).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{bad}");
    }
}
