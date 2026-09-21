use super::*;

pub(super) fn propose_groups(
    draft: &mut Draft,
    files: &RuleFiles,
    rules: &[&dyn DiscoveryRule],
    grouping: &dyn GroupingRule,
    visited: &BTreeSet<PathBuf>,
    remaining: &mut u64,
) -> Result<()> {
    let mut siblings: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    for (index, c) in draft.candidates.iter().enumerate() {
        if let Some(parent) = c.path.parent().filter(|p| p.starts_with(&draft.root)) {
            siblings.entry(parent.into()).or_default().push(index);
            // One wrapper such as Frontend/Vue is still a possible service member.
            // Reuse visited names only; do not walk outside the scan or infer a merge.
            if parent
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| grouping.is_service_directory(n))
                && let Some(root) = parent.parent().filter(|p| p.starts_with(&draft.root))
            {
                siblings.entry(root.into()).or_default().push(index);
            }
        }
    }
    for (root, all_indices) in siblings {
        let directories: BTreeSet<String> = visited
            .iter()
            .filter(|p| p.parent() == Some(root.as_path()))
            .filter_map(|p| p.file_name()?.to_str().map(String::from))
            .collect();
        let pairs = grouping.sibling_pairs(&directories);
        if pairs.len() > 16
            || pairs.iter().any(|pair| {
                pair[0] == pair[1]
                    || pair
                        .iter()
                        .any(|n| !valid_input_name(n) || !directories.contains(n))
            })
        {
            return Err(Error::new(
                "INVALID_INPUT",
                "Grouping rule returned invalid or unvisited members",
            ));
        }
        let member = |c: &Candidate| {
            pairs
                .iter()
                .flatten()
                .any(|n| c.path == root.join(n) || c.path.parent() == Some(root.join(n).as_path()))
        };
        if !all_indices.iter().any(|i| member(&draft.candidates[*i])) {
            continue;
        }
        // Pair evidence still requires a boundary decision when a collection guard prevents
        // grouping. Only inferred pair members are affected; unrelated siblings stay selected.
        for i in &all_indices {
            let c = &mut draft.candidates[*i];
            if member(c) && !authoritative(c) {
                c.selected = false;
                c.notes.push(Issue::new("group", "Possible service pair; importing this directory alone may omit another service. Confirm the App boundary. 如需登记为同一个 App 且没有合适的合并建议，请先确认 App 的共同根目录，在该目录编写 apphost.toml，用多个 [[services]] 和各自 cwd 声明前后端，然后重新扫描；不要将整个项目集合当作一个 App。"));
            }
        }
        // A scan root is never an inferred App root. Other runnable descendants also prevent
        // grouping, without manufacturing candidates for markerless directories.
        if all_indices.iter().any(|i| {
            member(&draft.candidates[*i])
                && draft.candidates[*i].path.parent() != Some(root.as_path())
        }) || pairs.len() != 1
            || root == draft.root
            || draft.candidates.iter().any(|c| {
                c.path.starts_with(&root)
                    && !member(c)
                    && c.manifest
                        .as_ref()
                        .is_some_and(|m| m.validate(&c.path).is_empty())
            })
            || all_indices
                .iter()
                .any(|i| member(&draft.candidates[*i]) && authoritative(&draft.candidates[*i]))
        {
            continue;
        }
        let pair = &pairs[0];
        let mut indices = Vec::new();
        for name in pair {
            let path = root.join(name);
            let index = match draft.candidates.iter().position(|c| c.path == path) {
                Some(index) => index,
                None => {
                    draft
                        .candidates
                        .push(candidate(&path, files, rules, remaining)?);
                    draft.candidates.len() - 1
                }
            };
            indices.push(index);
        }
        if indices.iter().any(|i| authoritative(&draft.candidates[*i])) {
            continue;
        }
        // Suggestions use the already bounded traversal, never a second recursive filesystem walk.
        // Keep original candidates until review explicitly accepts the common App boundary.
        let mut combined = candidate(&root, files, rules, remaining)?;
        // A declaration appearing since traversal still owns this boundary.
        if authoritative(&combined) || combined.manifest.is_none() {
            continue;
        }
        combined.issues.retain(|i| i.field != "rules");
        combined.selected = false;
        let mut manifest = combined.manifest.take().unwrap();
        let mut members = Vec::new();
        for i in &all_indices {
            if !indices.contains(i) {
                combined.nested_projects.push(
                    draft.candidates[*i]
                        .path
                        .strip_prefix(&root)
                        .unwrap()
                        .to_owned(),
                );
            }
        }
        if !combined.nested_projects.is_empty() {
            combined.notes.push(Issue::new("group", "Other descendant candidates remain in the draft; choose non-overlapping App boundaries before import"));
        }
        for index in indices {
            let child = &mut draft.candidates[index];
            let directory = child
                .path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            combined.source_dirs.push(directory.clone());
            members.push(child.path.clone());
            combined
                .evidence
                .extend(child.evidence.iter().map(|e| Evidence {
                    path: format!("{directory}/{}", e.path),
                    sha256: e.sha256.clone(),
                }));
            combined.issues.extend(
                child
                    .issues
                    .iter()
                    .map(|i| Issue::new(format!("{directory}.{}", i.field), &i.message)),
            );
            let services = child
                .manifest
                .as_ref()
                .map(|m| m.services.clone())
                .unwrap_or_default();
            if services.is_empty() {
                manifest.services.push(crate::manifest::Service {
                    startup_timeout_ms: None,
                    stop_timeout_ms: None,
                    name: directory.clone(),
                    cwd: directory.clone(),
                    command: vec![],
                    port: None,
                    env: BTreeMap::new(),
                });
            } else {
                let single = services.len() == 1;
                for mut service in services {
                    service.name = if single {
                        directory.clone()
                    } else {
                        format!("{directory}-{}", service.name)
                    };
                    service.cwd = if service.cwd == "." {
                        directory.clone()
                    } else {
                        format!("{directory}/{}", service.cwd)
                    };
                    manifest.services.push(service);
                }
            }
            child.selected = false;
            child.notes.push(Issue::new(
                "group",
                "Possible multi-service App; confirm common root or import separately",
            ));
        }
        combined.notes.push(Issue::new(
            "group",
            "Confirm these directories form one App before importing",
        ));
        combined.manifest = Some(manifest);
        draft.groups.push(GroupSuggestion {
            members,
            candidate: combined,
        });
    }
    Ok(())
}

