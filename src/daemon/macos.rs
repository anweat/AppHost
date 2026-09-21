use super::{protocol, unavailable};
use crate::{Error, Result, storage};
use fs2::FileExt;
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    mem,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const DEADLINE: Duration = Duration::from_secs(2);
const MAX_CLIENTS: usize = 16;

fn rpc_error(error: io::Error) -> Error {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        Error::new("RPC_TIMEOUT", "Daemon RPC exceeded its deadline")
    } else {
        unavailable()
    }
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| Error::new("RPC_TIMEOUT", "Daemon RPC exceeded its deadline"))
}

fn endpoint(data_dir: &Path) -> Result<PathBuf> {
    let path = data_dir.join("daemon.sock");
    if path.as_os_str().as_bytes().len() >= 104 {
        return Err(Error::new(
            "SOCKET_PATH_TOO_LONG",
            "Choose a shorter --data-dir; macOS socket paths must be under 104 bytes",
        ));
    }
    Ok(path)
}

fn verify_peer(stream: &UnixStream) -> Result<()> {
    let mut uid = 0;
    let mut gid = 0;
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if rc != 0 || uid != unsafe { libc::geteuid() } {
        return Err(Error::new(
            "PEER_REJECTED",
            "Daemon socket peer must be the current user",
        ));
    }
    Ok(())
}

// std's blocking connect has no deadline. Use a nonblocking socket for the
// handshake, then restore blocking IO with absolute read/write deadlines below.
fn connect(path: &Path) -> Result<UnixStream> {
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(unavailable());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        || unsafe { libc::fcntl(raw, libc::F_SETFL, libc::O_NONBLOCK) } < 0
    {
        return Err(unavailable());
    }
    let mut address: libc::sockaddr_un = unsafe { mem::zeroed() };
    address.sun_family = libc::AF_UNIX as _;
    address.sun_len = mem::size_of_val(&address) as _;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() {
        return Err(unavailable());
    }
    for (target, byte) in address.sun_path.iter_mut().zip(bytes) {
        *target = *byte as _;
    }
    let rc = unsafe {
        libc::connect(
            raw,
            &address as *const _ as *const libc::sockaddr,
            mem::size_of_val(&address) as _,
        )
    };
    if rc != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(unavailable().details(json!({"os_code":error.raw_os_error()})));
        }
        let deadline = Instant::now() + DEADLINE;
        loop {
            let mut poll = libc::pollfd {
                fd: raw,
                events: libc::POLLOUT,
                revents: 0,
            };
            let timeout = remaining(deadline)?.as_millis().max(1) as i32;
            let rc = unsafe { libc::poll(&mut poll, 1, timeout) };
            if rc < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if rc <= 0 {
                return Err(unavailable());
            }
            let mut error: libc::c_int = 0;
            let mut size = mem::size_of_val(&error) as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    raw,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    &mut error as *mut _ as *mut _,
                    &mut size,
                )
            } != 0
                || error != 0
            {
                return Err(unavailable());
            }
            break;
        }
    }
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false).map_err(rpc_error)?;
    verify_peer(&stream)?;
    Ok(stream)
}

fn read_frame(stream: &mut UnixStream, maximum: usize) -> Result<Vec<u8>> {
    let deadline = Instant::now() + DEADLINE;
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(rpc_error)?;
        let limit = buffer.len().min(maximum + 1 - bytes.len());
        let count = match stream.read(&mut buffer[..limit]) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(rpc_error(error)),
        };
        if count == 0 {
            return Err(if bytes.is_empty() {
                unavailable()
            } else {
                Error::new("PROTOCOL_ERROR", "Incomplete daemon frame")
            });
        }
        if let Some(end) = buffer[..count].iter().position(|byte| *byte == b'\n') {
            bytes.extend_from_slice(&buffer[..end]);
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > maximum {
            return Err(Error::new(
                "MESSAGE_TOO_LARGE",
                "Daemon frame exceeds its byte limit",
            ));
        }
    }
}

fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> Result<()> {
    let deadline = Instant::now() + DEADLINE;
    let mut remaining_bytes = bytes;
    while !remaining_bytes.is_empty() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(rpc_error)?;
        match stream.write(remaining_bytes) {
            Ok(0) => return Err(unavailable()),
            Ok(count) => remaining_bytes = &remaining_bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(rpc_error(error)),
        }
    }
    Ok(())
}

struct Endpoint {
    _lock: File,
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        // Never unlink a path that was replaced after we bound our own socket.
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
        let _ = FileExt::unlock(&self._lock);
    }
}

fn bind(data_dir: &Path) -> Result<Endpoint> {
    let path = endpoint(data_dir)?;
    let lock_path = data_dir.join("daemon.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&lock_path)
        .map_err(|e| storage::io_error(&lock_path, e))?;
    if !lock
        .metadata()
        .map_err(|e| storage::io_error(&lock_path, e))?
        .is_file()
    {
        return Err(Error::new("CONFLICT", "Daemon lock must be a regular file"));
    }
    FileExt::try_lock_exclusive(&lock).map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            Error::new(
                "DAEMON_ALREADY_RUNNING",
                "Another daemon owns this data directory",
            )
        } else {
            storage::io_error(&lock_path, error)
        }
    })?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err(Error::new(
                    "CONFLICT",
                    "Refusing to replace a non-owned socket path",
                ));
            }
            // Under our singleton lock, only ECONNREFUSED proves the old socket
            // stale. A live or inaccessible endpoint is preserved for inspection.
            match connect(&path) {
                Err(error) if error.details["os_code"] == libc::ECONNREFUSED => {}
                _ => {
                    return Err(Error::new(
                        "CONFLICT",
                        "Existing daemon socket is active or cannot be classified",
                    ));
                }
            }
            let current = fs::symlink_metadata(&path).map_err(|e| storage::io_error(&path, e))?;
            if (current.dev(), current.ino()) != (metadata.dev(), metadata.ino()) {
                return Err(Error::new(
                    "CONFLICT",
                    "Daemon socket changed during startup",
                ));
            }
            fs::remove_file(&path).map_err(|e| storage::io_error(&path, e))?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(storage::io_error(&path, error)),
    }
    let listener = UnixListener::bind(&path).map_err(|e| storage::io_error(&path, e))?;
    let metadata = fs::symlink_metadata(&path).map_err(|e| storage::io_error(&path, e))?;
    let endpoint = Endpoint {
        _lock: lock,
        listener,
        path,
        identity: (metadata.dev(), metadata.ino()),
    };
    fs::set_permissions(&endpoint.path, fs::Permissions::from_mode(0o600))
        .map_err(|e| storage::io_error(&endpoint.path, e))?;
    Ok(endpoint)
}

fn handle(mut stream: UnixStream, data_dir: &Path, hello: &Value) -> Result<()> {
    verify_peer(&stream)?;
    let request =
        read_frame(&mut stream, protocol::MAX_REQUEST).and_then(|bytes| protocol::decode(&bytes));
    let (id, result) = match &request {
        Ok(request) => (
            Some(request.request_id.as_str()),
            protocol::dispatch(data_dir, request, hello),
        ),
        Err(_) => (None, Err(request.err().unwrap())),
    };
    let response = protocol::response(id, result);
    let bytes = protocol::encode(&response, protocol::MAX_RESPONSE).or_else(|error| {
        protocol::encode(&protocol::response(id, Err(error)), protocol::MAX_RESPONSE)
    })?;
    write_frame(&mut stream, &bytes)
}

