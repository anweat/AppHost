use super::{DiscoveryRule, Proposal, RuleContext, proposal};
use crate::{Error, Result};
pub struct PythonRule;
impl DiscoveryRule for PythonRule {
    fn id(&self) -> &str {
        "python"
    }
    fn markers(&self) -> &[&str] {
        &["pyproject.toml", "requirements.txt"]
    }
    fn evaluate(&self, context: &RuleContext<'_>) -> Result<Proposal> {
        // A console entry point is not evidence of a long-running app; keep the suggestion editable.
        if !context.has("pyproject.toml") {
            // Dependency lists identify a Python directory, never its ASGI module or runtime.
            return Ok(Proposal {
                name: context.name().into(),
                description: String::new(),
                services: vec![],
                requires_confirmation: true,
            });
        }
        infer_python(context).map(|parts| proposal(parts, true))
    }
}
fn infer_python(context: &RuleContext<'_>) -> Result<(String, String, Vec<String>)> {
    let bytes = context
        .file("pyproject.toml")
        .ok_or_else(|| Error::new("INVALID_INPUT", "pyproject.toml unavailable"))?;
    let text =
        std::str::from_utf8(bytes).map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
    let value: toml::Value =
        toml::from_str(text).map_err(|e| Error::new("INVALID_INPUT", e.to_string()))?;
    let project = value.get("project").ok_or_else(|| {
        Error::new(
            "VALIDATION_ERROR",
            "No project table; provide launch command",
        )
    })?;
    let scripts = project
        .get("scripts")
        .and_then(toml::Value::as_table)
        .filter(|s| s.len() == 1)
        .ok_or_else(|| {
            Error::new(
                "VALIDATION_ERROR",
                "Expected one project.scripts entry; provide launch command",
            )
        })?;
    let (name, target) = scripts.iter().next().unwrap();
    if target.as_str().is_none_or(|s| s.trim().is_empty()) {
        return Err(Error::new(
            "VALIDATION_ERROR",
            "Invalid Python script declaration",
        ));
    }
    Ok((
        project
            .get("name")
            .and_then(toml::Value::as_str)
            .unwrap_or(context.name())
            .into(),
        project
            .get("description")
            .and_then(toml::Value::as_str)
            .unwrap_or("")
            .into(),
        vec![name.clone()],
    ))
}
