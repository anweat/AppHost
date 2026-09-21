use crate::{
    Error, Result,
    discovery::{Draft, accept_group},
    manifest::Service,
};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
};

fn line(input: &mut impl BufRead, output: &mut impl Write, prompt: &str) -> Result<String> {
    write!(output, "{prompt}")
        .and_then(|_| output.flush())
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    let mut value = String::new();
    // EOF is not consent. A cancelled review never publishes its output draft.
    let count = std::io::Read::take(&mut *input, 16 * 1024 + 1)
        .read_line(&mut value)
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    if count == 0 {
        return Err(Error::new(
            "INVALID_INPUT",
            "Review cancelled at end of input",
        ));
    }
    if value.len() > 16 * 1024 {
        return Err(Error::new("INVALID_INPUT", "Review answer exceeds 16 KiB"));
    }
    Ok(value.trim().to_string())
}

/// User-controlled review only mutates an in-memory draft. All project writes stay in import.
pub fn review(draft: &mut Draft, input: &mut impl BufRead, output: &mut impl Write) -> Result<()> {
    review_with_descriptions(draft, input, output, false)
}

pub fn review_with_descriptions(
    draft: &mut Draft,
    input: &mut impl BufRead,
    output: &mut impl Write,
    describe: bool,
) -> Result<()> {
    let mut accepted = Vec::new();
    let mut remaining = crate::discovery::MAX_READ_BYTES;
    while !draft.groups.is_empty() {
        let group = &mut draft.groups[0];
        group
            .candidate
            .refresh_launch_notes_budgeted(&mut remaining)?;
        show_notes(&group.candidate, output, false)?;
        show_declarations(&group.candidate, "合并建议", output)?;
        let prompt = format!(
            "合为一个 App：{}\n接受合并？[y/N] ",
            group.candidate.path.display(),
        );
        let response = line(input, output, &prompt)?;
        if response.eq_ignore_ascii_case("y") || response.eq_ignore_ascii_case("yes") {
            accepted.push(group.candidate.path.clone());
            accept_group(draft, 0)?;
        } else if response.is_empty()
            || response.eq_ignore_ascii_case("n")
            || response.eq_ignore_ascii_case("no")
        {
            draft.groups.remove(0);
        } else {
            return Err(Error::new("INVALID_INPUT", "Expected y or n"));
        }
    }
    for candidate in &mut draft.candidates {
        candidate.refresh_launch_notes_budgeted(&mut remaining)?;
        show_notes(candidate, output, false)?;
        if candidate.selected {
            show_declarations(candidate, "已选", output)?;
            describe_candidate(candidate, input, output, describe)?;
            continue;
        }
        // A fresh review of an unselected item replaces any earlier review, including refusal.
        candidate.reviewed_launch_sha256 = None;
        show_declarations(candidate, "待确认", output)?;
        let read_only = has_project_manifest(candidate);
        if read_only {
            writeln!(
                output,
                "已有 apphost.toml，仅确认当前声明；如需修改，请直接编辑 apphost.toml 后重新扫描。"
            )
            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
        }
        let Some(manifest) = candidate.manifest.as_mut() else {
            writeln!(
                output,
                "跳过 {}：请先修正已有清单",
                candidate.path.display()
            )
            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
            continue;
        };
        if !accepted.contains(&candidate.path) {
            let prompt = format!(
                "{}\n确认这个 App 的启动方式？[y/N] ",
                candidate.path.display(),
            );
            let response = line(input, output, &prompt)?;
            if response.is_empty()
                || response.eq_ignore_ascii_case("n")
                || response.eq_ignore_ascii_case("no")
            {
                continue;
            }
            if !response.eq_ignore_ascii_case("y") && !response.eq_ignore_ascii_case("yes") {
                return Err(Error::new("INVALID_INPUT", "Expected y or n"));
            }
        }
        if !read_only && manifest.services.is_empty() {
            manifest.services.push(Service {
                startup_timeout_ms: None,
                stop_timeout_ms: None,
                name: "app".into(),
                command: vec![],
                cwd: ".".into(),
                port: None,
                env: BTreeMap::new(),
            });
        }
        for service in &mut manifest.services {
            // Import never overwrites a project manifest. Review must not offer edits that
            // would inevitably conflict with it, even when --all requests another confirmation.
            if read_only {
                break;
            }
            // The displayed group declaration was already confirmed. Only incomplete commands
            // require another answer; no per-App wizard for the previously selected batch.
            if accepted.contains(&candidate.path) && !service.command.is_empty() {
                continue;
            }
            let prompt = format!(
                "{} (cwd={}) 命令 {:?}；回车保留，或输入参数数组，如 [\"uv\",\"run\",\"uvicorn\",\"main:app\"]：",
                service.name, service.cwd, service.command
            );
            let response = line(input, output, &prompt)?;
            if !response.is_empty() {
                service.command = serde_json::from_str::<Vec<String>>(&response).map_err(|e| {
                    Error::new(
                        "INVALID_INPUT",
                        format!("Expected an array of command arguments: {e}"),
                    )
                })?;
            }
        }
        let issues = manifest.validate(&candidate.path);
        if !issues.is_empty() {
            return Err(Error::new(
                "VALIDATION_ERROR",
                "Launch declarations still need correction",
            )
            .details(serde_json::json!(issues)));
        }
        candidate.selected = true;
        candidate.reviewed_launch_sha256 = Some(manifest.launch_sha256());
        describe_candidate(candidate, input, output, describe)?;
        candidate.refresh_launch_notes_budgeted(&mut remaining)?;
        // Only launch diagnostics can change during command review. Do not repeat the
        // boundary notes and old/new declaration comparison a second time.
        show_notes(candidate, output, true)?;
    }
    Ok(())
}

