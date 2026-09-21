use super::{DiscoveryRule, Proposal, RuleContext, proposal};
use crate::{Error, Result};
use serde_json::Value;
pub struct NodeRule;
impl DiscoveryRule for NodeRule {
    fn id(&self) -> &str {
        "node"
    }
    fn markers(&self) -> &[&str] {
        &["package.json"]
    }
    fn evaluate(&self, context: &RuleContext<'_>) -> Result<Proposal> {
        let value: Value = serde_json::from_slice(
            context
                .file("package.json")
                .ok_or_else(|| Error::new("INVALID_INPUT", "package.json unavailable"))?,
        )
        .map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
        let parts = infer_node(context, &value)?;
        let script = &value["scripts"][&parts.2[2]];
        let library = value.get("exports").is_some()
            || (value.get("main").is_some() && value.get("files").is_some());
        Ok(proposal(
            parts,
            library || build_watch(script.as_str().unwrap_or("")),
        ))
    }
}
// These metadata clues do not prove that a package is a library. Preserve the
// proposed command and require confirmation instead of excluding the candidate.
fn build_watch(script: &str) -> bool {
    let words: Vec<_> = script.split_ascii_whitespace().collect();
    // Deliberately not a shell parser. Only flag direct, known build tools; a
    // dev server using --watch must not be classified as a build-only watcher.
    matches!(
        words.first().copied(),
        Some("tsc" | "tsdown" | "tsup" | "rollup" | "esbuild")
    ) && words
        .iter()
        .any(|w| matches!(*w, "-w" | "--watch") || w.starts_with("--watch="))
}
fn infer_node(context: &RuleContext<'_>, value: &Value) -> Result<(String, String, Vec<String>)> {
    let scripts = value
        .get("scripts")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::new("VALIDATION_ERROR", "No scripts object in package.json"))?;
    let script = ["dev", "start"]
        .into_iter()
        .find(|s| {
            scripts
                .get(*s)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        })
        .ok_or_else(|| {
            Error::new(
                "VALIDATION_ERROR",
                "No dev/start script; provide launch command",
            )
        })?;
    let mut managers = Vec::new();
    for (file, manager) in [
        ("package-lock.json", "npm"),
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
    ] {
        if context.has(file) && !managers.contains(&manager) {
            managers.push(manager);
        }
    }
    if let Some(declared) = value.get("packageManager") {
        let manager = declared
            .as_str()
            .and_then(|s| s.split('@').next())
            .filter(|s| ["npm", "pnpm", "yarn", "bun"].contains(s))
            .ok_or_else(|| {
                Error::new(
                    "CONFLICT",
                    "Unsupported packageManager; provide launch command",
                )
            })?;
        if managers.iter().any(|m| *m != manager) {
            return Err(Error::new(
                "CONFLICT",
                "packageManager conflicts with lockfiles",
            ));
        }
        managers = vec![manager];
    }
    if managers.len() > 1 {
        return Err(Error::new(
            "CONFLICT",
            "Multiple package manager lockfiles; provide launch command",
        ));
    }
    Ok((
        value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(context.name())
            .into(),
        value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        vec![
            managers.first().unwrap_or(&"npm").to_string(),
            "run".into(),
            script.into(),
        ],
    ))
}
