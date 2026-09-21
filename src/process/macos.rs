use super::Identity;
use crate::{Error, Result};
use std::{
    io, mem,
    os::unix::process::CommandExt,
    process::{Child, Command},
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
    if info.pbi_status == libc::SZOMB {
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

#[derive(Debug)]
pub struct Group {
    group: u32,
    boot: (i64, i64),
    known: Vec<Identity>,
    exhausted: bool,
}

impl Group {
    /// Capture a child still owned by this supervisor; never adopt an arbitrary PID.
    pub fn capture(child: &mut Child) -> Result<Self> {
        if child.try_wait().map_err(system_error)?.is_some() {
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
