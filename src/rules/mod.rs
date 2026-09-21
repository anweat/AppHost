use crate::{Result, manifest::Service};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Trusted Rust rules run against a bounded metadata snapshot. They receive no traversal handle.
/// This is a source-level extension point, not a binary ABI or an untrusted-code sandbox.
pub trait DiscoveryRule {
    fn id(&self) -> &str;
    fn markers(&self) -> &[&str];
    fn inputs(&self) -> &[&str] {
        self.markers()
    }
    fn evaluate(&self, context: &RuleContext<'_>) -> Result<Proposal>;
}

pub struct RuleContext<'a> {
    pub(crate) name: &'a str,
    pub(crate) files: &'a BTreeMap<String, Vec<u8>>,
    pub(crate) present: &'a BTreeSet<String>,
}
impl RuleContext<'_> {
    pub fn name(&self) -> &str {
        self.name
    }
    pub fn file(&self, name: &str) -> Option<&[u8]> {
        self.files.get(name).map(Vec::as_slice)
    }
    pub fn has(&self, name: &str) -> bool {
        self.present.contains(name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub name: String,
    pub description: String,
    pub services: Vec<Service>,
    pub requires_confirmation: bool,
}

fn proposal(
    (name, description, command): (String, String, Vec<String>),
    requires_confirmation: bool,
) -> Proposal {
    Proposal {
        name,
        description,
        requires_confirmation,
        services: vec![Service {
            name: "app".into(),
            command,
            cwd: ".".into(),
            port: None,
            env: BTreeMap::new(),
        }],
    }
}

/// Rule inputs are single filenames. Rules cannot add globs, traversal or secret-file reads.
pub fn valid_input_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name != ".env"
        && !name.starts_with(".env.")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub mod grouping;
mod node;
mod python;
pub use node::NodeRule;
pub use python::PythonRule;
pub static NODE: NodeRule = NodeRule;
pub static PYTHON: PythonRule = PythonRule;
pub const BUILTIN_RULES: &[&dyn DiscoveryRule] = &[&NODE, &PYTHON];
