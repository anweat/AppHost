//! Read-only presentation rules shared by CLI, agent and desktop callers.
use super::*;

fn action(id: &str, title: &str, argv: Vec<String>) -> Value {
    json!({"id":id,"title":title,"primary":true,"argv":argv,"requires_user_action":true})
}

/// Ownership is an observation, never authority to kill a process. The existing
/// stop-owner-and-start operation rechecks both identities and the actual port.
pub fn advice(failure: &Failure, consecutive: u64, managed_owner: Option<(Uuid, Uuid)>) -> Value {
    let code = failure.evidence.code.as_str();
    let repeated = consecutive > 1;
    let id = failure.installation_id.to_string();
    let (title, operation, caption, args) = match code {
        "PORT_IN_USE" if managed_owner.is_some() => {
            let (owner, instance) = managed_owner.unwrap();
            (
                "端口被另一个受管应用占用",
                "stop_owner_and_start",
                "停止占用应用并启动",
                vec![
                    "stop-owner-and-start".into(),
                    id.clone(),
                    "--owner-installation-id".into(),
                    owner.to_string(),
                    "--owner-instance-id".into(),
                    instance.to_string(),
                ],
            )
        }
        "PORT_IN_USE" if !repeated => (
            "端口已被占用",
            "retry",
            "重新启动",
            vec!["restart".into(), id.clone()],
        ),
        "PORT_IN_USE" | "READINESS_TIMEOUT" | "PORT_NOT_READY" => (
            "声明的端口尚不可用",
            "edit_manifest",
            "检查声明端口",
            vec!["manifest".into(), "read".into(), id.clone()],
        ),
        "DEPENDENCY_MISSING" => (
            "项目依赖可能缺失",
            "view_reason",
            "查看依赖证据",
            vec!["failure".into(), id.clone(), "--json".into()],
        ),
        "RUNTIME_MISMATCH" => (
            "运行环境不符合项目要求",
            "view_environment",
            "查看运行环境",
            vec!["environment".into(), "show".into()],
        ),
        "INVALID_COMMAND" => (
            "启动命令不可用",
            "edit_manifest",
            "检查命令与 PATH",
            vec!["manifest".into(), "read".into(), id.clone()],
        ),
        "REVIEW_REQUIRED" => (
            "启动声明需要重新确认",
            "review_manifest",
            "查看待确认声明",
            vec!["manifest".into(), "read".into(), id.clone()],
        ),
        "SOURCE_MISSING" => ("应用目录不可用", "relink", "重新关联项目目录", vec![]),
        "DAEMON_UNAVAILABLE" => (
            "后台服务不可用",
            "inspect_daemon",
            "检查后台服务",
            vec!["daemon".into(), "ping".into()],
        ),
        "EARLY_EXIT" | "UNKNOWN" if !repeated => (
            "服务提前退出",
            "retry",
            "完整重新启动",
            vec!["restart".into(), id.clone()],
        ),
        _ => (
            "应用需要处理",
            "preview_diagnostics",
            "预览诊断信息",
            vec!["failure".into(), id.clone(), "--export".into()],
        ),
    };
    let alive = failure.details["services"]
        .as_array()
        .is_some_and(|s| s.iter().any(|v| v["alive"] == true));
    let argv = if args.is_empty() {
        vec![]
    } else {
        std::iter::once("apphost".to_owned()).chain(args).collect()
    };
    json!({"code":code,"level":"blocking","title":title,"impact":if alive {"失败时其他服务仍运行；当前状态请查看 status"} else {"应用未能完成启动或继续提供服务"},"evidence":failure.evidence,"actions":[action(operation, caption, argv)],"clears_when":"用户显式停止或重新启动；历史失败证据继续保留","consecutive_same_cause":consecutive,"hints":[]})
}

