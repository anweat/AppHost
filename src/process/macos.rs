use super::Identity;
use crate::{Error, Result};
use std::{
    io, mem,
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

fn system_error(error: io::Error) -> Error {
    Error::new("PROCESS_ERROR", error.to_string())
}

fn uncertain(message: &str) -> Error {
    Error::new("MANAGEMENT_UNCERTAIN", message)
}

fn identity(pid: u32) -> Result<Option<Identity>> {
    identity_inner(pid, false)
}

fn identity_inner(pid: u32, include_zombie: bool) -> Result<Option<Identity>> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err(uncertain("Invalid process ID"));
    }
    let mut info: libc::proc_bsdinfo = unsafe { mem::zeroed() };
    let size = mem::size_of_val(&info) as i32;
    // libproc writes exactly PROC_PIDTBSDINFO_SIZE bytes to this initialized struct.
    let count = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            size,
        )
    };
    if count <= 0 {
        let error = io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ENOENT)) {
            return Ok(None);
        }
        return Err(system_error(error));
    }
    if count != size || info.pbi_pid != pid {
        return Err(uncertain("Incomplete process identity"));
    }
    if info.pbi_status == libc::SZOMB && !include_zombie {
        return Ok(None);
    }
    Ok(Some(Identity {
        pid,
        group: info.pbi_pgid,
        uid: info.pbi_uid,
        started_seconds: info.pbi_start_tvsec,
        started_microseconds: info.pbi_start_tvusec,
    }))
}

fn boot_time() -> Result<(i64, i64)> {
    let mut time: libc::timeval = unsafe { mem::zeroed() };
    let mut size = mem::size_of_val(&time);
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.boottime".as_ptr(),
            &mut time as *mut _ as *mut _,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return Err(system_error(io::Error::last_os_error()));
    }
    if size != mem::size_of_val(&time) {
        return Err(uncertain("Incomplete system boot identity"));
    }
    Ok((time.tv_sec, time.tv_usec as i64))
}

pub fn spawn_group(command: &mut Command) -> Result<Child> {
    command.process_group(0).spawn().map_err(system_error)
}

/// Observe without reaping: the unreaped leader pins its PID/PGID until the
/// supervisor proves the whole group empty and explicitly calls Child::wait.
pub fn observe_exit(child: &Child) -> Result<Option<ExitStatus>> {
    let mut info: libc::siginfo_t = unsafe { mem::zeroed() };
    loop {
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if rc == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ECHILD) {
            return Err(uncertain(
                "Child was already reaped or is not owned by this supervisor",
            ));
        }
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(system_error(error));
        }
    }
    if info.si_pid == 0 {
        return Ok(None);
    }
    let raw = if info.si_code == libc::CLD_EXITED {
        info.si_status << 8
    } else {
        info.si_status
            | if info.si_code == libc::CLD_DUMPED {
                0x80
            } else {
                0
            }
    };
    Ok(Some(ExitStatus::from_raw(raw)))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    group: u32,
    boot: (i64, i64),
    known: Vec<Identity>,
    exhausted: bool,
}

impl Group {
    /// Capture a child still owned by this supervisor; never adopt an arbitrary PID.
    pub fn capture(child: &mut Child) -> Result<Self> {
        if observe_exit(child)?.is_some() {
            return Err(uncertain("Leader exited before its identity was captured"));
        }
        let leader =
            identity(child.id())?.ok_or_else(|| uncertain("Leader disappeared before capture"))?;
        if leader.group != child.id() || leader.uid != unsafe { libc::geteuid() } {
            return Err(uncertain(
                "Child is not the leader of an owned independent group",
            ));
        }
        Ok(Self {
            group: leader.group,
            boot: boot_time()?,
            known: vec![leader],
            exhausted: false,
        })
    }

    /// A freshly spawned, unreaped leader pins its process-group ID even when
    /// exec exits immediately. Capture its OS identity before classifying exit.
    pub fn capture_spawned(child: &Child) -> Result<Self> {
        let _ = observe_exit(child)?; // ECHILD rejects an already-reaped handle.
        let leader = identity_inner(child.id(), true)?
            .ok_or_else(|| uncertain("Spawned leader disappeared"))?;
        if leader.group != child.id() || leader.uid != unsafe { libc::geteuid() } {
            return Err(uncertain("Invalid spawned group"));
        }
        Ok(Self {
            group: leader.group,
            boot: boot_time()?,
            known: vec![leader],
            exhausted: false,
        })
    }

