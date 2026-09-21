//! Platform process-group primitives. Trust checks and runtime persistence belong to L1.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
pub struct Group;

#[cfg(not(target_os = "macos"))]
impl Group {
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
