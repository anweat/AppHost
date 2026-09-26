//! Stable local entry points and restricted browser navigation admission.
// G2 acceptance blocker: Chrome returned ERR_BLOCKED_BY_CLIENT for the isolated
// ComfyUI origin. Safari and the other project/browser pairs were exercised, but
// this case needs manual browser resolution and revalidation before G2 sign-off.
// It does not gate the backend implementation or authorize a browser bypass.
use crate::{Error, Result, storage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use uuid::Uuid;
pub(crate) mod server;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub port: u16,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            port: 18480,
        }
    }
}
pub fn config(data: &Path) -> Result<Config> {
    let bytes = match storage::read(&data.join("gateway.json")) {
        Ok(bytes) => bytes,
        Err(e) if e.code == "NOT_FOUND" => return Ok(Config::default()),
        Err(e) => return Err(e),
    };
    let config: Config = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("GATEWAY_CONFIG_INVALID", "Invalid gateway settings"))?;
    if config.schema_version != 1 || config.port == 0 {
        return Err(Error::new(
            "GATEWAY_CONFIG_INVALID",
            "Expected version 1 and a nonzero port",
        ));
    }
    Ok(config)
}

pub fn save_config(data: &Path, port: u16) -> Result<Config> {
    if port == 0 {
        return Err(Error::new("INVALID_PORT", "Gateway port must be nonzero"));
    }
    let _lock = config_lock(data)?;
    publish_config(data, port)
}
pub(crate) fn config_lock(data: &Path) -> Result<std::fs::File> {
    storage::private_directory(data)?;
    let path = data.join("gateway.lock");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
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
        return Err(Error::new(
            "GATEWAY_CONFIG_INVALID",
            "Gateway lock must be a regular file",
        ));
    }
    fs2::FileExt::try_lock_exclusive(&file)
        .map_err(|_| Error::new("BUSY", "Another gateway configuration is being saved"))?;
    Ok(file)
}
pub(crate) fn publish_config(data: &Path, port: u16) -> Result<Config> {
    if port == 0 {
        return Err(Error::new("INVALID_PORT", "Gateway port must be nonzero"));
    }
    storage::private_directory(data)?;
    config(data)?; // An explicit port edit is not permission to reset an unknown schema.
    let config = Config {
        schema_version: 1,
        port,
    };
    storage::publish(
        &data.join("gateway.json"),
        &serde_json::to_vec(&config)
            .map_err(|e| Error::new("GATEWAY_CONFIG_INVALID", e.to_string()))?,
        true,
    )?;
    Ok(config)
}

pub fn set_port(data: &Path, port: u16) -> Result<Value> {
    let actor = crate::journal::Actor::from_environment()?;
    match crate::daemon::call(data, "gateway-set-port", json!({"port":port}), false) {
        Err(e) if e.code == "DAEMON_UNAVAILABLE" => {
            let result = save_config(data, port).map(|c| json!({"config":c,"live":false}));
            crate::journal::record_result(data, "gateway-set-port", actor, None, result)
        }
        other => other,
    }
}

pub fn url(data: &Path, id: Uuid) -> Result<Value> {
    let snapshot = crate::registry::launch_snapshot(data, id)?;
    let target = snapshot.manifest().open_target()?;
    let Some(target) = target else {
        return Ok(json!({"installation_id":id,"url":null}));
    };
    let config = config(data)?;
    Ok(
        json!({"installation_id":id,"url":format!("http://{}.localhost:{}{}",
        snapshot.installation().gateway_slug(), config.port, target.path),"service":target.service}),
    )
}

/// Inspect raw header pairs so duplicate metadata cannot be silently collapsed
/// by an HTTP adapter. Values are tokens and deliberately case-sensitive.
pub fn is_wake_navigation(method: &str, headers: &[(&str, &str)]) -> bool {
    if method != "GET" || headers.len() > 128 {
        return false;
    }
    let names = [
        "sec-fetch-dest",
        "sec-fetch-mode",
        "sec-fetch-site",
        "sec-fetch-user",
        "sec-purpose",
        "purpose",
    ];
    let mut values = [None; 6];
    for (name, value) in headers {
        if let Some(index) = names.iter().position(|n| name.eq_ignore_ascii_case(n)) {
            if values[index].is_some()
                || value.len() > 256
                || value.bytes().any(|b| b.is_ascii_control() && b != b'\t')
            {
                return false;
            }
            values[index] = Some(value.trim_matches([' ', '\t']));
        }
    }
    if values[0] != Some("document")
        || values[1] != Some("navigate")
        || values[2] != Some("none")
        || values[3].is_some_and(|v| v != "?1")
    {
        return false;
    }
    // Safari may omit Sec-Fetch-User. Site=none still distinguishes browser UI
    // navigation from ordinary web-page initiators; never infer it from the UA.
    !values[4..].iter().flatten().any(|v| {
        v.split(|c: char| c.is_ascii_whitespace() || c == ';' || c == ',')
            .any(|token| {
                token.eq_ignore_ascii_case("prefetch") || token.eq_ignore_ascii_case("prerender")
            })
    })
}
