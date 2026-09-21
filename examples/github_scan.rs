//! No-clone GitHub scan experiment. Uses `gh api` GET requests and the real local scanner.
//! `cargo run --example github_scan -- --repo owner/repo` (repeatable), or
//! `cargo run --example github_scan -- --search 'topic:self-hosted stars:>1000' --limit 10`.
//! With neither option, run a deliberately mixed corpus including negative controls.
//! JSON goes to stdout only; temporary metadata is removed on exit. Optional import smoke tests use only temporary metadata and an isolated registry; no launch calls.

use apphost::{
    discovery::{EXCLUDED, INPUTS},
    manifest::FILENAME,
    rules::grouping::{CONVENTIONAL_GROUPING, GroupingRule},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
const FILE_LIMIT: u64 = 1024 * 1024;
const FINGERPRINT_LIMIT: u64 = 16 * 1024 * 1024;
const TREE_ENTRY_LIMIT: usize = 20_000;
const TRANSFER_LIMIT: u64 = 8 * 1024 * 1024;
const API_LIMIT: u64 = 12 * 1024 * 1024;
const MARKERS: &[&str] = &[
    FILENAME,
    "package.json",
    "pyproject.toml",
    "requirements.txt",
];

#[derive(Parser)]
#[command(
    about = "Exercise AppHost scan on GitHub metadata without git clone or project execution"
)]
struct Args {
    /// Scan the temporary parent collection, retaining max-depth relative to the repository.
    #[arg(long)]
    scan_parent: bool,
    /// Exercise review/import/list on temporary metadata; skip all unresolved decisions.
    #[arg(long)]
    exercise_import: bool,
    /// Public repository in owner/name form; repeat for several repositories.
    #[arg(long = "repo")]
    repositories: Vec<String>,
    /// GitHub repository search query, sorted by stars. Combined with explicit repositories.
    #[arg(long)]
    search: Option<String>,
    /// Maximum number of search results (explicit repos are not included in this count).
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=30))]
    limit: u8,
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(0..=16))]
    max_depth: u8,
    /// Restrict the fetched tree to one repository subdirectory, without fetching the whole tree.
    #[arg(long)]
    subdir: Option<String>,
    /// Optional branch/tag/commit for explicitly named repositories; pinned before reading files.
    #[arg(long, conflicts_with = "search", requires = "repositories")]
    reference: Option<String>,
}

#[derive(Clone)]
struct Case {
    repo: String,
    negative_control: bool,
}

fn corpus() -> Vec<Case> {
    [
        ("louislam/uptime-kuma", false),
        ("gethomepage/homepage", false),
        ("actualbudget/actual", false),
        ("usememos/memos", false),
        ("immich-app/immich", false),
        ("open-webui/open-webui", false),
        ("comfyanonymous/ComfyUI", false),
        ("AUTOMATIC1111/stable-diffusion-webui", false),
        // These are libraries / developer CLI tools, not directly launchable local web Apps.
        ("encode/httpx", true),
        ("pallets/flask", true),
        ("yt-dlp/yt-dlp", true),
    ]
    .into_iter()
    .map(|(repo, negative_control)| Case {
        repo: repo.into(),
        negative_control,
    })
    .collect()
}

#[derive(Debug, Clone, Deserialize)]
struct Entry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    size: Option<u64>,
}
#[derive(Deserialize)]
struct Tree {
    tree: Vec<Entry>,
    truncated: bool,
}
#[derive(Deserialize)]
struct Blob {
    content: String,
    encoding: String,
    size: u64,
    sha: String,
}

#[derive(Default, Serialize)]
struct Stats {
    requests: usize,
    retries: usize,
    metadata_files: usize,
    metadata_bytes: u64,
}

// A fixed, read-only gh command is used instead of a second HTTP/authentication stack.
// stdout/stderr are bounded temporary files so a slow or oversized response cannot fill memory.
fn api(endpoint: &str, params: &[(&str, &str)], stats: &mut Stats) -> Result<Value> {
    match api_once(endpoint, params, stats) {
        Err(error) if transient(&error) => {
            // One retry for transport failures only. No auth, rate-limit or not-found fallback.
            stats.retries += 1;
            eprintln!("Transient GitHub transport error; retrying GET {endpoint} once");
            thread::sleep(Duration::from_millis(500));
            api_once(endpoint, params, stats)
        }
        result => result,
    }
}

fn transient(error: &str) -> bool {
    [
        "EOF",
        "TLS handshake timeout",
        "connection reset by peer",
        "HTTP 502",
        "HTTP 503",
        "HTTP 504",
    ]
    .iter()
    .any(|text| error.contains(text))
}

