use crate::{
    Error, Result,
    manifest::{FILENAME, Issue, Manifest},
    rules::{BUILTIN_RULES, DiscoveryRule, Proposal, RuleContext, valid_input_name},
    storage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const INPUTS: &[&str] = &[
    FILENAME,
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
];
// Shared with the remote metadata test harness so it exercises the same traversal boundary.
pub const EXCLUDED: &[&str] = &[
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "target",
    "dist",
    "build",
    "__pycache__",
];

pub const MAX_DIRECTORIES: usize = 10_000;
pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_NESTED_HINTS: usize = 32;
pub const HINT_DEPTH: usize = 2;
// Kept as re-exports for metadata projection consumers.
use crate::rules::grouping::{CONVENTIONAL_GROUPING, GroupingRule};
pub use crate::rules::grouping::{SERVICE_DIRS, SERVICE_PAIRS};
mod groups;
pub use groups::accept_group;
use groups::{propose_groups, propose_nested_groups};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanOptions {
    pub max_depth: usize,
    pub max_directories: usize,
    pub max_entries: usize,
    pub max_read_bytes: u64,
    pub exclude: Vec<String>,
}
impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_directories: MAX_DIRECTORIES,
            max_entries: MAX_ENTRIES,
            max_read_bytes: MAX_READ_BYTES,
            exclude: vec![],
        }
    }
}
impl ScanOptions {
    fn validate(&self) -> Result<()> {
        if self.max_depth > 16
            || !(1..=MAX_DIRECTORIES).contains(&self.max_directories)
            || !(1..=MAX_ENTRIES).contains(&self.max_entries)
            || !(1..=MAX_READ_BYTES).contains(&self.max_read_bytes)
            || self.exclude.len() > 32
            || self.exclude.iter().any(|s| !valid_input_name(s))
        {
            return Err(Error::new(
                "INVALID_INPUT",
                "Invalid scan limits or excluded directory names",
            ));
        }
        Ok(())
    }
    fn excluded(&self, name: &str) -> bool {
        EXCLUDED.contains(&name) || self.exclude.iter().any(|s| s == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub schema_version: u32,
    pub root: PathBuf,
    pub candidates: Vec<Candidate>,
    pub issues: Vec<Issue>,
    #[serde(default)]
    pub scope: ScanOptions,
    #[serde(default)]
    pub groups: Vec<GroupSuggestion>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupSuggestion {
    pub members: Vec<PathBuf>,
    pub candidate: Candidate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub path: PathBuf,
    pub selected: bool,
    pub human_label: String,
    pub manifest: Option<Manifest>,
    pub evidence: Vec<Evidence>,
    pub issues: Vec<Issue>,
    #[serde(default)]
    pub notes: Vec<Issue>,
    #[serde(default)]
    pub nested_projects: Vec<PathBuf>,
    #[serde(default)]
    pub hints_truncated: bool,
    #[serde(default)]
    pub rule_inputs: Vec<String>,
    #[serde(default)]
    pub suggestions: BTreeMap<String, Proposal>,
    #[serde(default)]
    pub source_dirs: Vec<String>,
    #[serde(default)]
    pub reviewed_launch_sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub path: String,
    pub sha256: Option<String>,
}

struct RuleFiles {
    markers: BTreeSet<String>,
    bodies: BTreeSet<String>,
    inputs: BTreeSet<String>,
}
fn rule_files(rules: &[&dyn DiscoveryRule]) -> Result<RuleFiles> {
    if rules.len() > 16 {
        return Err(Error::new("INVALID_INPUT", "At most 16 discovery rules"));
    }
    let mut files = RuleFiles {
        markers: BTreeSet::from([FILENAME.into()]),
        bodies: BTreeSet::from([FILENAME.into()]),
        inputs: INPUTS.iter().map(|s| s.to_string()).collect(),
    };
    let mut ids = BTreeSet::new();
    for rule in rules {
        if !valid_input_name(rule.id())
            || !ids.insert(rule.id())
            || rule.markers().is_empty()
            || rule.markers().len() > 32
            || rule.inputs().len() > 32
        {
            return Err(Error::new(
                "INVALID_INPUT",
                "Rules require unique IDs and bounded, nonempty markers",
            ));
        }
        for name in rule.markers().iter().chain(rule.inputs()) {
            if !valid_input_name(name) || *name == FILENAME {
                return Err(Error::new(
                    "INVALID_INPUT",
                    "Rule inputs must be safe filenames; apphost.toml is reserved",
                ));
            }
            files.inputs.insert((*name).into());
            files.bodies.insert((*name).into());
        }
        files
            .markers
            .extend(rule.markers().iter().map(|s| s.to_string()));
    }
    if files.inputs.len() > 32 {
        return Err(Error::new(
            "INVALID_INPUT",
            "At most 32 distinct discovery inputs",
        ));
    }
    Ok(files)
}

pub fn scan(root: &Path, max_depth: usize) -> Result<Draft> {
    scan_with_options(
        root,
        &ScanOptions {
            max_depth,
            ..ScanOptions::default()
        },
    )
}
pub fn scan_with_options(root: &Path, options: &ScanOptions) -> Result<Draft> {
    scan_with_rules(root, options, BUILTIN_RULES)
}
/// The caller supplies the complete rule set; replacing a built-in is explicit, not priority-based.
pub fn scan_with_rules(
    root: &Path,
    options: &ScanOptions,
    rules: &[&dyn DiscoveryRule],
) -> Result<Draft> {
    scan_with_policies(root, options, rules, &CONVENTIONAL_GROUPING)
}

/// Replace stack and grouping judgments without replacing bounded traversal or import checks.
pub fn scan_with_policies(
    root: &Path,
    options: &ScanOptions,
    rules: &[&dyn DiscoveryRule],
    grouping: &dyn GroupingRule,
) -> Result<Draft> {
    scan_internal(root, options, rules, grouping, None)
}

pub fn scan_with_registry(
    root: &Path,
    options: &ScanOptions,
    data_dir: Option<&Path>,
) -> Result<Draft> {
    scan_internal(
        root,
        options,
        BUILTIN_RULES,
        &CONVENTIONAL_GROUPING,
        data_dir,
    )
}

fn scan_internal(
    root: &Path,
    options: &ScanOptions,
    rules: &[&dyn DiscoveryRule],
    grouping: &dyn GroupingRule,
    data_dir: Option<&Path>,
) -> Result<Draft> {
    options.validate()?;
    let files = rule_files(rules)?;
    let root = storage::directory(root)?;
    let mut draft = Draft {
        schema_version: 0,
        root: root.clone(),
        candidates: vec![],
        issues: vec![],
        scope: options.clone(),
        groups: vec![],
    };
    let mut pending = vec![(root.clone(), 0, None::<usize>, 0)];
    let mut visited = 0;
    let mut visited_paths = BTreeSet::new();
    let mut entries_seen = 0;
    let mut remaining = options.max_read_bytes;
    while let Some((path, depth, mut owner, mut owner_depth)) = pending.pop() {
        if owner.is_some_and(|i| draft.candidates[i].hints_truncated) {
            continue;
        }
        visited += 1;
        if visited > options.max_directories {
            return Err(Error::new(
                "SCAN_LIMIT",
                "Directory budget exhausted; narrow scan scope",
            ));
        }
        if path.to_str().is_none() {
            draft.issues.push(Issue::new(
                path.to_string_lossy(),
                "Directory path must be valid UTF-8",
            ));
            continue;
        }
        visited_paths.insert(path.clone());
        // A conventional service directory is evidence of a possible missing service even
        // without a language marker. Only already visited direct children qualify.
        let mut found = owner.is_some_and(|i| {
            path.parent() == Some(draft.candidates[i].path.as_path())
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| grouping.is_service_directory(n))
        });
        for name in &files.markers {
            match fs::symlink_metadata(path.join(name)) {
                Ok(_) => found = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => draft
                    .issues
                    .push(Issue::new(path.display().to_string(), e.to_string())),
            }
        }
        if found {
            if let Some(index) = owner {
                let candidate = &mut draft.candidates[index];
                candidate
                    .nested_projects
                    .push(path.strip_prefix(&candidate.path).unwrap().to_owned());
                if candidate.nested_projects.len() >= MAX_NESTED_HINTS {
                    candidate.hints_truncated = true;
                    continue;
                }
            } else {
                draft
                    .candidates
                    .push(candidate(&path, &files, rules, &mut remaining)?);
                // A project-authored manifest owns its boundary. Nested inspection is only a hint for inferred projects.
                if fs::symlink_metadata(path.join(FILENAME)).is_ok() {
                    continue;
                }
                owner = Some(draft.candidates.len() - 1);
                owner_depth = depth;
            }
        }
        if depth >= options.max_depth || owner.is_some() && depth >= owner_depth + HINT_DEPTH {
            // Validate a root with depth zero as well; unreadable must not mean an empty scan.
            if path == root {
                fs::read_dir(&path).map_err(|e| storage::io_error(&path, e))?;
            }
            continue;
        }
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) if path == root => return Err(storage::io_error(&path, e)),
            Err(e) => {
                draft
                    .issues
                    .push(Issue::new(path.display().to_string(), e.to_string()));
                continue;
            }
        };
        let mut children = vec![];
        for entry in entries {
            entries_seen += 1;
            if entries_seen > options.max_entries {
                return Err(Error::new(
                    "SCAN_LIMIT",
                    "Directory-entry budget exhausted; narrow scan scope",
                ));
            }
            match entry {
                Ok(entry) => match entry.file_type() {
                    Ok(kind)
                        if kind.is_dir()
                            && !options.excluded(&entry.file_name().to_string_lossy()) =>
                    {
                        children.push(entry.path())
                    }
                    Ok(_) => {}
                    Err(e) => draft.issues.push(Issue::new(
                        entry.path().display().to_string(),
                        e.to_string(),
                    )),
                },
                Err(e) => draft
                    .issues
                    .push(Issue::new(path.display().to_string(), e.to_string())),
            }
            if pending.len() + children.len() + visited > options.max_directories {
                return Err(Error::new(
                    "SCAN_LIMIT",
                    "Directory budget exhausted; narrow scan scope",
                ));
            }
        }
        children.sort();
        pending.extend(
            children
                .into_iter()
                .rev()
                .map(|p| (p, depth + 1, owner, owner_depth)),
        );
    }
    propose_groups(
        &mut draft,
        &files,
        rules,
        grouping,
        &visited_paths,
        &mut remaining,
    )?;
    propose_nested_groups(&mut draft, &files, rules, grouping, &mut remaining)?;
    for c in &mut draft.candidates {
        if !c.nested_projects.is_empty() && !authoritative(c) {
            c.selected = false;
            c.notes.push(Issue::new("group", "Nested project/service directories may contain additional services; confirm the App boundary"));
        }
    }
    for c in draft
        .candidates
        .iter_mut()
        .chain(draft.groups.iter_mut().map(|g| &mut g.candidate))
    {
        c.refresh_launch_notes_budgeted(&mut remaining)?;
    }
    if let Some(data_dir) = data_dir {
        crate::registry::annotate_copies(data_dir, &mut draft, &mut remaining);
    }
    draft.candidates.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(draft)
}