    pub fn id(&self) -> u32 {
        self.group
    }

    pub fn refresh(&mut self) -> Result<Vec<Identity>> {
        if self.exhausted {
            return Ok(vec![]);
        }
        if boot_time()? != self.boot {
            return Err(uncertain("System boot identity changed"));
        }
        let mut anchors = Vec::new();
        for known in &self.known {
            if let Some(current) = identity(known.pid)? {
                if current == *known {
                    anchors.push(current);
                } else if current.started_seconds == known.started_seconds
                    && current.started_microseconds == known.started_microseconds
                {
                    return Err(uncertain("A known process changed its group or user"));
                }
            }
        }
        let members = members(self.group)?;
        if members.is_empty() {
            // Check again: a changing enumeration must not hide a live known member.
            for anchor in anchors {
                if identity(anchor.pid)?.as_ref() == Some(&anchor) {
                    return Err(uncertain("Group enumeration omitted a live known member"));
                }
            }
            self.exhausted = true;
            self.known.clear();
            return Ok(members);
        }
        if !members.iter().any(|member| anchors.contains(member)) {
            return Err(uncertain("No live known member proves group ownership"));
        }
        if members
            .iter()
            .any(|member| member.uid != unsafe { libc::geteuid() })
        {
            return Err(uncertain("Group contains a different user's process"));
        }
        self.known = members.clone();
        Ok(members)
    }

    fn signal(&mut self, signal: i32) -> Result<()> {
        if self.group <= 1 || self.group == unsafe { libc::getpgrp() } as u32 {
            return Err(uncertain("Refusing to signal the supervisor's group"));
        }
        if self.refresh()?.is_empty() {
            return Ok(());
        }
        // macOS has no atomic compare-identity-and-kill API. Revalidate immediately
        // before signalling; never reuse this handle once its group became empty.
        if unsafe { libc::kill(-(self.group as i32), signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(system_error(error));
            }
        }
        Ok(())
    }

    /// Run on a worker, not the daemon event loop. One TERM, at most one KILL.
    /// Returns whether escalation was needed; success always means no live members.
    pub fn stop(&mut self, grace: Duration) -> Result<bool> {
        if grace.is_zero() || grace > Duration::from_secs(60) {
            return Err(Error::new(
                "INVALID_INPUT",
                "Stop grace must be in (0, 60s]",
            ));
        }
        self.signal(libc::SIGTERM)?;
        if self.wait_empty(grace)? {
            return Ok(false);
        }
        self.signal(libc::SIGKILL)?;
        if self.wait_empty(Duration::from_secs(2))? {
            return Ok(true);
        }
        Err(Error::new(
            "STOP_FAILED",
            "Group still has live members after SIGKILL",
        ))
    }

    fn wait_empty(&mut self, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.refresh()?.is_empty() {
                return Ok(true);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            thread::sleep((deadline - now).min(Duration::from_millis(20)));
        }
    }
}

fn members(group: u32) -> Result<Vec<Identity>> {
    // PROC_PGRP_ONLY is defined by the macOS SDK. A full buffer may be truncated;
    // grow with a hard bound rather than claiming that omitted members exited.
    let mut pids = vec![0i32; 64];
    loop {
        let bytes = (pids.len() * mem::size_of::<i32>()) as i32;
        let count = unsafe { libc::proc_listpids(2, group, pids.as_mut_ptr() as *mut _, bytes) };
        if count < 0 {
            return Err(system_error(io::Error::last_os_error()));
        }
        if count >= bytes {
            if pids.len() >= 65536 {
                return Err(uncertain("Process group exceeds enumeration limit"));
            }
            pids.resize(pids.len() * 2, 0);
            continue;
        }
        if count % mem::size_of::<i32>() as i32 != 0 {
            return Err(uncertain("Incomplete PID list"));
        }
        let mut result = Vec::new();
        for pid in &pids[..count as usize / mem::size_of::<i32>()] {
            if *pid <= 0 {
                continue;
            }
            if let Some(info) = identity(*pid as u32)? {
                if info.group != group {
                    return Err(uncertain("Process changed groups during enumeration"));
                }
                result.push(info);
            }
        }
        return Ok(result);
    }
}