fn api_once(endpoint: &str, params: &[(&str, &str)], stats: &mut Stats) -> Result<Value> {
    stats.requests += 1;
    let mut out = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut err = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut command = Command::new("gh");
    command.args([
        "api",
        "--hostname",
        "github.com",
        "--method",
        "GET",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
        endpoint,
    ]);
    for (name, value) in params {
        command.arg("-f").arg(format!("{name}={value}"));
    }
    command
        .env("GH_PROMPT_DISABLED", "1")
        .env_remove("GH_DEBUG")
        .stdin(Stdio::null())
        .stdout(out.try_clone().map_err(|e| e.to_string())?)
        .stderr(err.try_clone().map_err(|e| e.to_string())?);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot run gh; install/login with GitHub CLI: {e}"))?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(30)
            || out.metadata().map_err(|e| e.to_string())?.len() > API_LIMIT
            || err.metadata().map_err(|e| e.to_string())?.len() > API_LIMIT
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err("GitHub request timed out or exceeded response limit".into());
        }
        thread::sleep(Duration::from_millis(25));
    };
    if out.metadata().map_err(|e| e.to_string())?.len() > API_LIMIT {
        return Err("GitHub response exceeds limit".into());
    }
    if !status.success() {
        err.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut message = String::new();
        err.take(4096)
            .read_to_string(&mut message)
            .map_err(|e| e.to_string())?;
        return Err(format!("GitHub GET {endpoint} failed: {}", message.trim()));
    }
    out.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    serde_json::from_reader(out).map_err(|e| format!("Invalid GitHub JSON: {e}"))
}

fn repo_name(value: &str) -> Result<()> {
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    {
        return Err("Expected owner/repo, not a URL or local path".into());
    }
    Ok(())
}

fn sha(value: &str) -> Result<&str> {
    if value.len() == 40 && value.bytes().all(|c| c.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err("Invalid Git object SHA".into())
    }
}

fn encode_segment(value: &str) -> String {
    value
        .bytes()
        .map(|c| {
            if c.is_ascii_alphanumeric() || b"-_.~".contains(&c) {
                (c as char).to_string()
            } else {
                format!("%{c:02X}")
            }
        })
        .collect()
}

fn relative(value: &str) -> Result<&Path> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains(['\\', ':', '\0'])
        || value
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        Err(format!("Unsafe tree path: {value:?}"))
    } else {
        Ok(path)
    }
}

fn excluded(path: &Path) -> bool {
    path.components()
        .any(|c| EXCLUDED.contains(&c.as_os_str().to_string_lossy().as_ref()))
}
fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}
fn marker(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| MARKERS.contains(&s))
}

fn input_limit(path: &str) -> u64 {
    if Path::new(path)
        .file_name()
        .and_then(|p| p.to_str())
        .is_some_and(|name| MARKERS.contains(&name))
    {
        FILE_LIMIT
    } else {
        FINGERPRINT_LIMIT
    }
}

fn scoped_tree_sha(
    repo: &str,
    tree_sha: &str,
    subdir: Option<&str>,
    stats: &mut Stats,
) -> Result<String> {
    let Some(subdir) = subdir else {
        return Ok(tree_sha.into());
    };
    let path = relative(subdir)?;
    if path.components().count() > 16 {
        return Err("Subdirectory scope exceeds 16 levels".into());
    }
    let mut current = tree_sha.to_string();
    for component in path.components() {
        let tree: Tree = serde_json::from_value(api(
            &format!("repos/{repo}/git/trees/{current}"),
            &[],
            stats,
        )?)
        .map_err(|e| e.to_string())?;
        if tree.truncated || tree.tree.len() > TREE_ENTRY_LIMIT {
            return Err("Scope resolution tree is truncated or exceeds entry budget".into());
        }
        let name = component
            .as_os_str()
            .to_str()
            .ok_or("Invalid subdirectory")?;
        let entry = tree
            .tree
            .iter()
            .find(|e| e.path == name && e.kind == "tree")
            .ok_or("Requested subdirectory is not a tracked directory")?;
        current = sha(&entry.sha)?.into();
    }
    Ok(current)
}

struct Plan {
    directories: Vec<PathBuf>,
    files: Vec<Entry>,
    visible: Vec<PathBuf>,
    hidden: Vec<PathBuf>,
    unsupported_markers: Vec<String>,
}

#[cfg(test)]
fn plan(tree: &Tree, depth: usize) -> Result<Plan> {
    plan_with_parent(tree, depth, false)
}

