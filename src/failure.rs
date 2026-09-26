//! Independent, bounded failure evidence. Reading this module never starts an App.
use crate::{Error, Result, storage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub code: String,
    pub rule_id: String,
    pub inferred: bool,
    pub excerpt: Option<String>,
    pub line: Option<usize>,
}

// Ordered, small evidence rules: a concrete diagnostic wins over a generic exit.
// A successful version query is not available here, so versions are never guessed.
const RULES: &[(&str, &str, &[&str])] = &[
    (
        "PORT_IN_USE",
        "stderr.address_in_use",
        &["EADDRINUSE", "Address already in use"],
    ),
    (
        "DEPENDENCY_MISSING",
        "stderr.missing_dependency",
        &[
            "ModuleNotFoundError:",
            "Cannot find module",
            "ERR_MODULE_NOT_FOUND",
        ],
    ),
    (
        "RUNTIME_MISMATCH",
        "stderr.unsupported_runtime",
        &[
            "EBADENGINE",
            "Unsupported engine",
            "The engine \"node\" is incompatible",
        ],
    ),
];

// Restrict shell diagnostics to common project-local executables. An arbitrary
// missing executable is not evidence that installing project dependencies helps.
fn missing_project_tool(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    let tool = [": command not found", ": not found"]
        .iter()
        .find_map(|marker| {
            line.split_once(marker)
                .and_then(|(prefix, _)| prefix.split_whitespace().last())
        });
    tool.is_some_and(|tool| {
        [
            "vite",
            "next",
            "tsx",
            "nuxt",
            "webpack",
            "astro",
            "vue-cli-service",
        ]
        .contains(&tool)
    })
}
fn vite_port_conflict(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    line.split("port ").skip(1).any(|suffix| {
        suffix
            .split_once(" is already in use")
            .is_some_and(|(port, _)| port.parse::<u16>().is_ok_and(|port| port != 0))
    })
}

pub fn classify(cause: &str, phase: &str, stderr: &str) -> Evidence {
    if matches!(
        cause,
        "SERVICE_EXITED" | "READINESS_TIMEOUT" | "SPAWN_FAILED"
    ) {
        for (code, rule, patterns) in RULES {
            if let Some((line, text)) = stderr.lines().enumerate().find(|(_, line)| {
                patterns.iter().any(|pattern| {
                    line.to_ascii_lowercase()
                        .contains(&pattern.to_ascii_lowercase())
                        || (*code == "PORT_IN_USE" && vite_port_conflict(line))
                        || (*code == "DEPENDENCY_MISSING" && missing_project_tool(line))
                })
            }) {
                return Evidence {
                    code: (*code).into(),
                    rule_id: (*rule).into(),
                    inferred: true,
                    excerpt: Some(text.chars().take(1024).collect()),
                    line: Some(line + 1),
                };
            }
        }
    }
    let code = match cause {
        "SERVICE_EXITED" if phase == "startup" => "EARLY_EXIT",
        "SERVICE_EXITED" | "SPAWN_FAILED" => "UNKNOWN",
        other => other,
    };
    Evidence {
        code: code.into(),
        rule_id: format!("runtime.{}", cause.to_ascii_lowercase()),
        inferred: false,
        excerpt: None,
        line: None,
    }
}

