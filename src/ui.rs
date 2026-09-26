//! Desktop appearance only; never a source of lifecycle state.
use crate::{Error, Result, storage};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: Theme::System,
        }
    }
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
    Ok(value)
}
pub fn save_theme(data: &Path, theme: Theme) -> Result<Preferences> {
    // Preserve damaged/unknown versions for the user instead of silently resetting.
    let mut value = load(data)?;
    value.theme = theme;
    storage::private_directory(data)?;
    storage::publish(
        &data.join("ui.json"),
        &serde_json::to_vec(&value).unwrap(),
        true,
    )?;
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
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
