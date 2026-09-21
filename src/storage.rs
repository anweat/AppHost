use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

pub const MAX_BYTES: u64 = 1024 * 1024;
pub const MAX_FINGERPRINT_BYTES: u64 = 16 * 1024 * 1024;

pub fn io_error(path: &Path, error: std::io::Error) -> Error {
    let code = if error.kind() == std::io::ErrorKind::NotFound {
        "NOT_FOUND"
    } else {
        "IO_ERROR"
    };
    Error::new(code, format!("{}: {error}", path.display()))
}

/// Create private AppHost storage without changing permissions on user directories.
pub(crate) fn private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| io_error(path, e))?;
    check_private_directory(path)
}

pub(crate) fn check_private_directory(path: &Path) -> Result<()> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).map_err(|e| io_error(path, e))?;
        if metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(Error::new(
                "DATA_DIR_PERMISSIONS",
                format!(
                    "Data directory {} must be private and owned by the current user; choose a private --data-dir. Existing permissions were preserved",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Vec<u8>> {
    let mut remaining = u64::MAX;
    read_budgeted(path, &mut remaining)
}

pub(crate) fn read_budgeted(path: &Path, remaining: &mut u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path).map_err(|e| io_error(path, e))?;
    if !meta.is_file() {
        return Err(Error::new(
            "INVALID_INPUT",
            format!("Expected a regular file: {}", path.display()),
        ));
    }
    if meta.len() > MAX_BYTES {
        return Err(Error::new(
            "INVALID_INPUT",
            format!("File exceeds 1 MiB: {}", path.display()),
        ));
    }
    if meta.len() > *remaining {
        return Err(Error::new("SCAN_LIMIT", "Metadata read budget exhausted"));
    }
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES.min(*remaining) + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(path, e))?;
    if bytes.len() > MAX_BYTES as usize {
        return Err(Error::new(
            "INVALID_INPUT",
            format!("File exceeds 1 MiB: {}", path.display()),
        ));
    }
    if bytes.len() as u64 > *remaining {
        return Err(Error::new("SCAN_LIMIT", "Metadata read budget exhausted"));
    }
    *remaining -= bytes.len() as u64;
    Ok(bytes)
}

pub fn fingerprint(path: &Path) -> Result<Option<String>> {
    let mut remaining = u64::MAX;
    fingerprint_budgeted(path, &mut remaining)
}

pub(crate) fn fingerprint_budgeted(path: &Path, remaining: &mut u64) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(path, e)),
        Ok(meta) => {
            if !meta.is_file() {
                return Err(Error::new(
                    "INVALID_INPUT",
                    format!("Expected a regular file: {}", path.display()),
                ));
            }
            if meta.len() > MAX_FINGERPRINT_BYTES {
                return Err(Error::new(
                    "INVALID_INPUT",
                    format!("Fingerprint input exceeds 16 MiB: {}", path.display()),
                ));
            }
            if meta.len() > *remaining {
                return Err(Error::new("SCAN_LIMIT", "Metadata read budget exhausted"));
            }
            // Lockfiles need an integrity fingerprint, not a parsed in-memory copy.
            // Keep a separate bound from the 1 MiB limit on declarations and drafts.
            let mut file = File::open(path)
                .map_err(|e| io_error(path, e))?
                .take(MAX_FINGERPRINT_BYTES + 1);
            let mut hasher = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            let mut total = 0;
            loop {
                let size = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
                let count = file
                    .read(&mut buffer[..size])
                    .map_err(|e| io_error(path, e))?;
                if count == 0 {
                    break;
                }
                if count as u64 > *remaining {
                    return Err(Error::new("SCAN_LIMIT", "Metadata read budget exhausted"));
                }
                *remaining -= count as u64;
                total += count as u64;
                if total > MAX_FINGERPRINT_BYTES {
                    return Err(Error::new(
                        "INVALID_INPUT",
                        format!("Fingerprint input exceeds 16 MiB: {}", path.display()),
                    ));
                }
                hasher.update(&buffer[..count]);
            }
            Ok(Some(format!("{:x}", hasher.finalize())))
        }
    }
}

pub fn directory(path: &Path) -> Result<std::path::PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|e| io_error(path, e))?;
    if !canonical.is_dir() {
        return Err(Error::new(
            "INVALID_INPUT",
            format!("Expected directory: {}", path.display()),
        ));
    }
    if canonical.to_str().is_none() {
        return Err(Error::new(
            "INVALID_INPUT",
            "Directory path must be valid UTF-8",
        ));
    }
    Ok(canonical)
}

pub fn publish(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| io_error(parent, e))?;
    temp.write_all(bytes).map_err(|e| io_error(path, e))?;
    temp.as_file().sync_all().map_err(|e| io_error(path, e))?;
    let result = if replace {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    };
    result.map_err(|e| {
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::new(
                "CONFLICT",
                format!("Refusing to overwrite {}", path.display()),
            )
        } else {
            io_error(path, e.error)
        }
    })?;
    Ok(())
}