fn show_declarations(
    candidate: &crate::discovery::Candidate,
    status: &str,
    output: &mut impl Write,
) -> Result<()> {
    if let Some(manifest) = &candidate.manifest {
        if manifest.services.is_empty() {
            writeln!(
                output,
                "{status} {}  （待填写启动声明）",
                candidate.human_label
            )
            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
        }
        if manifest.schema_version == 1 {
            writeln!(output, "  入口：{}", manifest.open_summary())
                .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
        }
        for service in &manifest.services {
            writeln!(
                output,
                "{status} {} / {}  {}",
                candidate.human_label,
                service.name,
                service.launch_summary()
            )
            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
        }
    }
    Ok(())
}

fn describe_candidate(
    candidate: &mut crate::discovery::Candidate,
    input: &mut impl BufRead,
    output: &mut impl Write,
    describe: bool,
) -> Result<()> {
    // Project-owned manifests are not overwritten by review/import, including descriptions.
    if !describe || has_project_manifest(candidate) {
        return Ok(());
    }
    if let Some(manifest) = candidate
        .manifest
        .as_mut()
        .filter(|m| m.description.is_empty())
    {
        manifest.description = line(input, output, "这个 App 用来做什么？（回车跳过）：")?;
        if manifest.description.contains('\0') {
            return Err(Error::new(
                "INVALID_INPUT",
                "Description cannot contain NUL",
            ));
        }
    }
    Ok(())
}

fn has_project_manifest(candidate: &crate::discovery::Candidate) -> bool {
    candidate
        .evidence
        .iter()
        .any(|e| e.path == crate::manifest::FILENAME && e.sha256.is_some())
        // A manifest may also have appeared since scan (including a retried import).
        // Import still verifies its contents and evidence before writing anything.
        || std::fs::symlink_metadata(candidate.path.join(crate::manifest::FILENAME)).is_ok()
}

fn show_notes(
    candidate: &crate::discovery::Candidate,
    output: &mut impl Write,
    launch_only: bool,
) -> Result<()> {
    for note in candidate
        .notes
        .iter()
        .filter(|n| !launch_only || n.field.starts_with("launch_check."))
    {
        writeln!(
            output,
            "{} [{}] {}",
            candidate.path.display(),
            note.field,
            note.message
        )
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    }
    Ok(())
}
