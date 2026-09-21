use crate::{Error, Result};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    path::Path,
};

fn unavailable(message: impl Into<String>) -> Error {
    Error::new("ENVIRONMENT_UNAVAILABLE", message)
}

pub(super) fn user_environment() -> Result<BTreeMap<String, String>> {
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0; 65536];
    // Reentrant account lookup uses a bounded buffer and does not trust HOME/USER
    // supplied by a terminal, GUI launcher or project process.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            &mut record,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if rc != 0 {
        return Err(unavailable(
            std::io::Error::from_raw_os_error(rc).to_string(),
        ));
    }
    if result.is_null() || record.pw_dir.is_null() || record.pw_name.is_null() {
        return Err(unavailable("Current system account has no home/name"));
    }
    let home = unsafe { CStr::from_ptr(record.pw_dir) }
        .to_str()
        .map_err(|_| unavailable("System home must be UTF-8"))?
        .to_owned();
    let user = unsafe { CStr::from_ptr(record.pw_name) }
        .to_str()
        .map_err(|_| unavailable("System user must be UTF-8"))?
        .to_owned();
    let mut temp = vec![0; 65536];
    let size = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            temp.as_mut_ptr(),
            temp.len(),
        )
    };
    if size == 0 || size > temp.len() {
        return Err(unavailable("System temporary directory is unavailable"));
    }
    let temp = unsafe { CStr::from_ptr(temp.as_ptr()) }
        .to_str()
        .map_err(|_| unavailable("System temporary directory must be UTF-8"))?
        .to_owned();
    if !Path::new(&home).is_absolute() || !Path::new(&temp).is_absolute() || user.is_empty() {
        return Err(unavailable("Invalid current account environment"));
    }
    Ok(BTreeMap::from([
        ("HOME".into(), home),
        ("USER".into(), user.clone()),
        ("LOGNAME".into(), user),
        ("TMPDIR".into(), temp),
        ("LANG".into(), "en_US.UTF-8".into()),
    ]))
}

pub(super) fn can_execute(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // Keep the symlink spelling: resolving .venv/bin/python to its interpreter
    // target would change virtual-environment discovery in the child.
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}