impl Group {
    /// Boot changes prove the old processes gone. On the same boot, a saved
    /// identity must still anchor a nonempty group before it can be managed.
    pub fn reconcile(&mut self) -> Result<Vec<Identity>> {
        if self.group <= 1
            || self.group > i32::MAX as u32
            || self.group == unsafe { libc::getpgrp() } as u32
            || self.known.len() > 65536
            || self
                .known
                .iter()
                .any(|p| p.group != self.group || p.uid != unsafe { libc::geteuid() })
        {
            return Err(uncertain("Invalid saved process group"));
        }
        if boot_time()? != self.boot {
            self.known.clear();
            self.exhausted = true;
            return Ok(Vec::new());
        }
        self.refresh()
    }

    /// Passive running observation using the identities just refreshed by the worker.
    pub(crate) fn port_listening(members: &[Identity], port: u16) -> Result<bool> {
        Ok(owns_listener(members, port, libc::AF_INET)?
            || owns_listener(members, port, libc::AF_INET6)?)
    }

    pub fn port_ready(&mut self, port: u16) -> Result<bool> {
        use std::net::{SocketAddr, TcpStream};
        let members = self.refresh()?;
        for (host, family) in [("127.0.0.1", libc::AF_INET), ("::1", libc::AF_INET6)] {
            let address: SocketAddr = format!(
                "{}:{port}",
                if family == libc::AF_INET6 {
                    "[::1]"
                } else {
                    host
                }
            )
            .parse()
            .unwrap();
            // Connect only to the declared loopback port; never search for an entry.
            if TcpStream::connect_timeout(&address, Duration::from_millis(30)).is_err() {
                continue;
            }
            if owns_listener(&members, port, family)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn observed_listeners(members: &[Identity]) -> serde_json::Value {
        unsafe extern "C" {
            fn apphost_pid_listener_ports(
                pid: i32,
                ports: *mut u16,
                capacity: i32,
                partial: *mut i32,
            ) -> i32;
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut ports = std::collections::BTreeSet::new();
        let mut incomplete = members.len() > 128;
        for member in members.iter().take(128) {
            if Instant::now() >= deadline {
                incomplete = true;
                break;
            }
            let mut buffer = [0u16; 128];
            let mut partial = 0;
            let count = unsafe {
                apphost_pid_listener_ports(
                    member.pid as i32,
                    buffer.as_mut_ptr(),
                    128,
                    &mut partial,
                )
            };
            // Discard evidence when PID/start-time identity no longer matches.
            if count < 0
                || !identity(member.pid).is_ok_and(|current| current.as_ref() == Some(member))
            {
                incomplete = true;
                continue;
            }
            incomplete |= partial != 0;
            ports.extend(buffer.into_iter().take(count as usize));
            if ports.len() > 128 {
                incomplete = true;
                break;
            }
        }
        serde_json::json!({"ports":ports.into_iter().take(128).collect::<Vec<_>>(),"incomplete":incomplete,
            "scope":"owned_loopback_tcp","observed_at_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()})
    }

    pub(crate) fn listener_addresses(&mut self, port: u16) -> Result<Vec<std::net::SocketAddr>> {
        let members = self.refresh()?;
        let mut addresses = Vec::new();
        for (address, family) in [
            (std::net::Ipv4Addr::LOCALHOST.into(), libc::AF_INET),
            (std::net::Ipv6Addr::LOCALHOST.into(), libc::AF_INET6),
        ] {
            if owns_listener(&members, port, family)? {
                addresses.push(std::net::SocketAddr::new(address, port));
            }
        }
        if addresses.is_empty() {
            return Err(Error::new(
                "UPSTREAM_NOT_READY",
                "No owned listener is available",
            ));
        }
        Ok(addresses)
    }
}

fn owns_listener(members: &[Identity], port: u16, family: i32) -> Result<bool> {
    unsafe extern "C" {
        fn apphost_pid_listens(pid: i32, port: u16, family: i32) -> i32;
    }
    for member in members {
        let found = unsafe { apphost_pid_listens(member.pid as i32, port, family) };
        if found < 0 {
            return Err(uncertain("Cannot inspect listening socket owner"));
        }
        if found == 1 && identity(member.pid)?.as_ref() == Some(member) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Find one representative owner for display, never authority to signal.
/// `incomplete` describes an unsuccessful lookup, not an exhaustive owner list.
pub fn port_owners(port: u16) -> serde_json::Value {
    let end = Instant::now() + Duration::from_millis(250);
    let mut pids = vec![0i32; 65536];
    let count = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            (pids.len() * mem::size_of::<i32>()) as i32,
        )
    };
    if count <= 0 {
        return serde_json::json!({"scope":"first_match","found":false,"owners":[],"incomplete":true,
            "limitations":{"pid_list_failed":true,"permission_denied":0,"unavailable":0,"timed_out":false,"pid_limit":false}});
    }
    let limited = count as usize >= pids.len();
    pids.truncate((count as usize).min(pids.len()));
    lookup_port_owner(port, pids, end, limited)
}
fn lookup_port_owner(
    port: u16,
    mut pids: Vec<i32>,
    end: Instant,
    limited: bool,
) -> serde_json::Value {
    unsafe extern "C" {
        fn apphost_pid_listens(pid: i32, port: u16, family: i32) -> i32;
    }
    // Prefer newer PID values as a search heuristic only, never as identity proof.
    pids.sort_unstable_by(|a, b| b.cmp(a));
    let mut owners = Vec::new();
    let (mut permission_denied, mut unavailable, mut timed_out) = (0, 0, false);
    for pid in pids {
        if Instant::now() >= end {
            timed_out = true;
            break;
        }
        if pid <= 0 {
            continue;
        }
        match unsafe { apphost_pid_listens(pid, port, 0) } {
            1 => {
                let mut name = [0u8; 256];
                let n =
                    unsafe { libc::proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
                owners.push(serde_json::json!({"pid":pid,"name":if n>0 {Some(String::from_utf8_lossy(&name[..(n as usize).min(name.len())]).trim_end_matches('\0').to_string())}else{None}}));
                break;
            }
            -2 => permission_denied += 1,
            -1 => unavailable += 1,
            _ => {}
        }
    }
    let found = !owners.is_empty();
    serde_json::json!({"scope":"first_match","found":found,"owners":owners,
        "incomplete":!found && (timed_out || limited || permission_denied > 0 || unavailable > 0),
        "limitations":{"pid_list_failed":false,"permission_denied":permission_denied,"unavailable":unavailable,"timed_out":timed_out,"pid_limit":limited}})
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        child: Child,
        group: Group,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[test]
    fn changed_start_time_boot_and_uid_never_authorize_a_signal() {
        let mut child = spawn_group(Command::new("/bin/sleep").arg("10")).unwrap();
        let group = match Group::capture(&mut child) {
            Ok(group) => group,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{error}");
            }
        };
        let mut fixture = Fixture { child, group };
        let original = fixture.group.known[0].clone();
        // Simulate persisted PID reuse / stale boot evidence, without trying to
        // force the OS to recycle a PID or ever signalling a foreign process.
        fixture.group.known[0].started_microseconds += 1;
        assert_eq!(
            fixture
                .group
                .stop(Duration::from_millis(10))
                .unwrap_err()
                .code,
            "MANAGEMENT_UNCERTAIN"
        );
        assert!(fixture.child.try_wait().unwrap().is_none());
        fixture.group.known[0] = original.clone();
        fixture.group.boot.0 -= 1;
        assert_eq!(
            fixture
                .group
                .stop(Duration::from_millis(10))
                .unwrap_err()
                .code,
            "MANAGEMENT_UNCERTAIN"
        );
        assert!(fixture.child.try_wait().unwrap().is_none());
        fixture.group.boot = boot_time().unwrap();
        fixture.group.known[0].uid = original.uid.wrapping_add(1);
        assert_eq!(
            fixture
                .group
                .stop(Duration::from_millis(10))
                .unwrap_err()
                .code,
            "MANAGEMENT_UNCERTAIN"
        );
        assert!(fixture.child.try_wait().unwrap().is_none());
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;
    #[test]
    fn expired_owner_search_is_not_reported_as_a_complete_absence() {
        let result = lookup_port_owner(
            12345,
            vec![std::process::id() as i32],
            Instant::now(),
            false,
        );
        assert_eq!(result["found"], false);
        assert_eq!(result["incomplete"], true);
        assert_eq!(result["limitations"]["timed_out"], true);
        let empty = lookup_port_owner(
            12345,
            vec![],
            Instant::now() + Duration::from_secs(1),
            false,
        );
        assert_eq!(empty["found"], false);
        assert_eq!(empty["incomplete"], false);
    }
}