fn candidate(
    root: &Path,
    inputs: &RuleFiles,
    rules: &[&dyn DiscoveryRule],
    remaining: &mut u64,
) -> Result<Candidate> {
    let fallback = root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let human_label = fallback.clone();
    let mut result = Candidate {
        path: root.into(),
        selected: false,
        human_label,
        manifest: None,
        evidence: vec![],
        issues: vec![],
        notes: vec![],
        nested_projects: vec![],
        hints_truncated: false,
        rule_inputs: vec![],
        suggestions: BTreeMap::new(),
        source_dirs: vec![],
        reviewed_launch_sha256: None,
    };
    let authoritative = fs::symlink_metadata(root.join(FILENAME)).is_ok();
    let names = if authoritative {
        BTreeSet::from([FILENAME.to_string()])
    } else {
        inputs.inputs.clone()
    };
    let mut contents = BTreeMap::new();
    let mut present = BTreeSet::new();
    for name in &names {
        let path = root.join(name);
        let value = (|| {
            match fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(storage::io_error(&path, e)),
                Ok(_) => {
                    present.insert(name.clone());
                }
            }
            if inputs.bodies.contains(name) {
                let bytes = storage::read_budgeted(&path, remaining)?;
                let digest = format!("{:x}", Sha256::digest(&bytes));
                contents.insert(name.clone(), bytes);
                Ok(Some(digest))
            } else {
                storage::fingerprint_budgeted(&path, remaining)
            }
        })();
        match value {
            Ok(sha256) => result.evidence.push(Evidence {
                path: name.clone(),
                sha256,
            }),
            Err(e) if e.code == "SCAN_LIMIT" => return Err(e),
            Err(e) => result.issues.push(Issue::new(name, e.to_string())),
        }
    }
    if authoritative {
        if let Some(bytes) = contents.get(FILENAME) {
            match std::str::from_utf8(bytes)
                .map_err(|e| e.to_string())
                .and_then(|s| toml::from_str::<Manifest>(s).map_err(|e| e.to_string()))
            {
                Ok(manifest) => {
                    result.issues.extend(manifest.validate(root));
                    result.manifest = Some(manifest);
                }
                Err(e) => result.issues.push(Issue::new("manifest", e)),
            }
        }
        result.selected = result.issues.is_empty() && result.manifest.is_some();
        return Ok(result);
    }
    result.rule_inputs = names
        .iter()
        .filter(|name| !INPUTS.contains(&name.as_str()))
        .cloned()
        .collect();
    let context = RuleContext {
        name: &fallback,
        files: &contents,
        present: &present,
    };
    let matched: Vec<_> = rules
        .iter()
        .filter(|r| r.markers().iter().any(|name| present.contains(*name)))
        .collect();
    for rule in &matched {
        match rule.evaluate(&context) {
            Ok(proposal) => {
                result.suggestions.insert(rule.id().to_string(), proposal);
            }
            Err(e) => result
                .issues
                .push(Issue::new(format!("rules.{}", rule.id()), e.to_string())),
        }
    }
    let mut manifest = Manifest {
        schema_version: 0,
        project_id: Uuid::new_v4(),
        name: fallback,
        description: String::new(),
        services: vec![],
    };
    if matched.len() > 1 {
        result.issues.push(Issue::new(
            "rules",
            "Multiple discovery rules match; explicitly choose or combine launch declarations",
        ));
    } else if let Some(proposal) = result.suggestions.values().next() {
        manifest.name = proposal.name.clone();
        manifest.description = proposal.description.clone();
        manifest.services = proposal.services.clone();
        result.issues.extend(manifest.validate(root));
        if proposal.requires_confirmation {
            result.notes.push(Issue::new(
                "launch",
                if proposal.services.is_empty() {
                    "Launch command is unknown; specify the service command explicitly"
                } else {
                    "A command-line entry point may be a library/tool or build watcher; confirm it launches the intended App"
                },
            ));
        }
        result.selected = !proposal.requires_confirmation && result.issues.is_empty();
    }
    if matched.is_empty() {
        result
            .issues
            .push(Issue::new("rules", "No discovery rule matched"));
    }
    result.manifest = Some(manifest);
    Ok(result)
}

fn authoritative(candidate: &Candidate) -> bool {
    candidate
        .evidence
        .iter()
        .any(|e| e.path == FILENAME && e.sha256.is_some())
        || fs::symlink_metadata(candidate.path.join(FILENAME)).is_ok()
}

impl Candidate {
    pub(crate) fn refresh_launch_notes_budgeted(&mut self, remaining: &mut u64) -> Result<()> {
        self.notes.retain(|n| !n.field.starts_with("launch_check."));
        if let Some(manifest) = &self.manifest {
            self.notes.extend(
                manifest
                    .launch_warnings_budgeted(&self.path, remaining)?
                    .into_iter()
                    .map(|i| Issue::new(format!("launch_check.{}", i.field), i.message)),
            );
        }
        Ok(())
    }
}
