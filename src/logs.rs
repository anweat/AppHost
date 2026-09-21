//! Byte-preserving, bounded log segments. Observation order is not application causality.
use serde::{Deserialize, Serialize};

mod window;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
pub use window::{LogChunk, LogWindow, read_window};

pub const READ_CHUNK: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_success_then_index_failure_cannot_be_retried_as_complete_output() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("partial");
        let mut writer = LogWriter::create(root.clone(), 1024, 2).unwrap();
        // A read-only index descriptor forces failure after the raw bytes were
        // accepted, exercising the non-atomic boundary without a fake storage layer.
        writer.index = File::open(root.join("0/index.jsonl")).unwrap();
        assert!(writer.append("web", Stream::Stdout, b"unindexed").is_err());
        assert_eq!(fs::read(root.join("0/web.out")).unwrap(), b"unindexed");
        assert!(fs::read(root.join("0/index.jsonl")).unwrap().is_empty());
        writer.index = OpenOptions::new()
            .append(true)
            .open(root.join("0/index.jsonl"))
            .unwrap();
        assert!(writer.append("web", Stream::Stdout, b"retry").is_err());
        assert_eq!(fs::read(root.join("0/web.out")).unwrap(), b"unindexed");
    }
}

#[derive(Serialize)]
struct Observation<'a> {
    sequence: u64,
    observed_ms: u128,
    elapsed_ms: u128,
    service_key: &'a str,
    stream: Stream,
    segment: u64,
    offset: u64,
    length: usize,
}

pub struct LogWriter {
    root: PathBuf,
    segment_limit: usize,
    retained_segments: u64,
    segment: u64,
    used: usize,
    sequence: u64,
    started: Instant,
    index: File,
    files: BTreeMap<(String, Stream), (File, u64)>,
    failed: bool,
}

impl LogWriter {
    /// Requires a new private instance directory; never reopens an unknown writer's files.
    /// The budget includes the index so one-byte writes cannot grow metadata without bound.
    pub fn create(root: PathBuf, segment_limit: usize, retained_segments: u64) -> io::Result<Self> {
        if !(1024..=16 * 1024 * 1024).contains(&segment_limit)
            || !(1..=100).contains(&retained_segments)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid segment budget",
            ));
        }
        fs::create_dir(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        let directory = root.join("0");
        fs::create_dir(&directory)?;
        let index = File::create(directory.join("index.jsonl"))?;
        Ok(Self {
            root,
            segment_limit,
            retained_segments,
            segment: 0,
            used: 0,
            sequence: 0,
            started: Instant::now(),
            index,
            files: BTreeMap::new(),
            failed: false,
        })
    }

    pub fn append(&mut self, key: &str, stream: Stream, bytes: &[u8]) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other(
                "Log writer closed after an earlier IO failure",
            ));
        }
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid service key",
            ));
        }
        // Bound both the input chunk and open streams; service names never become paths.
        if self.files.len() >= 64 && !self.files.contains_key(&(key.to_owned(), stream)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Too many log streams",
            ));
        }
        for chunk in bytes.chunks(READ_CHUNK.min(self.segment_limit / 2)) {
            if let Err(error) = self.write_chunk(key, stream, chunk) {
                // A partial raw/index write cannot be retried as if it were atomic.
                // End this collector; the supervisor records the gap and retains services.
                self.failed = true;
                return Err(error);
            }
        }
        Ok(())
    }

    fn write_chunk(&mut self, key: &str, stream: Stream, chunk: &[u8]) -> io::Result<()> {
        let file_key = (key.to_owned(), stream);
        let offset = self.files.get(&file_key).map_or(0, |(_, offset)| *offset);
        let mut observation = Observation {
            sequence: self.sequence,
            observed_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_millis(),
            elapsed_ms: self.started.elapsed().as_millis(),
            service_key: key,
            stream,
            segment: self.segment,
            offset,
            length: chunk.len(),
        };
        let mut index = serde_json::to_vec(&observation)?;
        if self.used + chunk.len() + index.len() + 1 > self.segment_limit {
            self.rotate()?;
            observation.segment = self.segment;
            observation.offset = 0;
            index = serde_json::to_vec(&observation)?;
        }
        if !self.files.contains_key(&file_key) {
            let suffix = match stream {
                Stream::Stdout => "out",
                Stream::Stderr => "err",
            };
            let file = OpenOptions::new().write(true).create_new(true).open(
                self.root
                    .join(self.segment.to_string())
                    .join(format!("{key}.{suffix}")),
            )?;
            self.files.insert(file_key.clone(), (file, 0));
        }
        let (file, offset) = self.files.get_mut(&file_key).unwrap();
        // Publish the index after raw bytes. IO failure remains explicit rather
        // than acknowledging an observation whose bytes were never stored.
        file.write_all(chunk)?;
        index.push(b'\n');
        self.index.write_all(&index)?;
        *offset += chunk.len() as u64;
        self.used += chunk.len() + index.len();
        self.sequence += 1;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.files.clear();
        self.segment += 1;
        let directory = self.root.join(self.segment.to_string());
        fs::create_dir(&directory)?;
        self.index = File::create(directory.join("index.jsonl"))?;
        self.used = 0;
        if self.segment >= self.retained_segments {
            fs::remove_dir_all(
                self.root
                    .join((self.segment - self.retained_segments).to_string()),
            )?;
        }
        Ok(())
    }
}