struct Slot(Arc<AtomicUsize>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) fn serve(data_dir: &Path) -> Result<()> {
    storage::private_directory(data_dir)?;
    let data_dir = Arc::new(storage::directory(data_dir)?);
    let endpoint = bind(&data_dir)?;
    let hello = Arc::new(
        json!({"protocol_version":protocol::VERSION,"daemon_id":Uuid::new_v4(),
        "pid":std::process::id(),"data_dir":*data_dir,"capabilities":["hello","check-launch"]}),
    );
    let clients = Arc::new(AtomicUsize::new(0));
    for stream in endpoint.listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(storage::io_error(&endpoint.path, error)),
        };
        if clients.load(Ordering::Relaxed) >= MAX_CLIENTS {
            continue;
        }
        clients.fetch_add(1, Ordering::Relaxed);
        let slot = Slot(Arc::clone(&clients));
        let data_dir = Arc::clone(&data_dir);
        let hello = Arc::clone(&hello);
        // Failed worker creation drops both stream and slot. No unbounded queue,
        // persistent idle worker pool, or client-owned cancellation state.
        let _ = thread::Builder::new()
            .name("apphost-rpc".into())
            .spawn(move || {
                let _slot = slot;
                let _ = handle(stream, &data_dir, &hello);
            });
    }
    Ok(())
}

fn client_root(data_dir: &Path) -> Result<PathBuf> {
    let data_dir = storage::directory(data_dir).map_err(|error| {
        if error.code == "NOT_FOUND" {
            unavailable()
        } else {
            error
        }
    })?;
    storage::check_private_directory(&data_dir)?;
    Ok(data_dir)
}

fn rpc(data_dir: &Path, method: &str, params: Value) -> Result<Value> {
    let path = endpoint(data_dir)?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| unavailable())?;
    if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(Error::new(
            "PEER_REJECTED",
            "Daemon endpoint is not an owned socket",
        ));
    }
    let mut stream = connect(&path)?;
    let id = Uuid::new_v4().to_string();
    let request =
        json!({"version":protocol::VERSION,"request_id":id,"method":method,"params":params});
    write_frame(
        &mut stream,
        &protocol::encode(&request, protocol::MAX_REQUEST)?,
    )?;
    protocol::result(&read_frame(&mut stream, protocol::MAX_RESPONSE)?, &id)
}

fn handshake(data_dir: &Path) -> Result<Value> {
    let hello = rpc(data_dir, "hello", json!({}))?;
    if hello["protocol_version"] != protocol::VERSION
        || hello["data_dir"].as_str() != data_dir.to_str()
        || hello["daemon_id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_none()
        || hello["pid"]
            .as_u64()
            .is_none_or(|pid| pid == 0 || pid > i32::MAX as u64)
        || !hello["capabilities"]
            .as_array()
            .is_some_and(|methods| methods.iter().any(|method| method == "hello"))
    {
        return Err(Error::new(
            "PROTOCOL_ERROR",
            "Daemon handshake does not match this data directory or protocol",
        ));
    }
    Ok(hello)
}

pub(super) fn ping(data_dir: &Path) -> Result<Value> {
    handshake(&client_root(data_dir)?)
}

pub(super) fn check_launch(data_dir: &Path, installation_id: Uuid) -> Result<Value> {
    let data_dir = client_root(data_dir)?;
    let hello = handshake(&data_dir)?;
    if !hello["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|method| method == "check-launch")
    {
        return Err(Error::new(
            "METHOD_NOT_FOUND",
            "Daemon does not advertise check-launch",
        ));
    }
    rpc(
        &data_dir,
        "check-launch",
        json!({"installation_id":installation_id}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_that_never_reads_cannot_hold_response_writer_forever() {
        let (mut writer, _reader) = UnixStream::pair().unwrap();
        verify_peer(&writer).unwrap();
        let size: libc::c_int = 4096;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    writer.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    &size as *const _ as *const _,
                    mem::size_of_val(&size) as _,
                )
            },
            0
        );
        let start = Instant::now();
        assert_eq!(
            write_frame(&mut writer, &vec![0; protocol::MAX_RESPONSE])
                .unwrap_err()
                .code,
            "RPC_TIMEOUT"
        );
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}
