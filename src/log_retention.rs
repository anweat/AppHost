//! Global ordinary-log policy; failure evidence retains its independent lifetime.
use crate::{Error, Result, storage};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    pub days: u16,
    pub max_mib: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            days: 7,
            max_mib: 100,
        }
    }
}
impl Settings {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || !(1..=365).contains(&self.days)
            || !(10..=10240).contains(&self.max_mib)
        {
            return Err(Error::new(
                "INVALID_LOG_RETENTION",
                "days must be 1..365 and max_mib 10..10240 (per installation)",
            ));
        }
        Ok(())
    }
}
pub fn load(data: &Path) -> Result<Settings> {
    let policy = match storage::read(&data.join("log-retention.json")) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes)
            .map_err(|_| Error::new("INVALID_LOG_RETENTION", "Invalid log retention settings"))?,
        Err(e) if e.code == "NOT_FOUND" => Settings::default(),
        Err(e) => return Err(e),
    };
    policy.validate()?;
    Ok(policy)
}
/// Complete replacement makes concurrent saves atomic without a read/merge race.
/// Lower limits take effect on the next ordinary archive cleanup, never mid-write.
pub fn save(data: &Path, days: u16, max_mib: u32) -> Result<Settings> {
    let settings = Settings {
        schema_version: 1,
        days,
        max_mib,
    };
    settings.validate()?;
    storage::private_directory(data)?;
    storage::publish(
        &data.join("log-retention.json"),
        &serde_json::to_vec(&settings).unwrap(),
        true,
    )?;
    Ok(settings)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_read_only_and_invalid_changes_preserve_previous_policy() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        assert_eq!(load(&data).unwrap(), Settings::default());
        assert!(!data.exists());
        save(&data, 1, 10).unwrap();
        for (days, mib) in [(0, 10), (366, 10), (7, 9), (7, 10241)] {
            assert!(save(&data, days, mib).is_err());
        }
        assert_eq!(load(&data).unwrap().days, 1);
        save(&data, 365, 10240).unwrap();
        assert_eq!(load(&data).unwrap().max_mib, 10240);
    }
}