/// Best effort export redaction, not a guarantee that arbitrary program output is safe.
/// Known env values are removed before applying token/path heuristics.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_owned();
    let mut secrets: Vec<_> = secrets.iter().filter(|v| !v.is_empty()).collect();
    secrets.sort_by_key(|v| std::cmp::Reverse(v.len()));
    for secret in secrets {
        text = text.replace(secret, "[redacted]");
    }
    text.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "authorization:",
                "bearer ",
                "password",
                "passwd",
                "secret",
                "api_key",
                "apikey",
                "access_token",
                "token=",
                "token\"",
                "private key",
            ]
            .iter()
            .any(|marker| lower.contains(marker))
            {
                return "[redacted sensitive line]".to_owned();
            }
            line.split_inclusive(char::is_whitespace)
                .map(|word| {
                    if word.contains("/Users/")
                        || word.contains("/home/")
                        || word.contains("\\Users\\")
                    {
                        format!(
                            "[local path]{}",
                            if word.ends_with(char::is_whitespace) {
                                " "
                            } else {
                                ""
                            }
                        )
                    } else {
                        word.to_owned()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn invalid(message: &str) -> Error {
    Error::new("FAILURE_INVALID", message)
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|e| invalid(&e.to_string()))?;
    // Leave room for the RPC envelope rather than filling its complete 1 MiB budget.
    if bytes.len() > 900 * 1024 {
        return Err(Error::new(
            "FAILURE_LIMIT",
            "Failure evidence exceeds its byte budget",
        ));
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub schema_version: u32,
    pub failure_id: Uuid,
    pub installation_id: Uuid,
    pub instance_id: Option<Uuid>,
    pub service: Option<String>,
    pub phase: String,
    pub at_ms: u128,
    pub cause_code: String,
    pub evidence: Evidence,
    pub details: Value,
    pub output: Vec<Output>,
    pub stderr_tail: String,
    pub window: Value,
    pub collecting: bool,
    pub gaps: Vec<String>,
    pub redaction: String,
}

/// Diagnostic facts captured when preparing the actual launch, never commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchContext {
    pub branch: Option<String>,
    pub launch_sha256: String,
    pub manifest_source: String,
    pub confirmed_actor: Option<crate::journal::Actor>,
    pub environment_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunObservation {
    pub instance_id: Uuid,
    pub started_at_ms: u128,
    pub success_at_ms: Option<u128>,
    pub finished_at_ms: Option<u128>,
    pub result: Option<String>,
    pub context: Option<LaunchContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub snapshot: String,
    pub sequence: u64,
    pub observed_ms: u128,
    pub service: String,
    pub stream: String,
    pub text: String,
}

impl Failure {
    pub fn new(installation_id: Uuid, cause: &str, at_ms: u128) -> Self {
        Self {
            schema_version: 1,
            failure_id: Uuid::new_v4(),
            installation_id,
            instance_id: None,
            service: None,
            phase: "preflight".into(),
            at_ms,
            cause_code: cause.into(),
            evidence: classify(cause, "preflight", ""),
            details: Value::Null,
            output: vec![],
            stderr_tail: String::new(),
            window: json!({"from_ms":at_ms.saturating_sub(30000),"to_ms":at_ms.saturating_add(10000)}),
            collecting: false,
            gaps: vec![],
            redaction: "best_effort; review before sharing".into(),
        }
    }
}

mod advice;
mod store;
pub use advice::advice;
mod export;
pub use export::export;
pub(crate) use store::observe_run;
pub use store::read_history;
pub fn read(data: &Path, id: Uuid, failure_id: Option<Uuid>) -> Result<Value> {
    let mut value = store::read(data, id, failure_id)?;
    let saved = value["history"]["last_success_context"].clone();
    let comparison = match serde_json::from_value::<LaunchContext>(saved) {
        Ok(previous) => {
            match crate::runtime::LaunchPlan::current_context(data, id) {
                Ok(current) => {
                    let mut changes = Vec::new();
                    if previous.branch.is_some()
                        && current.branch.is_some()
                        && previous.branch != current.branch
                    {
                        changes.push(json!({"field":"branch","before":previous.branch,"after":current.branch}));
                    }
                    for (field, old, new) in [
                        (
                            "launch_sha256",
                            &previous.launch_sha256,
                            &current.launch_sha256,
                        ),
                        (
                            "manifest_source",
                            &previous.manifest_source,
                            &current.manifest_source,
                        ),
                        (
                            "environment_sha256",
                            &previous.environment_sha256,
                            &current.environment_sha256,
                        ),
                    ] {
                        if old != new {
                            changes.push(json!({"field":field,"before":old,"after":new}));
                        }
                    }
                    json!({"available":true,"changes":changes,"branch_known":previous.branch.is_some()&&current.branch.is_some(),"current":current})
                }
                Err(e) => json!({"available":false,"reason":e.code}),
            }
        }
        Err(_) => json!({"available":false,"reason":"NO_SUCCESS_BASELINE"}),
    };
    value["changes_since_success"] = comparison;
    if let Ok(failure) = serde_json::from_value::<Failure>(value["failure"].clone()) {
        value["advice"] = advice::enrich(
            data,
            &failure,
            value["consecutive_same_cause"].as_u64().unwrap_or(1),
        );
    }
    Ok(value)
}
pub(crate) mod capture;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concrete_evidence_beats_early_exit_but_unrelated_logs_do_not_override_identity_failure() {
        let e = classify(
            "SERVICE_EXITED",
            "startup",
            "trace\nModuleNotFoundError: No module named 'gradio'\n",
        );
        assert_eq!(e.code, "DEPENDENCY_MISSING");
        assert_eq!(e.line, Some(2));
        assert!(e.inferred);
        assert_eq!(classify("SERVICE_EXITED", "startup", "").code, "EARLY_EXIT");
        assert_eq!(classify("SERVICE_EXITED", "runtime", "").code, "UNKNOWN");
        assert_eq!(
            classify("MANAGEMENT_UNCERTAIN", "runtime", "EADDRINUSE").code,
            "MANAGEMENT_UNCERTAIN"
        );
        assert_eq!(
            classify("READINESS_TIMEOUT", "startup", "v22.0.0").code,
            "READINESS_TIMEOUT"
        );
    }
    #[test]
    fn common_diagnostics_are_case_insensitive_and_keep_structured_failures() {
        for message in [
            "address already in use",
            "Port 5173 is already in use",
            "Eaddrinuse",
        ] {
            assert_eq!(
                classify("SERVICE_EXITED", "startup", message).code,
                "PORT_IN_USE"
            );
        }
        for message in ["sh: vite: command not found", "sh: 1: next: not found"] {
            assert_eq!(
                classify("SERVICE_EXITED", "startup", message).code,
                "DEPENDENCY_MISSING"
            );
        }
        assert_eq!(
            classify("SERVICE_EXITED", "startup", "sh: invite: command not found").code,
            "EARLY_EXIT"
        );
        assert_eq!(
            classify("SERVICE_EXITED", "startup", "sh: custom-tool: not found").code,
            "EARLY_EXIT"
        );
        assert_eq!(
            classify("INVALID_COMMAND", "preflight", "ModuleNotFoundError:").code,
            "INVALID_COMMAND"
        );
    }
    #[test]
    fn redaction_handles_environment_tokens_paths_and_utf8() {
        let output = redact(
            "密钥: long-private-value\nGET https://x/?token=abc\nAuthorization: Bearer xyz\nat /Users/alice/project/main.ts\nModuleNotFoundError: gradio",
            &["long-private-value".into()],
        );
        for secret in ["long-private-value", "abc", "xyz", "alice"] {
            assert!(!output.contains(secret));
        }
        assert!(output.contains("ModuleNotFoundError"));
    }
}