/// Confirmation only edits the draft; registry writes still require import.
pub fn accept_group(draft: &mut Draft, index: usize) -> Result<()> {
    let group = draft
        .groups
        .get(index)
        .ok_or_else(|| Error::new("NOT_FOUND", "Unknown group suggestion"))?;
    if group.candidate.path == draft.root
        || !group.candidate.path.starts_with(&draft.root)
        || group.members.is_empty()
        || group.members.iter().collect::<BTreeSet<_>>().len() != group.members.len()
        || group.members.iter().any(|p| {
            p != &group.candidate.path && p.parent() != Some(group.candidate.path.as_path())
        })
        || draft
            .candidates
            .iter()
            .any(|c| c.path == group.candidate.path && !group.members.contains(&c.path))
    {
        return Err(Error::new("CONFLICT", "Invalid group boundary or members"));
    }
    if group
        .members
        .iter()
        .any(|path| !draft.candidates.iter().any(|c| &c.path == path))
    {
        return Err(Error::new(
            "CONFLICT",
            "Group members are no longer in this draft",
        ));
    }
    let mut group = draft.groups.remove(index);
    draft
        .candidates
        .retain(|c| !group.members.contains(&c.path));
    group.candidate.selected = false; // Launch declarations still need confirmation below.
    draft.candidates.push(group.candidate);
    draft.candidates.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(())
}

pub(super) fn propose_nested_groups(
    draft: &mut Draft,
    files: &RuleFiles,
    rules: &[&dyn DiscoveryRule],
    grouping: &dyn GroupingRule,
    remaining: &mut u64,
) -> Result<()> {
    for index in 0..draft.candidates.len() {
        let root = &draft.candidates[index];
        if root.path == draft.root
            || root.hints_truncated
            || root
                .evidence
                .iter()
                .any(|e| e.path == FILENAME && e.sha256.is_some())
            || draft.groups.iter().any(|g| g.members.contains(&root.path))
        {
            continue;
        }
        let children: Vec<_> = root
            .nested_projects
            .iter()
            .filter(|p| p.components().count() == 1)
            .cloned()
            .collect();
        if children.len() != 1
            || !children[0]
                .to_str()
                .is_some_and(|n| grouping.is_service_directory(n))
        {
            continue;
        }
        let child_path = root.path.join(&children[0]);
        // Existing declarations own their project boundary, even if their content is invalid.
        if fs::symlink_metadata(child_path.join(FILENAME)).is_ok() {
            continue;
        }
        let child = candidate(&child_path, files, rules, remaining)?;
        if child
            .evidence
            .iter()
            .any(|e| e.path == FILENAME && e.sha256.is_some())
        {
            continue;
        }
        let directory = children[0].to_str().unwrap().to_string();
        let mut combined = root.clone();
        let Some(manifest) = combined.manifest.as_mut() else {
            continue;
        };
        if manifest.services.is_empty() {
            manifest.services.push(crate::manifest::Service {
                startup_timeout_ms: None,
                stop_timeout_ms: None,
                name: "root".into(),
                cwd: ".".into(),
                command: vec![],
                port: None,
                env: BTreeMap::new(),
            });
        } else {
            for service in &mut manifest.services {
                service.name = format!("root-{}", service.name);
            }
        }
        let services = child.manifest.map(|m| m.services).unwrap_or_default();
        if services.is_empty() {
            manifest.services.push(crate::manifest::Service {
                startup_timeout_ms: None,
                stop_timeout_ms: None,
                name: directory.clone(),
                cwd: directory.clone(),
                command: vec![],
                port: None,
                env: BTreeMap::new(),
            });
        } else {
            let single = services.len() == 1;
            for mut service in services {
                service.name = if single {
                    directory.clone()
                } else {
                    format!("{directory}-{}", service.name)
                };
                service.cwd = if service.cwd == "." {
                    directory.clone()
                } else {
                    format!("{directory}/{}", service.cwd)
                };
                manifest.services.push(service);
            }
        }
        combined.selected = false;
        combined.source_dirs.push(directory.clone());
        combined
            .evidence
            .extend(child.evidence.into_iter().map(|e| Evidence {
                path: format!("{directory}/{}", e.path),
                sha256: e.sha256,
            }));
        combined.issues.extend(
            child
                .issues
                .into_iter()
                .map(|i| Issue::new(format!("{directory}.{}", i.field), i.message)),
        );
        combined.notes.push(Issue::new(
            "group",
            "Nested directory may be an App service or documentation; confirm before grouping",
        ));
        let members = vec![root.path.clone()];
        draft.candidates[index].selected = false;
        draft.candidates[index].notes.push(Issue::new(
            "group",
            "Root and nested service directory require an explicit App boundary decision",
        ));
        draft.groups.push(GroupSuggestion {
            members,
            candidate: combined,
        });
    }
    Ok(())
}
