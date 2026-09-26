use crate::{Error, Result};
use std::{
    path::Path,
    process::{Child, Stdio},
};
pub struct Pipes {
    pub child: Child,
    pub outputs: Vec<(Stdio, Stdio)>,
}
pub fn spawn(_: &Path, _: usize) -> Result<Pipes> {
    Err(Error::new(
        "UNSUPPORTED_PLATFORM",
        "Managed collector transport requires G3 on this platform",
    ))
}
pub fn ready(_: &mut Child) -> Result<()> {
    Err(Error::new(
        "UNSUPPORTED_PLATFORM",
        "Managed collector transport requires G3 on this platform",
    ))
}
