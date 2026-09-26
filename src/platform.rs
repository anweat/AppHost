//! OS boundary. Shared lifecycle/RPC/gateway compile on every target. Windows
//! process execution and collector transport stay unsupported pending G3; this
//! module must never silently substitute ordinary children for managed jobs.
pub use crate::environment;
pub use crate::login;
pub use crate::process;
use crate::{Error, Result};
use serde_json::json;
use std::net::TcpListener;

#[cfg(target_os = "macos")]
#[path = "runtime/collector.rs"]
pub mod collector;
#[cfg(not(target_os = "macos"))]
#[path = "platform/collector_unsupported.rs"]
pub mod collector;

pub fn exit_evidence(status: std::process::ExitStatus) -> serde_json::Value {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    json!({"code":status.code(),"signal":signal})
}
pub(crate) fn port_available(port: u16) -> Result<()> {
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

/// Authenticated local framed transport. Implementations must enforce bounded
/// frame IO deadlines and same-user identity before dispatch; raw TCP is not an
/// implementation. Windows named-pipe authentication remains a G3 requirement.
pub trait LocalStream {
    fn authenticate(&self) -> Result<()>;
    fn read_frame(&mut self, maximum: usize) -> Result<Vec<u8>>;
    fn write_frame(&mut self, bytes: &[u8]) -> Result<()>;
}

/// The worker chooses readiness policy; the adapter supplies owned listener and
/// connection evidence. Keeping this narrow allows deterministic platform tests.
pub(crate) trait Readiness {
    fn startup_ready(&mut self, port: u16) -> Result<bool>;
    fn listening(&mut self, members: &[process::Identity], port: u16) -> Result<bool>;
}
impl Readiness for process::Group {
    fn startup_ready(&mut self, port: u16) -> Result<bool> {
        self.port_ready(port)
    }
    fn listening(&mut self, members: &[process::Identity], port: u16) -> Result<bool> {
        Self::port_listening(members, port)
    }
}
pub(crate) fn service_ready(
    group: &mut impl Readiness,
    members: &[process::Identity],
    port: Option<u16>,
    starting: bool,
    elapsed: std::time::Duration,
) -> Result<bool> {
    if members.is_empty() {
        return Ok(false);
    }
    match port {
        Some(port) if starting => group.startup_ready(port),
        Some(port) => group.listening(members, port),
        None => Ok(elapsed >= std::time::Duration::from_secs(2)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct FakePlatform {
        connects: usize,
        listens: usize,
        ready: bool,
    }
    impl Readiness for FakePlatform {
        fn startup_ready(&mut self, _: u16) -> Result<bool> {
            self.connects += 1;
            Ok(self.ready)
        }
        fn listening(&mut self, _: &[process::Identity], _: u16) -> Result<bool> {
            self.listens += 1;
            Ok(self.ready)
        }
    }
    #[test]
    fn runtime_observations_never_connect_and_dead_groups_never_probe() {
        use std::time::Duration;
        let mut platform = FakePlatform::default();
        let members = [process::Identity {
            pid: 100,
            group: 100,
            uid: 1,
            started_seconds: 1,
            started_microseconds: 0,
        }];
        let mut status = crate::runtime::InstanceStatus::default();
        assert!(!service_ready(&mut platform, &members, Some(1234), true, Duration::ZERO).unwrap());
        platform.ready = true;
        assert!(service_ready(&mut platform, &members, Some(1234), true, Duration::ZERO).unwrap());
        status.all_ready();
        platform.ready = false;
        assert!(
            !service_ready(&mut platform, &members, Some(1234), false, Duration::ZERO).unwrap()
        );
        platform.ready = true;
        assert!(service_ready(&mut platform, &members, Some(1234), false, Duration::ZERO).unwrap());
        assert_eq!((platform.connects, platform.listens), (2, 2));
        assert!(!service_ready(&mut platform, &[], Some(1234), false, Duration::ZERO).unwrap());
        assert_eq!((platform.connects, platform.listens), (2, 2));
        status.request_stop();
        assert!(!status.finish_stop(false));
        assert!(status.finish_stop(true));
    }
}