fn plan_with_parent(tree: &Tree, depth: usize, scan_parent: bool) -> Result<Plan> {
    if tree.truncated {
        return Err("GitHub tree is truncated; cannot judge scan coverage".into());
    }
    if tree.tree.len() > TREE_ENTRY_LIMIT {
        return Err("Tree exceeds 20,000 entries; narrow with --subdir".into());
    }
    let mut directories = BTreeSet::new();
    let mut markers = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut unsupported_markers = vec![];
    for entry in &tree.tree {
        let path = relative(&entry.path)?;
        if !seen.insert(path) {
            return Err("Duplicate tree path".into());
        }
        if entry.kind == "tree" {
            directories.insert(path.to_owned());
        }
        if excluded(path) {
            continue;
        }
        if entry.kind != "tree" && marker(path) && parent(path).components().count() <= depth {
            markers.insert(parent(path).to_owned());
        }
        if path.components().count() == 1
            && [
                "go.mod",
                "Cargo.toml",
                "setup.py",
                "Dockerfile",
                "docker-compose.yml",
                "compose.yaml",
            ]
            .contains(&entry.path.as_str())
        {
            unsupported_markers.push(entry.path.clone());
        }
    }
    if directories.len() > 10_000 {
        return Err(
            "Repository exceeds 10,000 directories; bounded experiment cannot judge it".into(),
        );
    }
    let mut ordered: Vec<_> = markers.into_iter().collect();
    ordered.sort_by_key(|p| (p.components().count(), p.clone()));
    let mut visible: Vec<PathBuf> = vec![];
    let mut hidden = vec![];
    for path in ordered {
        if visible.iter().any(|p| path.starts_with(p)) {
            hidden.push(path);
        } else {
            visible.push(path);
        }
    }
    let inferred_roots = visible.clone();
    let mut metadata_roots = visible.clone();
    for directory in &directories {
        if (!scan_parent && parent(directory).as_os_str().is_empty())
            || excluded(directory)
            || directory.components().count() > depth
            || visible.iter().any(|p| directory.starts_with(p))
        {
            continue;
        }
        let siblings = directories
            .iter()
            .filter(|p| parent(p) == parent(directory))
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        let paired = CONVENTIONAL_GROUPING
            .sibling_pairs(&siblings)
            .iter()
            .any(|pair| {
                pair.iter()
                    .any(|name| directory.file_name().is_some_and(|n| n == name.as_str()))
                    && inferred_roots
                        .iter()
                        .any(|p| pair.iter().any(|name| p == &parent(directory).join(name)))
            });
        if paired {
            metadata_roots.push(directory.clone());
        }
    }
    let manifest_dirs: BTreeSet<_> = tree
        .tree
        .iter()
        .filter(|e| {
            Path::new(&e.path)
                .file_name()
                .is_some_and(|name| name == FILENAME)
        })
        .map(|e| parent(Path::new(&e.path)))
        .collect();
    // A confirmed grouping proposal needs real nested metadata, not the existence-only
    // placeholders used for other hints. Apply exactly the core's direct frontend scope.
    let mut grouped_children = BTreeSet::new();
    for root in &visible {
        if (!scan_parent && root.as_os_str().is_empty()) || manifest_dirs.contains(root.as_path()) {
            continue;
        }
        let frontends: BTreeSet<_> = hidden
            .iter()
            .filter(|p| parent(p) == root)
            .cloned()
            .chain(
                directories
                    .iter()
                    .filter(|p| {
                        parent(p) == root
                            && p.components().count() <= depth
                            && p.file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| CONVENTIONAL_GROUPING.is_service_directory(n))
                    })
                    .cloned(),
            )
            .collect();
        let frontends: Vec<_> = frontends.into_iter().collect();
        if frontends.len() == 1
            && frontends[0]
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| CONVENTIONAL_GROUPING.is_service_directory(n))
            && !manifest_dirs.contains(frontends[0].as_path())
        {
            grouped_children.insert(frontends[0].clone());
        }
    }
    let files: Vec<_> = tree
        .tree
        .iter()
        .filter(|entry| {
            let path = Path::new(&entry.path);
            (metadata_roots.iter().any(|p| p == parent(path))
                || grouped_children.contains(parent(path)))
                && (!manifest_dirs.contains(parent(path))
                    || path.file_name().is_some_and(|name| name == FILENAME))
                && path
                    .file_name()
                    .and_then(|p| p.to_str())
                    .is_some_and(|p| INPUTS.contains(&p))
        })
        .cloned()
        .collect();
    if !manifest_dirs.is_empty() && tree.tree.iter().any(|e| e.mode == "120000") {
        return Err("Repository has a manifest and symlinks; remote cwd validation cannot faithfully reproduce them".into());
    }
    if files.len() > 80 {
        return Err("More than 80 metadata files required; use a narrower repository".into());
    }
    for file in &files {
        if file.kind != "blob" || !["100644", "100755"].contains(&file.mode.as_str()) {
            return Err(format!(
                "Metadata {} is not a regular Git blob; no symlink approximation",
                file.path
            ));
        }
        sha(&file.sha)?;
        if file.size.is_none() {
            return Err("Metadata size is missing".into());
        }
    }
    if files
        .iter()
        .filter(|f| f.size.is_some_and(|s| s <= input_limit(&f.path)))
        .filter_map(|f| f.size)
        .sum::<u64>()
        > TRANSFER_LIMIT
    {
        return Err("Required metadata exceeds 8 MiB transfer budget".into());
    }
    Ok(Plan {
        directories: directories.into_iter().collect(),
        files,
        visible,
        hidden,
        unsupported_markers,
    })
}

fn blob_bytes(value: Value, file: &Entry) -> Result<Vec<u8>> {
    let blob: Blob = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if blob.encoding != "base64"
        || blob.sha != file.sha
        || Some(blob.size) != file.size
        || blob.size > input_limit(&file.path)
    {
        return Err("Blob encoding, SHA or size does not match pinned tree".into());
    }
    let content: String = blob
        .content
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let bytes = STANDARD.decode(content).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != blob.size {
        return Err("Decoded blob length mismatch".into());
    }
    Ok(bytes)
}

