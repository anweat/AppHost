//! One independent reader per instance. It owns only log files and descriptors;
//! losing the daemon never closes the services' pipes.
use crate::{
    Error, Result,
    logs::{LogWriter, READ_CHUNK, Stream},
    process, storage,
};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, Stdio},
};

pub struct Pipes {
    pub child: Child,
    pub outputs: Vec<(Stdio, Stdio)>,
}
pub fn spawn(root: &Path, count: usize) -> Result<Pipes> {
    if !(1..=32).contains(&count) {
        return Err(Error::new(
            "RUNTIME_LIMIT",
            "At most 32 services per instance",
        ));
    }
    let mut readers = Vec::new();
    let mut outputs = Vec::new();
    for _ in 0..count {
        let mut writes = Vec::new();
        for _ in 0..2 {
            let (read, write) = UnixStream::pair().map_err(|e| storage::io_error(root, e))?;
            // Allocate before Command's internal exec-error pipe exists. The
            // child inherits these exact descriptors, never dup2s over that pipe.
            let fd = unsafe { libc::fcntl(read.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 128) };
            if fd < 0 {
                return Err(storage::io_error(root, io::Error::last_os_error()));
            }
            readers.push(unsafe { OwnedFd::from_raw_fd(fd) });
            writes.push(Stdio::from(OwnedFd::from(write)));
        }
        let err = writes.pop().unwrap();
        let out = writes.pop().unwrap();
        outputs.push((out, err));
    }
    let mut command =
        Command::new(std::env::current_exe().map_err(|e| storage::io_error(root, e))?);
    let descriptors = readers
        .iter()
        .map(|fd| fd.as_raw_fd().to_string())
        .collect::<Vec<_>>()
        .join(",");
    command
        .arg("internal-collect")
        .arg(root)
        .arg(descriptors)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            for reader in &readers {
                if libc::fcntl(reader.as_raw_fd(), libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    Ok(Pipes {
        child: process::spawn_group(&mut command)?,
        outputs,
    })
}
pub fn ready(child: &mut Child) -> Result<()> {
    let mut output = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("LOG_COLLECTION_FAILED", "Missing collector acknowledgement"))?;
    let mut poll = libc::pollfd {
        fd: output.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    if unsafe { libc::poll(&mut poll, 1, 2000) } <= 0 {
        return Err(Error::new(
            "LOG_COLLECTION_FAILED",
            "Collector initialization timed out",
        ));
    }
    let mut byte = [0];
    if output.read(&mut byte).ok() != Some(1) || byte[0] != b'R' {
        return Err(Error::new(
            "LOG_COLLECTION_FAILED",
            "Collector initialization failed",
        ));
    }
    Ok(())
}
pub fn run(root: &Path, descriptors: &str) -> Result<()> {
    if descriptors.len() > 1024 {
        return Err(Error::new(
            "INVALID_INPUT",
            "Too many collector descriptors",
        ));
    }
    let fds: Vec<i32> = descriptors
        .split(',')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| Error::new("INVALID_INPUT", "Invalid collector descriptor"))?;
    if fds.is_empty()
        || fds.len() > 64
        || !fds.len().is_multiple_of(2)
        || fds.iter().any(|fd| *fd < 3)
        || fds.iter().collect::<std::collections::BTreeSet<_>>().len() != fds.len()
    {
        return Err(Error::new(
            "INVALID_INPUT",
            "Invalid collector stream count or duplicate descriptors",
        ));
    }
    // Only the hidden child entry accepts this list, never the management RPC.
    // Ownership transfers once per distinct inherited descriptor.
    let mut inputs: Vec<File> = fds
        .into_iter()
        .map(|fd| unsafe { File::from_raw_fd(fd) })
        .collect();
    for file in &inputs {
        let mut kind = 0i32;
        let mut size = std::mem::size_of_val(&kind) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                file.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                &mut kind as *mut _ as *mut _,
                &mut size,
            )
        } != 0
            || kind != libc::SOCK_STREAM
        {
            return Err(Error::new(
                "INVALID_INPUT",
                "Collector input is not a stream socket",
            ));
        }
    }
    let mut writer = LogWriter::create(root.to_path_buf(), 1024 * 1024, 8)
        .map_err(|e| storage::io_error(root, e))?;
    io::stdout()
        .write_all(b"R\n")
        .map_err(|e| storage::io_error(root, e))?;
    let mut polls: Vec<_> = inputs
        .iter()
        .map(|f| libc::pollfd {
            fd: f.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let mut buffer = [0; READ_CHUNK];
    while polls.iter().any(|p| p.fd >= 0) {
        if unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as _, -1) } < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(storage::io_error(root, e));
        }
        for i in 0..inputs.len() {
            if polls[i].fd < 0 || polls[i].revents == 0 {
                continue;
            }
            match inputs[i].read(&mut buffer) {
                Ok(0) => polls[i].fd = -1,
                Ok(n) => writer
                    .append(
                        &format!("s{}", i / 2),
                        if i % 2 == 0 {
                            Stream::Stdout
                        } else {
                            Stream::Stderr
                        },
                        &buffer[..n],
                    )
                    .map_err(|e| storage::io_error(root, e))?,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(storage::io_error(root, e)),
            }
        }
    }
    drop(writer);
    storage::publish(&root.with_extension("complete"), b"complete", false)
}
