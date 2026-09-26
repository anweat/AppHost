//! Platform process-group primitives. Trust checks and runtime persistence belong to L1.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub pid: u32,
    pub group: u32,
    pub uid: u32,
    pub started_seconds: u64,
    pub started_microseconds: u64,
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(target_os = "macos"))]
fn unsupported() -> crate::Error {
    crate::Error::new("UNSUPPORTED_PLATFORM", "Process management requires macOS")
}

#[cfg(not(target_os = "macos"))]
pub fn spawn_group(_: &mut std::process::Command) -> crate::Result<std::process::Child> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
pub fn observe_exit(_: &std::process::Child) -> crate::Result<Option<std::process::ExitStatus>> {
    Err(unsupported())
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    group: u32,
    boot: (i64, i64),
    known: Vec<Identity>,
    exhausted: bool,
}

#[cfg(not(target_os = "macos"))]
impl Group {
    pub fn capture_spawned(_: &std::process::Child) -> crate::Result<Self> {
        Err(unsupported())
    }
    pub fn id(&self) -> u32 {
        self.group
    }
    pub fn reconcile(&mut self) -> crate::Result<Vec<Identity>> {
        Err(unsupported())
    }
    pub fn port_ready(&mut self, _: u16) -> crate::Result<bool> {
        Err(unsupported())
    }
    pub(crate) fn port_listening(_: &[Identity], _: u16) -> crate::Result<bool> {
        Err(unsupported())
    }
    pub(crate) fn observed_listeners(_: &[Identity]) -> serde_json::Value {
        serde_json::json!({"incomplete":true,"error_code":"UNSUPPORTED_PLATFORM","ports":[]})
    }
    pub(crate) fn listener_addresses(
        &mut self,
        _: u16,
    ) -> crate::Result<Vec<std::net::SocketAddr>> {
        Err(unsupported())
    }
    pub fn capture(_: &mut std::process::Child) -> crate::Result<Self> {
        Err(unsupported())
    }
    pub fn refresh(&mut self) -> crate::Result<Vec<Identity>> {
        Err(unsupported())
    }
    pub fn stop(&mut self, _: std::time::Duration) -> crate::Result<bool> {
        Err(unsupported())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn port_owners(_: u16) -> serde_json::Value {
    serde_json::json!({"owners":[],"incomplete":true,"error_code":"UNSUPPORTED_PLATFORM"})
}
