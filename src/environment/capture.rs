use super::{Baseline, CaptureRecord, FILE_NAME, load, save, user_environment};
use crate::{
    Error, Result,
    process::{Group, spawn_group},
    storage,
};
use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsRawFd, OwnedFd},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn failed(message: impl Into<String>) -> Error {
    Error::new("ENV_CAPTURE_FAILED", message)
}

struct Session {
    child: Child,
    group: Option<Group>,
    cleanup_attempted: bool,
}
impl Session {
    fn cleanup(&mut self) -> Result<()> {
        self.cleanup_attempted = true;
        if let Some(group) = &mut self.group {
            group.stop(Duration::from_millis(100))?;
        } else if self
            .child
            .try_wait()
            .map_err(|e| failed(e.to_string()))?
            .is_none()
        {
            // Capture failed before a group identity was available. Only the fresh
            // Child handle is ours; no arbitrary/orphan PID is adopted for cleanup.
            self.child.kill().map_err(|e| failed(e.to_string()))?;
        }
        self.child.wait().map_err(|e| failed(e.to_string()))?;
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if !self.cleanup_attempted {
            let _ = self.cleanup();
        }
    }
}

pub(super) fn capture_path(data_dir: &Path, shell: &Path) -> Result<Baseline> {
    if !shell.is_absolute()
        || shell
            .to_str()
            .is_none_or(|path| path.len() > 4096 || path.chars().any(char::is_control))
        || !std::fs::metadata(shell).is_ok_and(|metadata| metadata.is_file())
        || !super::macos::can_execute(shell)
    {
        return Err(failed("Select an absolute executable shell path"));
    }
    let expected = storage::fingerprint(&data_dir.join(FILE_NAME))?;
    let baseline = load(data_dir)?;
    let mut environment = user_environment()?;
    environment.insert("PATH".into(), baseline.path);
    let marker = format!("APPHOST_PATH_{}", uuid::Uuid::new_v4().simple());
    let script = format!("command printf '\\000{marker}\\000%s\\000' \"$PATH\"");
    let child = spawn_group(
        Command::new(shell)
            .args(["-l", "-c", &script])
            .env_clear()
            .envs(environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    let mut session = Session {
        child,
        group: None,
        cleanup_attempted: false,
    };
    session.group = Some(Group::capture(&mut session.child)?);
    let output = collect(&mut session);
    // Cleanup also runs on successful shell exit: background children must not
    // silently become a persistent environment-management service.
    session.cleanup()?;
    let output = output?;
    let marker = format!("\0{marker}\0").into_bytes();
    let matches: Vec<_> = output
        .windows(marker.len())
        .enumerate()
        .filter(|(_, bytes)| *bytes == marker)
        .map(|(index, _)| index)
        .collect();
    if matches.len() != 1 {
        return Err(failed("Shell did not emit one valid PATH marker"));
    }
    let rest = &output[matches[0] + marker.len()..];
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| failed("Incomplete PATH capture"))?;
    let path = String::from_utf8(rest[..end].to_vec())
        .map_err(|_| failed("Captured PATH must be UTF-8"))?;
    let captured_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| failed("Invalid system time"))?
        .as_millis() as u64;
    let baseline = Baseline {
        schema_version: 1,
        path,
        capture: Some(CaptureRecord {
            shell: shell.to_owned(),
            captured_at_ms,
        }),
    };
    save(data_dir, baseline, Some(expected))
}

fn collect(session: &mut Session) -> Result<Vec<u8>> {
    let mut files = [
        File::from(OwnedFd::from(session.child.stdout.take().unwrap())),
        File::from(OwnedFd::from(session.child.stderr.take().unwrap())),
    ];
    let mut polls = files.each_ref().map(|file| libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    let mut total = 0;
    let mut buffer = [0; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(Error::new(
                "ENV_CAPTURE_TIMEOUT",
                "Login shell exceeded the five-second capture deadline",
            ));
        }
        session.group.as_mut().unwrap().refresh()?;
        let exit = session
            .child
            .try_wait()
            .map_err(|e| failed(e.to_string()))?;
        if let Some(exit) = exit
            && polls.iter().all(|poll| poll.fd < 0)
        {
            return if exit.success() {
                Ok(output)
            } else {
                Err(failed(
                    "Login shell exited unsuccessfully; configuration was preserved",
                ))
            };
        }
        let rc = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as _, 20) };
        if rc < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(failed(error.to_string()));
        }
        for index in 0..files.len() {
            if polls[index].fd < 0 || polls[index].revents == 0 {
                continue;
            }
            match files[index].read(&mut buffer) {
                Ok(0) => polls[index].fd = -1,
                Ok(count) => {
                    total += count;
                    if total > 65536 {
                        return Err(Error::new(
                            "ENV_CAPTURE_LIMIT",
                            "Login shell output exceeded 64 KiB",
                        ));
                    }
                    if index == 0 {
                        output.extend_from_slice(&buffer[..count]);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(failed(error.to_string())),
            }
        }
    }
}