fn observation(candidate: &apphost::discovery::Candidate, root: &Path) -> Value {
    let path = candidate.path.strip_prefix(root).unwrap().to_string_lossy();
    let mut details = json!({"path": if path.is_empty() { "." } else { &path }, "selected":candidate.selected,"issues":candidate.issues,"notes":candidate.notes,"nested_projects":candidate.nested_projects,"hints_truncated":candidate.hints_truncated,"suggestions":candidate.suggestions,
        "services":candidate.manifest.as_ref().map(|m| &m.services)});
    if let Ok(bytes) = fs::read(candidate.path.join("package.json"))
        && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
    {
        details["node_scripts"] = value.get("scripts").cloned().unwrap_or(Value::Null);
        details["package_manager"] = value.get("packageManager").cloned().unwrap_or(Value::Null);
        details["workspaces"] = value.get("workspaces").cloned().unwrap_or(Value::Null);
    }
    if let Ok(text) = fs::read_to_string(candidate.path.join("pyproject.toml"))
        && let Ok(value) = toml::from_str::<toml::Value>(&text)
    {
        details["python_entrypoints"] = value
            .get("project")
            .and_then(|v| v.get("scripts"))
            .map(|v| json!(v))
            .unwrap_or(Value::Null);
    }
    details
}

