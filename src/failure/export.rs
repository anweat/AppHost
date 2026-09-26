use super::*;

pub(super) fn sanitize(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(s) => *s = redact(s, secrets),
        Value::Array(items) => items.iter_mut().for_each(|v| sanitize(v, secrets)),
        Value::Object(items) => items.values_mut().for_each(|v| sanitize(v, secrets)),
        _ => {}
    }
}

/// Returns a preview, never writes a file, copies a clipboard, or contacts an AI.
/// The failed run did not persist argv; current confirmed commands are explicitly
/// labelled so a later manifest edit cannot masquerade as historical evidence.
pub fn export(data: &Path, id: Uuid, failure_id: Option<Uuid>) -> Result<Value> {
    let report = read(data, id, failure_id)?;
    let installation = crate::registry::registered(data, id).ok();
    let services = installation
        .as_ref()
        .and_then(|i| i.confirmed_services.as_ref());
    let secrets: Vec<String> = services
        .into_iter()
        .flatten()
        .flat_map(|s| s.env.values().cloned())
        .collect();
    let commands: Vec<_> = services
        .into_iter()
        .flatten()
        .map(|s| json!({"service":s.name,"cwd":s.cwd,"argv":s.command,"declared_port":s.port}))
        .collect();
    let environment = crate::environment::load(data).map(|baseline| json!({"path":baseline.path,"capture":baseline.capture,"runtime_versions":"not_observed"})).unwrap_or_else(|e|json!({"unavailable":e.code,"runtime_versions":"not_observed"}));
    let mut sections = json!([
        {"kind":"application","value":{"installation_id":id,"label":installation.as_ref().map(|i| &i.human_label)}},
        {"kind":"command","value":{"source":"current_confirmed_declaration_not_historical_argv","services":commands}},
        {"kind":"environment","value":environment},
        {"kind":"category","value":report["advice"]},
        {"kind":"evidence","value":{"failure_id":report["failure"]["failure_id"],"at_ms":report["failure"]["at_ms"],"evidence":report["failure"]["evidence"],"details":report["failure"]["details"],"window":report["failure"]["window"],"gaps":report["failure"]["gaps"],"collecting":report["failure"]["collecting"],"details_expired":report["details_expired"]}},
        {"kind":"output_tail","value":{"stderr":report["failure"]["stderr_tail"],"output":report["failure"]["output"]}},
        {"kind":"last_success","value":{"at_ms":report["history"]["last_success_at_ms"],"changes":report["changes_since_success"]}}
    ]);
    sanitize(&mut sections, &secrets);
    let mut preview = json!({"schema_version":1,"preview_required":true,"redaction":"best_effort_review_before_sharing","sections":sections,"omitted":["environment_variable_values","historical_argv_not_recorded","runtime_versions_not_probed","dependency_fingerprints","browser_console_and_network","unretained_or_outside_window_output"],"redaction_context_available":installation.is_some()});
    // Redaction and a current declaration may exceed the stored incident budget.
    // Preserve section order and explicitly mark omitted large sections.
    for index in [5, 4, 1, 6, 2, 3] {
        if encode(&preview).is_ok() {
            break;
        }
        preview["sections"][index]["value"] =
            json!({"truncated":true,"reason":"export_byte_limit"});
        if !preview["omitted"]
            .as_array()
            .unwrap()
            .contains(&json!("export_byte_limit"))
        {
            preview["omitted"]
                .as_array_mut()
                .unwrap()
                .push(json!("export_byte_limit"));
        }
    }
    encode(&preview)?;
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_export_has_ordered_sections_and_does_not_create_storage() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("absent");
        let id = Uuid::new_v4();
        assert_eq!(export(&data, id, None).unwrap_err().code, "NOT_FOUND");
        assert!(!data.exists());
        let failure = Failure::new(id, "EARLY_EXIT", 1);
        store::save(&data, &failure, 1).unwrap();
        let report = export(&data, id, None).unwrap();
        assert_eq!(
            report["sections"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "application",
                "command",
                "environment",
                "category",
                "evidence",
                "output_tail",
                "last_success"
            ]
        );
        assert_eq!(report["preview_required"], true);
    }
    #[test]
    fn export_redacts_confirmed_values_and_caps_redaction_expansion() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let root = t.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let manifest: crate::Manifest = serde_json::from_value(json!({"schema_version":1,"project_id":Uuid::new_v4(),"name":"fixture","services":[{"name":"web","command":["unused","private-value"],"env":{"EXAMPLE":"private-value","LETTER":"Z"}}]})).unwrap();
        std::fs::write(root.join("apphost.toml"), manifest.to_toml().unwrap()).unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        std::fs::write(root.join("package-lock.json"), "{}").unwrap();
        let id = crate::import(&data, &crate::scan(&root, 0).unwrap()).unwrap()[0].installation_id;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let mut failure = Failure::new(id, "DEPENDENCY_MISSING", now);
        failure.stderr_tail = "private-value".into();
        store::save(&data, &failure, now).unwrap();
        let first = read(&data, id, None).unwrap();
        assert_eq!(
            first["advice"]["actions"][0]["argv"],
            json!(["npm", "install"])
        );
        let preview = export(&data, id, None).unwrap();
        assert!(!preview.to_string().contains("private-value"));
        assert_eq!(
            preview["sections"][1]["value"]["source"],
            "current_confirmed_declaration_not_historical_argv"
        );
        failure.failure_id = Uuid::new_v4();
        failure.at_ms += 1;
        failure.stderr_tail = "Z".repeat(200000);
        store::save(&data, &failure, now + 1).unwrap();
        let again = read(&data, id, None).unwrap();
        assert_eq!(again["advice"]["actions"][0]["id"], "view_reason");
        let preview = export(&data, id, None).unwrap();
        assert!(serde_json::to_vec(&preview).unwrap().len() <= 900 * 1024);
        assert_eq!(preview["sections"][5]["value"]["truncated"], true);
    }
    #[test]
    fn recursive_preview_redacts_known_values_and_paths() {
        let mut v = json!({"argv":["echo","private-value"],"nested":{"output":"at /Users/alice/main.py\ntoken=abcd"}});
        sanitize(&mut v, &["private-value".into()]);
        for secret in ["private-value", "alice", "abcd"] {
            assert!(!v.to_string().contains(secret));
        }
    }
}