/// Only inspect fixed filenames in the affected service's confined cwd. Missing
/// folders are hints, not proof of the failure class; nothing is installed here.
fn project_hints(root: &Path, service: &crate::manifest::Service) -> (Vec<Value>, Option<Value>) {
    let Ok(root) = root.canonicalize() else {
        return (vec![], None);
    };
    let Ok(cwd) = root.join(&service.cwd).canonicalize() else {
        return (vec![], None);
    };
    if !cwd.starts_with(&root) {
        return (vec![], None);
    }
    let file = |name: &str| std::fs::symlink_metadata(cwd.join(name)).is_ok_and(|m| m.is_file());
    let mut hints = vec![];
    let mut suggested: Vec<&str> = vec![];
    if file("package.json") {
        if !cwd.join("node_modules").exists() {
            hints.push(json!({"code":"NODE_MODULES_ABSENT","level":"suggestion","title":"未发现 node_modules；该提示不能证明依赖缺失"}));
        }
        let managers: Vec<_> = [
            ("package-lock.json", "npm"),
            ("pnpm-lock.yaml", "pnpm"),
            ("yarn.lock", "yarn"),
            ("bun.lock", "bun"),
            ("bun.lockb", "bun"),
        ]
        .into_iter()
        .filter(|(name, _)| file(name))
        .map(|(_, manager)| manager)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
        if managers.len() == 1 {
            suggested = vec![managers[0], "install"];
        }
        // An ambiguous or absent lockfile is not permission to choose a manager.
        if managers.len() != 1 {
            hints.push(json!({"code":"PACKAGE_MANAGER_UNCONFIRMED","level":"suggestion","title":"请按项目说明确认包管理器后安装依赖"}));
        }
    } else if file("pyproject.toml") || file("requirements.txt") {
        if !cwd.join(".venv").exists() {
            hints.push(json!({"code":"VENV_ABSENT","level":"suggestion","title":"未发现 .venv；项目也可能使用其他环境"}));
        }
        if file("uv.lock") {
            suggested = vec!["uv", "sync"];
        }
    }
    let command = (!suggested.is_empty()).then(|| json!({"id":"copy_dependency_command","title":"复制建议命令","primary":true,"argv":suggested,"cwd":service.cwd,"copy_only":true,"requires_user_action":true}));
    (hints, command)
}

pub(super) fn enrich(data: &Path, failure: &Failure, count: u64) -> Value {
    let details = [
        &failure.details["details"],
        &failure.details["operation_error"]["details"],
    ];
    let owner = details.into_iter().find_map(|v| {
        let owner = Uuid::parse_str(v["installation_id"].as_str()?).ok()?;
        let instance = Uuid::parse_str(v["instance_id"].as_str()?).ok()?;
        (owner != failure.installation_id).then_some((owner, instance))
    });
    let mut result = advice(failure, count, owner);
    if let Ok(installation) = crate::registry::registered(data, failure.installation_id) {
        let services = installation.confirmed_services.unwrap_or_default();
        let service = failure
            .service
            .as_ref()
            .and_then(|name| services.iter().find(|s| &s.name == name))
            .or_else(|| (services.len() == 1).then(|| &services[0]));
        if let Some(service) = service {
            let (hints, command) = project_hints(&installation.path, service);
            result["hints"] = json!(hints);
            if failure.evidence.code == "DEPENDENCY_MISSING"
                && count <= 1
                && let Some(command) = command
            {
                result["actions"] = json!([command]);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_primary_action_and_repeated_failures_change_the_next_step() {
        for code in [
            "PORT_IN_USE",
            "DEPENDENCY_MISSING",
            "INVALID_COMMAND",
            "RUNTIME_MISMATCH",
            "READINESS_TIMEOUT",
            "EARLY_EXIT",
            "UNKNOWN",
        ] {
            let mut failure = Failure::new(Uuid::new_v4(), code, 0);
            failure.evidence.code = code.into();
            let first = advice(&failure, 1, None);
            let again = advice(&failure, 2, None);
            assert_eq!(first["actions"].as_array().unwrap().len(), 1);
            if matches!(code, "PORT_IN_USE" | "EARLY_EXIT" | "UNKNOWN") {
                assert_ne!(first["actions"], again["actions"]);
            }
        }
    }
    #[test]
    fn hints_do_not_guess_manager_or_escape_service_root() {
        let t = tempfile::tempdir().unwrap();
        let mut service = crate::manifest::Service {
            name: "web".into(),
            cwd: ".".into(),
            command: vec!["npm".into()],
            port: None,
            env: Default::default(),
            startup_timeout_ms: None,
            stop_timeout_ms: None,
        };
        std::fs::write(t.path().join("package.json"), "{}").unwrap();
        assert!(project_hints(t.path(), &service).1.is_none());
        std::fs::write(t.path().join("package-lock.json"), "{}").unwrap();
        assert_eq!(
            project_hints(t.path(), &service).1.unwrap()["argv"],
            json!(["npm", "install"])
        );
        std::fs::write(t.path().join("yarn.lock"), "").unwrap();
        assert!(project_hints(t.path(), &service).1.is_none());
        service.cwd = "..".into();
        assert!(project_hints(t.path(), &service).0.is_empty());
    }
}
