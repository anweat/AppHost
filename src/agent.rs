//! Instructions travel with the installed binary, including offline installations.
use serde_json::{Value, json};
pub fn guide() -> Value {
    json!({"version":env!("CARGO_PKG_VERSION"),"manifest":include_str!("../agent/skills/manifest.md"),"troubleshoot":include_str!("../agent/skills/troubleshoot.md"),"project_prompt":include_str!("../agent/project-prompt.md")})
}
