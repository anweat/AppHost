use super::Stream;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Debug, Deserialize, Serialize)]
pub struct LogChunk {
    pub sequence: u64,
    pub observed_ms: u128,
    pub elapsed_ms: u128,
    pub service_key: String,
    pub stream: Stream,
    pub segment: u64,
    pub offset: u64,
    pub length: usize,
    #[serde(default)]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default, Serialize)]
pub struct LogWindow {
    pub chunks: Vec<LogChunk>,
    pub warnings: Vec<serde_json::Value>,
    pub prefix_evicted: bool,
    pub incomplete_tail: bool,
    pub truncated: bool,
    pub sequence_gap: bool,
    pub clock_reversed: bool,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn open_file(path: &Path) -> io::Result<File> {
    // A corrupt observation must never redirect a read outside its private log tree.
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(invalid("Expected a regular log file"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

/// Bounded observation-window read. Flags describe observed gaps, not a guarantee
/// that a future 10-second tail has elapsed or that the collector stayed alive.
/// A concurrent rotation may return NotFound; the caller must report that gap.
pub fn read_window(
    root: &Path,
    from_ms: u128,
    to_ms: u128,
    max_bytes: usize,
) -> io::Result<LogWindow> {
    read_bounded(root, from_ms, to_ms, max_bytes, false, false, None)
}

/// Keep the newest complete blocks for failure evidence under output floods.
pub fn read_tail(
    root: &Path,
    from_ms: u128,
    to_ms: u128,
    max_bytes: usize,
) -> io::Result<LogWindow> {
    read_bounded(root, from_ms, to_ms, max_bytes, true, false, None)
}

pub fn read_stderr_tail(
    root: &Path,
    from_ms: u128,
    to_ms: u128,
    max_bytes: usize,
    service_key: Option<&str>,
) -> io::Result<LogWindow> {
    read_bounded(root, from_ms, to_ms, max_bytes, true, true, service_key)
}

fn read_bounded(
    root: &Path,
    from_ms: u128,
    to_ms: u128,
    max_bytes: usize,
    tail: bool,
    stderr_only: bool,
    service_key: Option<&str>,
) -> io::Result<LogWindow> {
    if from_ms > to_ms || !(1..=1024 * 1024).contains(&max_bytes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid log window or byte limit",
        ));
    }
    let mut segments = Vec::new();
    let mut ignored = 0;
    for (count, entry) in fs::read_dir(root)?.enumerate() {
        if count >= 4096 {
            return Err(invalid("Too many log directory entries"));
        }
        let entry = entry?;
        let name = entry.file_name();
        let number = name
            .to_str()
            .and_then(|name| name.parse::<u64>().ok())
            .filter(|number| name == number.to_string().as_str());
        let Some(number) = number else {
            ignored += 1;
            continue;
        };
        // A valid segment name still requires a real directory and valid contents.
        if !entry.file_type()?.is_dir() {
            return Err(invalid("Invalid segment directory"));
        }
        segments.push(number);
    }
    if segments.len() > 100 {
        return Err(invalid("Too many log segments"));
    }
    segments.sort_unstable();
    let mut window = LogWindow {
        warnings: crate::storage::directory_warnings(root, ignored),
        ..LogWindow::default()
    };
    let mut read_bytes = 0;
    let mut scanned = 0usize;
    let mut previous: Option<(u64, u128)> = None;
    for segment in segments {
        let directory = root.join(segment.to_string());
        let mut reader = BufReader::new(open_file(&directory.join("index.jsonl"))?);
        loop {
            let mut line = Vec::new();
            let size = (&mut reader).take(1025).read_until(b'\n', &mut line)?;
            if size == 0 {
                break;
            }
            scanned += size;
            if size > 1024 {
                return Err(invalid("Oversized observation"));
            }
            if scanned > 16 * 1024 * 1024 {
                window.truncated = true;
                return Ok(window);
            }
            if line.last() != Some(&b'\n') {
                window.incomplete_tail = true;
                break;
            }
            let mut chunk: LogChunk = serde_json::from_slice(&line).map_err(io::Error::other)?;
            if chunk.segment != segment
                || chunk.length == 0
                || chunk.length > super::READ_CHUNK
                || chunk.service_key.is_empty()
                || chunk.service_key.len() > 64
                || !chunk
                    .service_key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || !chunk.bytes.is_empty()
            {
                return Err(invalid("Invalid log observation"));
            }
            if let Some((sequence, timestamp)) = previous {
                if sequence.checked_add(1) != Some(chunk.sequence) {
                    window.sequence_gap = true;
                }
                if chunk.observed_ms < timestamp {
                    window.clock_reversed = true;
                }
            } else if chunk.sequence > 0 && from_ms <= chunk.observed_ms {
                window.prefix_evicted = true;
            }
            previous = Some((chunk.sequence, chunk.observed_ms));
            if chunk.observed_ms < from_ms || chunk.observed_ms > to_ms {
                continue;
            }
            if stderr_only && chunk.stream != Stream::Stderr {
                continue;
            }
            if service_key.is_some_and(|key| key != chunk.service_key) {
                continue;
            }
            if read_bytes + chunk.length > max_bytes || window.chunks.len() >= 4096 {
                window.truncated = true;
                if !tail {
                    return Ok(window);
                }
                while !window.chunks.is_empty()
                    && (read_bytes + chunk.length > max_bytes || window.chunks.len() >= 4096)
                {
                    read_bytes -= window.chunks.remove(0).length;
                }
                if chunk.length > max_bytes {
                    continue;
                }
            }
            let suffix = match chunk.stream {
                Stream::Stdout => "out",
                Stream::Stderr => "err",
            };
            let mut file = open_file(&directory.join(format!("{}.{suffix}", chunk.service_key)))?;
            let end = chunk
                .offset
                .checked_add(chunk.length as u64)
                .ok_or_else(|| invalid("Invalid log offset"))?;
            if end > file.metadata()?.len() {
                return Err(invalid("Observation exceeds raw output"));
            }
            file.seek(SeekFrom::Start(chunk.offset))?;
            chunk.bytes.resize(chunk.length, 0);
            file.read_exact(&mut chunk.bytes)?;
            read_bytes += chunk.length;
            window.chunks.push(chunk);
        }
    }
    Ok(window)
}