fn experiment(
    case: &Case,
    reference: Option<&str>,
    subdir: Option<&str>,
    depth: usize,
    exercise_import: bool,
    scan_parent: bool,
    stats: &mut Stats,
) -> Result<Value> {
    repo_name(&case.repo)?;
    let revision = encode_segment(reference.unwrap_or("HEAD"));
    let commit = api(
        &format!("repos/{}/commits/{revision}", case.repo),
        &[],
        stats,
    )?;
    let commit_sha = sha(commit["sha"].as_str().ok_or("Missing commit SHA")?)?;
    let tree_sha = sha(commit["commit"]["tree"]["sha"]
        .as_str()
        .ok_or("Missing tree SHA")?)?;
    let tree_sha = scoped_tree_sha(&case.repo, tree_sha, subdir, stats)?;
    let tree: Tree = serde_json::from_value(api(
        &format!("repos/{}/git/trees/{tree_sha}", case.repo),
        &[("recursive", "1")],
        stats,
    )?)
    .map_err(|e| e.to_string())?;
    let plan = plan_with_parent(&tree, depth, scan_parent)?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    // Match the actual scoped directory name because rules use it as the fallback display name.
    let root_name = subdir
        .and_then(|p| Path::new(p).file_name())
        .unwrap_or_else(|| std::ffi::OsStr::new(case.repo.split('/').nth(1).unwrap()));
    let root = temp.path().join(root_name);
    fs::create_dir(&root).map_err(|e| e.to_string())?;
    for directory in &plan.directories {
        fs::create_dir_all(root.join(directory)).map_err(|e| e.to_string())?;
    }
    // Only marker existence is consumed during nested-hint traversal. Preserve that existence
    // without downloading nested project contents or letting them become independent imports.
    for entry in &tree.tree {
        let path = relative(&entry.path)?;
        if marker(path) && plan.hidden.iter().any(|p| p == parent(path)) {
            fs::create_dir_all(root.join(parent(path))).map_err(|e| e.to_string())?;
            fs::write(root.join(path), []).map_err(|e| e.to_string())?;
        }
    }
    let mut oversized = vec![];
    for file in &plan.files {
        let path = root.join(relative(&file.path)?);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        if file.size.unwrap() > input_limit(&file.path) {
            // Content is irrelevant beyond scan's hard read bound. A sparse sentinel reproduces
            // that boundary without downloading the oversized blob; disclose it in the result.
            fs::File::create(&path)
                .and_then(|f| f.set_len(input_limit(&file.path) + 1))
                .map_err(|e| e.to_string())?;
            oversized.push(json!({"path":file.path,"bytes":file.size}));
        } else {
            let value = api(
                &format!("repos/{}/git/blobs/{}", case.repo, file.sha),
                &[],
                stats,
            )?;
            let bytes = blob_bytes(value, file)?;
            stats.metadata_files += 1;
            stats.metadata_bytes += bytes.len() as u64;
            fs::write(&path, bytes).map_err(|e| e.to_string())?;
        }
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let scan_started = Instant::now();
    let draft = scan_projection(&root, depth, scan_parent)?;
    let scan_ms = scan_started.elapsed().as_secs_f64() * 1000.0;
    if !draft.issues.is_empty() {
        return Err(format!("Metadata-view scan incomplete: {:?}", draft.issues));
    }
    let observed: BTreeSet<_> = draft
        .candidates
        .iter()
        .filter(|c| MARKERS.iter().any(|n| c.path.join(n).exists()))
        .map(|c| c.path.strip_prefix(&root).unwrap().to_owned())
        .collect();
    if observed != plan.visible.iter().cloned().collect() {
        return Err("Metadata projection did not preserve candidate directories".into());
    }
    let selected = draft.candidates.iter().filter(|c| c.selected).count();
    let outcome = if draft.candidates.is_empty() {
        "no_candidates"
    } else if selected == 0 {
        "manual_required"
    } else if selected == draft.candidates.len() {
        "auto_selected"
    } else {
        "mixed"
    };
    let mut findings = vec![];
    if case.negative_control && selected > 0 {
        findings.push("negative_control_auto_selected");
    }
    if !plan.hidden.is_empty() {
        findings.push("nested_markers_hidden_by_parent");
    }
    if !oversized.is_empty() {
        findings.push("metadata_exceeds_local_read_limit");
    }
    if !plan.unsupported_markers.is_empty() {
        findings.push("other_runtime_markers_need_review");
    }
    if draft.candidates.iter().any(|c| {
        c.selected && c.path.join("pyproject.toml").is_file() && !c.path.join(FILENAME).is_file()
    }) {
        findings.push("python_console_entrypoint_is_not_proof_of_a_server");
    }
    let pipeline = if exercise_import {
        Some(exercise_pipeline(&draft, &temp.path().join("registry"))?)
    } else {
        None
    };
    let result = json!({"repository":case.repo,"commit":commit_sha,"url":format!("https://github.com/{}/tree/{commit_sha}",case.repo),
        "scan_ms":scan_ms,"pipeline":pipeline,"temporary_data_removed":true,"scope":{"subdir":subdir,"max_depth":depth,"scan_parent":scan_parent,"max_tree_entries":TREE_ENTRY_LIMIT},"negative_control":case.negative_control,"outcome":outcome,"selected":selected,
        "candidates":draft.candidates.iter().map(|c| observation(c,&root)).collect::<Vec<_>>(),
        "groups":draft.groups.iter().map(|g| json!({"members":g.members.iter().map(|p|p.strip_prefix(&root).unwrap().to_string_lossy()).collect::<Vec<_>>(),"candidate":observation(&g.candidate,&root)})).collect::<Vec<_>>(),
        "hidden_marker_directories":plan.hidden.iter().map(|p|p.to_string_lossy()).collect::<Vec<_>>(),
        "other_runtime_markers":plan.unsupported_markers,"oversized_metadata":oversized,"findings":findings});
    temp.close()
        .map_err(|e| format!("Temporary data cleanup failed: {e}"))?;
    Ok(result)
}

// The extra collection level changes grouping eligibility, not repository coverage.
fn scan_projection(root: &Path, depth: usize, scan_parent: bool) -> Result<apphost::Draft> {
    let (scan_root, scan_depth) = if scan_parent {
        (root.parent().ok_or("Repository has no parent")?, depth + 1)
    } else {
        (root, depth)
    };
    apphost::scan(scan_root, scan_depth).map_err(|e| e.to_string())
}

fn exercise_pipeline(draft: &apphost::Draft, data_dir: &Path) -> Result<Value> {
    let started = Instant::now();
    let unresolved = draft.candidates.iter().filter(|c| !c.selected).count();
    let mut reviewed = draft.clone();
    // These are benchmark decisions, not human usability timings or inferred launch commands.
    // Reject every group and unknown candidate; only the already selected declarations proceed.
    let answers = "n\n".repeat(draft.groups.len() + unresolved);
    apphost::review::review(
        &mut reviewed,
        &mut std::io::Cursor::new(answers),
        &mut std::io::sink(),
    )
    .map_err(|e| e.to_string())?;
    let imported =
        apphost::registry::import_with_receipts(data_dir, &reviewed).map_err(|e| e.to_string())?;
    let listings = apphost::list(data_dir, false).map_err(|e| e.to_string())?;
    if listings.len() != imported.len()
        || listings
            .iter()
            .any(|i| i.requires_launch_confirmation || i.manifest.is_none())
    {
        return Err("Imported declaration is missing or unconfirmed in list".into());
    }
    let retried =
        apphost::registry::import_with_receipts(data_dir, &reviewed).map_err(|e| e.to_string())?;
    if imported
        .iter()
        .map(|r| r.installation.installation_id)
        .collect::<Vec<_>>()
        != retried
            .iter()
            .map(|r| r.installation.installation_id)
            .collect::<Vec<_>>()
        || retried.iter().any(|r| !r.created_files.is_empty())
    {
        return Err("Import retry changed identities or republished manifests".into());
    }
    Ok(
        json!({"registered": imported.len(), "unresolved_skipped":unresolved,
        "groups_declined":draft.groups.len(),"manual_command_edits":0,
        "created_manifests":imported.iter().map(|r|r.created_files.len()).sum::<usize>(),
        "review_import_list_retry_ms":started.elapsed().as_secs_f64()*1000.0,
        "review_policy":"keep selected declarations; decline unresolved candidates and groups; no commands executed"}),
    )
}

#[test]
fn temporary_import_pipeline_preserves_selected_apps_and_skips_unknowns() {
    let temp = tempfile::tempdir().unwrap();
    let projects = temp.path().join("projects");
    fs::create_dir_all(projects.join("web")).unwrap();
    fs::create_dir_all(projects.join("api")).unwrap();
    fs::write(
        projects.join("web/package.json"),
        r#"{"scripts":{"dev":"touch must-not-run"}}"#,
    )
    .unwrap();
    fs::write(projects.join("api/requirements.txt"), "fastapi").unwrap();
    fs::create_dir_all(projects.join("todo")).unwrap();
    fs::write(
        projects.join("todo/package.json"),
        r#"{"scripts":{"dev":"vite"}}"#,
    )
    .unwrap();
    let draft = apphost::scan(&projects, 1).unwrap();
    let result = exercise_pipeline(&draft, &temp.path().join("data")).unwrap();
    assert_eq!(result["registered"], 1);
    assert_eq!(result["unresolved_skipped"], 2);
    assert!(!projects.join("web/must-not-run").exists());
    assert!(!projects.join(FILENAME).exists());
    let path = temp.path().to_owned();
    temp.close().unwrap();
    assert!(!path.exists());
}

fn cases(args: &Args, stats: &mut Stats) -> Result<(Vec<Case>, Option<Value>)> {
    let mut cases: Vec<_> = args
        .repositories
        .iter()
        .map(|repo| Case {
            repo: repo.clone(),
            negative_control: corpus()
                .iter()
                .any(|case| case.negative_control && case.repo.eq_ignore_ascii_case(repo)),
        })
        .collect();
    let mut search_info = None;
    if let Some(query) = &args.search {
        let limit = args.limit.to_string();
        let found = api(
            "search/repositories",
            &[
                ("q", query),
                ("sort", "stars"),
                ("order", "desc"),
                ("per_page", &limit),
            ],
            stats,
        )?;
        if found["incomplete_results"] != false {
            return Err("GitHub repository search is incomplete".into());
        }
        let items = found["items"].as_array().ok_or("Missing search items")?;
        for item in items {
            let repo = item["full_name"]
                .as_str()
                .ok_or("Missing repository name")?;
            repo_name(repo)?;
            if !cases.iter().any(|c| c.repo.eq_ignore_ascii_case(repo)) {
                cases.push(Case {
                    repo: repo.into(),
                    negative_control: false,
                });
            }
        }
        search_info = Some(
            json!({"query":query,"returned":items.len(),"total_count":found["total_count"],"sample_only":true,
                "repositories":items.iter().map(|i| json!({"repository":i["full_name"],"stars":i["stargazers_count"],"license":i["license"]["spdx_id"]})).collect::<Vec<_>>()}),
        );
    }
    if args.search.is_none() && cases.is_empty() {
        cases = corpus();
    }
    let mut seen = BTreeSet::new();
    cases.retain(|c| seen.insert(c.repo.to_ascii_lowercase()));
    if cases.is_empty() {
        return Err("No repositories matched; this is not a successful coverage test".into());
    }
    if cases.len() > 40 {
        return Err("At most 40 repositories per run".into());
    }
    for case in &cases {
        repo_name(&case.repo)?;
    }
    Ok((cases, search_info))
}

fn run(args: Args) -> Result<(Value, bool)> {
    if args.scan_parent && args.max_depth > 15 {
        return Err(
            "--scan-parent supports --max-depth up to 15 (one level is used by the collection)"
                .into(),
        );
    }
    let mut stats = Stats::default();
    let (cases, search) = cases(&args, &mut stats)?;
    let mut results = vec![];
    for (index, case) in cases.iter().enumerate() {
        eprintln!("[{}/{}] {}", index + 1, cases.len(), case.repo);
        match experiment(case, args.reference.as_deref(), args.subdir.as_deref(), args.max_depth as usize, args.exercise_import, args.scan_parent, &mut stats) {
            Ok(result) => results.push(result),
            Err(error) => results.push(json!({"repository":case.repo,"negative_control":case.negative_control,"outcome":"inconclusive","error":error})),
        }
    }
    let mut counts = BTreeMap::new();
    for result in &results {
        *counts
            .entry(result["outcome"].as_str().unwrap())
            .or_insert(0) += 1;
    }
    let unexpected = results
        .iter()
        .filter(|r| r["negative_control"] == true && r["selected"].as_u64().unwrap_or(0) > 0)
        .count();
    let incomplete = results.iter().any(|r| r["outcome"] == "inconclusive");
    let failed = incomplete || unexpected > 0;
    Ok((
        json!({"mode":"remote_metadata_no_clone","search":search,"summary":{"repositories":cases.len(),"outcomes":counts,"negative_controls_auto_selected":unexpected},"transfer":stats,
        "limitations":["No project commands were executed; auto_selected does not establish runnability.","Only tracked metadata is projected locally; dependencies, ignored files, permissions and runtime environment are unverified.","Search samples are ranked convenience samples, not a population accuracy estimate."],"results":results}),
        failed,
    ))
}

fn main() {
    match run(Args::parse()) {
        Ok((value, failed)) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            if failed {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{}", json!({"error":error}));
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parent_scan_preserves_depth_and_enables_root_pairs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        for dir in ["Frontend", "Backend"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(
            root.join("Frontend/package.json"),
            r#"{"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(root.join("Backend/requirements.txt"), "fastapi").unwrap();
        let direct = scan_projection(&root, 1, false).unwrap();
        let parent = scan_projection(&root, 1, true).unwrap();
        assert_eq!(direct.candidates.len(), parent.candidates.len());
        assert!(direct.groups.is_empty());
        assert_eq!(parent.groups.len(), 1);
        assert_eq!(
            parent.groups[0].candidate.path,
            root.canonicalize().unwrap()
        );
        assert!(
            scan_projection(&root, 0, true)
                .unwrap()
                .candidates
                .is_empty()
        );
        assert_eq!(
            exercise_pipeline(&parent, &temp.path().join("registry")).unwrap()["registered"],
            0
        );
        assert!(!root.join(FILENAME).exists());
    }

    #[test]
    fn parent_projection_downloads_real_nested_configuration_for_root_group() {
        let mut directory = entry("Frontend", 0);
        directory.kind = "tree".into();
        directory.mode = "040000".into();
        let tree = tree(vec![
            entry("pyproject.toml", 24),
            directory,
            entry("Frontend/package.json", 32),
        ]);
        let direct = plan_with_parent(&tree, 1, false).unwrap();
        assert_eq!(direct.files.len(), 1);
        let parent = plan_with_parent(&tree, 1, true).unwrap();
        assert_eq!(parent.files.len(), 2);
        assert!(
            parent
                .files
                .iter()
                .any(|f| f.path == "Frontend/package.json")
        );
        assert_eq!(plan_with_parent(&tree, 0, true).unwrap().files.len(), 1);
        let args =
            Args::try_parse_from(["github_scan", "--scan-parent", "--max-depth", "16"]).unwrap();
        assert!(run(args).unwrap_err().contains("up to 15"));
    }

    fn entry(path: &str, size: u64) -> Entry {
        Entry {
            path: path.into(),
            mode: "100644".into(),
            kind: "blob".into(),
            sha: "a".repeat(40),
            size: Some(size),
        }
    }
    fn tree(entries: Vec<Entry>) -> Tree {
        Tree {
            tree: entries,
            truncated: false,
        }
    }

    #[test]
    fn validates_repository_and_tree_paths() {
        for name in ["owner/repo", "a-b/repo.js"] {
            repo_name(name).unwrap();
        }
        for name in [
            "https://github.com/a/b",
            "a/b/c",
            "../repo",
            "a/b?x=1",
            "a/b\n",
        ] {
            assert!(repo_name(name).is_err());
        }
        for path in [
            "../escape",
            "/absolute",
            "a/../b",
            "a//b",
            "C:\\file",
            "a/./b",
        ] {
            assert!(relative(path).is_err());
        }
        assert_eq!(encode_segment("feature/a?b"), "feature%2Fa%3Fb");
    }

    #[test]
    fn parent_marker_keeps_nested_projects_as_hints_and_loads_proposed_frontend() {
        let planned = plan(
            &tree(vec![
                entry("package.json", 2),
                entry("web/package.json", 2),
                entry("node_modules/lib/package.json", 2),
            ]),
            3,
        )
        .unwrap();
        assert_eq!(planned.visible, vec![PathBuf::new()]);
        assert_eq!(planned.hidden, vec![PathBuf::from("web")]);
        assert_eq!(planned.files.len(), 1); // Scan root never receives an inferred group.
    }

    #[test]
    fn remote_projection_preserves_requirements_markerless_services_and_extra_docs() {
        for (directories, files) in [
            (vec!["frontend", "backend"], vec!["frontend/package.json"]),
            (
                vec!["backend"],
                vec!["package.json", "backend/requirements.txt"],
            ),
            (
                vec!["frontend", "backend", "docs"],
                vec![
                    "frontend/package.json",
                    "backend/requirements.txt",
                    "docs/package.json",
                ],
            ),
        ] {
            let mut entries: Vec<_> = directories
                .iter()
                .map(|path| {
                    let mut e = entry(path, 0);
                    e.kind = "tree".into();
                    e.mode = "040000".into();
                    e.size = None;
                    e
                })
                .collect();
            entries.extend(files.iter().map(|p| entry(p, 2)));
            let planned = plan(&tree(entries), 1).unwrap();
            let temp = tempfile::tempdir().unwrap();
            for directory in &planned.directories {
                fs::create_dir_all(temp.path().join(directory)).unwrap();
            }
            for file in &planned.files {
                let text = if file.path.ends_with("package.json") {
                    r#"{"scripts":{"dev":"vite"}}"#
                } else {
                    "fastapi\n"
                };
                fs::write(temp.path().join(&file.path), text).unwrap();
            }
            let root = temp.path().canonicalize().unwrap();
            let draft = apphost::scan(&root, 1).unwrap();
            let observed: BTreeSet<_> = draft
                .candidates
                .iter()
                .filter(|c| MARKERS.iter().any(|n| c.path.join(n).exists()))
                .map(|c| c.path.strip_prefix(&root).unwrap().to_owned())
                .collect();
            assert_eq!(observed, planned.visible.into_iter().collect());
            assert!(draft.groups.is_empty());
            assert!(!root.join(FILENAME).exists());
        }
    }

    #[test]
    fn obeys_depth_and_detects_unrecognized_runtime_markers() {
        let planned = plan(
            &tree(vec![
                entry("go.mod", 2),
                entry("web/package.json", 2),
                entry("deeper/ui/package.json", 2),
            ]),
            1,
        )
        .unwrap();
        assert_eq!(planned.visible, vec![PathBuf::from("web")]);
        assert_eq!(planned.unsupported_markers, vec!["go.mod"]);
    }

    #[test]
    fn refuses_truncation_unsafe_paths_symlink_metadata_and_missing_size() {
        assert!(
            plan(
                &Tree {
                    tree: vec![],
                    truncated: true
                },
                3
            )
            .is_err()
        );
        assert!(plan(&tree(vec![entry("../package.json", 2)]), 3).is_err());
        let mut file = entry("package.json", 2);
        file.mode = "120000".into();
        assert!(plan(&tree(vec![file.clone()]), 3).is_err());
        file.mode = "100644".into();
        file.size = None;
        assert!(plan(&tree(vec![file]), 3).is_err());
        assert!(
            plan(
                &tree(vec![entry("package.json", 2), entry("package.json", 2)]),
                3
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_incorrect_blob_metadata_and_base64() {
        let file = entry("package.json", 2);
        let valid = json!({"encoding":"base64","sha":file.sha,"size":2,"content":"e30=\n"});
        assert_eq!(blob_bytes(valid.clone(), &file).unwrap(), b"{}");
        for (field, value) in [
            ("sha", json!("b".repeat(40))),
            ("size", json!(3)),
            ("encoding", json!("utf8")),
            ("content", json!("!")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(blob_bytes(invalid, &file).is_err());
        }
    }

    #[test]
    fn oversized_metadata_is_retained_for_the_local_size_boundary() {
        let planned = plan(
            &tree(vec![
                entry("package.json", 2),
                entry("pnpm-lock.yaml", FILE_LIMIT + 10),
            ]),
            3,
        )
        .unwrap();
        assert_eq!(planned.files.len(), 2);
        assert!(planned.files.iter().any(|f| f.size.unwrap() > FILE_LIMIT));
    }

    #[test]
    fn tree_and_transfer_budgets_accept_the_boundary_and_reject_overflow() {
        let mut bounded = tree(
            (0..TREE_ENTRY_LIMIT)
                .map(|i| entry(&format!("file-{i}"), 1))
                .collect(),
        );
        assert!(plan(&bounded, 0).is_ok());
        bounded.tree.push(entry("one-too-many", 1));
        assert!(plan(&bounded, 0).err().unwrap().contains("20,000"));

        let mut bounded = tree(vec![
            entry("package.json", 2),
            entry("pnpm-lock.yaml", TRANSFER_LIMIT - 2),
        ]);
        assert!(plan(&bounded, 0).is_ok());
        bounded.tree[1].size = Some(TRANSFER_LIMIT - 1);
        assert!(plan(&bounded, 0).is_err());
    }

    #[test]
    fn invalid_subdirectory_scopes_fail_before_network_access() {
        let mut stats = Stats::default();
        for path in ["../escape".into(), "a//b".into(), vec!["a"; 17].join("/")] {
            assert!(scoped_tree_sha("a/b", &"a".repeat(40), Some(&path), &mut stats).is_err());
        }
        assert_eq!(stats.requests, 0);
        assert_eq!(
            scoped_tree_sha("a/b", &"a".repeat(40), None, &mut stats).unwrap(),
            "a".repeat(40)
        );
    }

    #[test]
    fn oversized_sentinel_exercises_the_same_local_read_boundary() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("package.json"),
            r#"{"name":"oversized-lock","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::File::create(temp.path().join("pnpm-lock.yaml"))
            .unwrap()
            .set_len(FINGERPRINT_LIMIT + 1)
            .unwrap();
        let draft = apphost::scan(temp.path(), 0).unwrap();
        assert!(!draft.candidates[0].selected);
        assert!(
            draft.candidates[0]
                .issues
                .iter()
                .any(|i| i.field == "pnpm-lock.yaml" && i.message.contains("16 MiB"))
        );
    }

    #[test]
    fn authoritative_manifest_does_not_require_unrelated_configuration_downloads() {
        let planned = plan(
            &tree(vec![
                entry(FILENAME, 100),
                entry("package.json", FILE_LIMIT + 1),
                entry("pyproject.toml", 20),
            ]),
            3,
        )
        .unwrap();
        assert_eq!(planned.files.len(), 1);
        assert_eq!(planned.files[0].path, FILENAME);
    }

    #[test]
    fn preserves_excluded_directories_for_manifest_cwd_validation() {
        let mut directory = entry("dist", 0);
        directory.kind = "tree".into();
        directory.mode = "040000".into();
        directory.size = None;
        let planned = plan(
            &tree(vec![
                directory,
                entry(FILENAME, 100),
                entry("dist/package.json", 2),
            ]),
            3,
        )
        .unwrap();
        assert_eq!(planned.directories, vec![PathBuf::from("dist")]);
        assert_eq!(planned.files.len(), 1);
    }

    #[test]
    fn only_transport_errors_are_retryable() {
        assert!(transient("GitHub request failed: EOF"));
        assert!(transient("HTTP 503 Service unavailable"));
        for error in [
            "HTTP 401 Unauthorized",
            "HTTP 403 rate limit exceeded",
            "HTTP 404 Not Found",
            "GitHub tree is truncated",
            "Invalid GitHub JSON",
        ] {
            assert!(!transient(error));
        }
    }

    #[test]
    fn explicit_control_selection_is_deduplicated_and_never_calls_network() {
        let args = Args::try_parse_from([
            "github_scan",
            "--repo",
            "encode/httpx",
            "--repo",
            "Encode/HTTPX",
        ])
        .unwrap();
        let mut stats = Stats::default();
        let (cases, search) = cases(&args, &mut stats).unwrap();
        assert_eq!(cases.len(), 1);
        assert!(cases[0].negative_control);
        assert_eq!(stats.requests, 0);
        assert!(search.is_none());
        assert!(Args::try_parse_from(["github_scan", "--reference", "main"]).is_err());
    }
}
