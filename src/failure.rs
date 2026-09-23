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

pub fn classify(cause: &str, phase: &str, stderr: &str) -> Evidence {
    if matches!(
        cause,
        "SERVICE_EXITED" | "READINESS_TIMEOUT" | "SPAWN_FAILED"
    ) {
        for (code, rule, patterns) in RULES {
            if let Some((line, text)) = stderr
                .lines()
                .enumerate()
                .find(|(_, line)| patterns.iter().any(|pattern| line.contains(pattern)))
            {
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
#[cfg(any(target_os = "macos", test))]
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

mod store;
pub use store::read;
#[cfg(target_os = "macos")]
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
