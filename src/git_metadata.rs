use crate::{Result, storage};
use std::path::Path;

// Labels describe the local installation. Git metadata is optional and never changes identity.
// Reading only this directory's .git prevents a parent workspace branch labeling unrelated apps.
pub(crate) fn current_branch(root: &Path, remaining: &mut u64) -> Result<Option<String>> {
    let git = root.join(".git");
    let head = if git.is_dir() {
        git.join("HEAD")
    } else if git.is_file() {
        let bytes = match storage::read_budgeted(&git, remaining) {
            Ok(bytes) => bytes,
            Err(e) if e.code == "SCAN_LIMIT" => return Err(e),
            Err(_) => return Ok(None),
        };
        let Some(dir) = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| s.trim().strip_prefix("gitdir: "))
        else {
            return Ok(None);
        };
        root.join(dir).join("HEAD")
    } else {
        return Ok(None);
    };
    let bytes = match storage::read_budgeted(&head, remaining) {
        Ok(bytes) => bytes,
        Err(e) if e.code == "SCAN_LIMIT" => return Err(e),
        Err(_) => return Ok(None),
    };
    let text = std::str::from_utf8(&bytes).unwrap_or("").trim();
    let branch = text
        .strip_prefix("ref: refs/heads/")
        .filter(|s| crate::manifest::valid_text(s));
    if let Some(branch) = branch {
        return Ok(Some(branch.into()));
    }
    if text.len() == 40 && text.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Ok(Some(text[..8].into()));
    }
    Ok(None)
}
