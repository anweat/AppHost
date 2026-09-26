//! Fixed local raster candidates. No recursive search, SVG, network or manifest execution.
use crate::{Error, Result, storage};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;
const CANDIDATES: &[&str] = &[
    "icon.png",
    "public/icon.png",
    "public/favicon.png",
    "favicon.ico",
    "public/favicon.ico",
];
const MAX_BYTES: u64 = 256 * 1024;
#[derive(Debug, Clone, Serialize)]
pub struct Icon {
    pub path: String,
    pub mime: String,
    pub sha256: String,
    pub bytes: usize,
}
fn read_candidate(root: &Path, name: &str) -> Result<(Icon, Vec<u8>)> {
    if !CANDIDATES.contains(&name) {
        return Err(Error::new("ICON_INVALID", "Not an allowed icon path"));
    }
    let root = root
        .canonicalize()
        .map_err(|e| storage::io_error(root, e))?;
    let path = root.join(name);
    let real = path
        .canonicalize()
        .map_err(|e| storage::io_error(&path, e))?;
    if !real.starts_with(&root) {
        return Err(Error::new("ICON_INVALID", "Icon escapes project root"));
    }
    let mut remaining = MAX_BYTES;
    let bytes = storage::read_budgeted(&path, &mut remaining)?;
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        && bytes.len() >= 33
        && &bytes[12..16] == b"IHDR"
    {
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        if !(1..=1024).contains(&width) || !(1..=1024).contains(&height) {
            return Err(Error::new("ICON_INVALID", "Icon dimensions exceed 1024"));
        }
        "image/png"
    } else if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() >= 22 {
        let count = usize::from(u16::from_le_bytes([bytes[4], bytes[5]]));
        if count == 0 || count > 32 || bytes.len() < 6 + count * 16 {
            return Err(Error::new("ICON_INVALID", "Invalid icon directory"));
        }
        for entry in bytes[6..6 + count * 16].as_chunks::<16>().0 {
            let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
            if size == 0
                || offset < 6 + count * 16
                || offset.checked_add(size).is_none_or(|end| end > bytes.len())
            {
                return Err(Error::new("ICON_INVALID", "Invalid icon payload"));
            }
        }
        "image/x-icon"
    } else {
        return Err(Error::new("ICON_INVALID", "Unsupported raster icon"));
    };
    Ok((
        Icon {
            path: name.into(),
            mime: mime.into(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes: bytes.len(),
        },
        bytes,
    ))
}
pub fn discover(data: &Path, id: Uuid) -> Result<Option<Icon>> {
    let installation = crate::registry::registered(data, id)?;
    Ok(discover_root(&installation.path))
}
fn discover_root(root: &Path) -> Option<Icon> {
    CANDIDATES
        .iter()
        .find_map(|name| read_candidate(root, name).ok().map(|(icon, _)| icon))
}
/// Tauri reads through this checked API rather than opening an arbitrary frontend
/// path. A changed file requires rediscovery and cannot reuse an old descriptor.
pub fn read(data: &Path, id: Uuid, name: &str, digest: &str) -> Result<Vec<u8>> {
    let installation = crate::registry::registered(data, id)?;
    let (icon, bytes) = read_candidate(&installation.path, name)?;
    if icon.sha256 != digest {
        return Err(Error::new("ICON_CHANGED", "Rediscover the changed icon"));
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_fixed_rasters_only_and_fallback_when_invalid() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("icon.png"), b"<svg onload='bad()'/>").unwrap();
        assert!(discover_root(t.path()).is_none());
        let mut png = vec![0; 33];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&32u32.to_be_bytes());
        png[20..24].copy_from_slice(&32u32.to_be_bytes());
        std::fs::write(t.path().join("icon.png"), &png).unwrap();
        assert_eq!(discover_root(t.path()).unwrap().mime, "image/png");
        png[16..20].copy_from_slice(&20000u32.to_be_bytes());
        std::fs::write(t.path().join("icon.png"), png).unwrap();
        assert!(discover_root(t.path()).is_none());
        assert!(read_candidate(t.path(), "../icon.png").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn linked_directory_cannot_escape_root() {
        let t = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), t.path().join("public")).unwrap();
        std::fs::write(outside.path().join("icon.png"), "x").unwrap();
        assert!(read_candidate(t.path(), "public/icon.png").is_err());
    }
}
