use apphost::{Error, Result};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::{env, path::PathBuf};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    version,
    about = "AppHost: inspect and register local apps; explicitly configure their runtime environment"
)]
struct Cli {
    /// AppHost-owned registry directory (also APPHOST_DATA_DIR).
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read the stable browser entry without starting the daemon or App.
    Url { installation_id: Uuid },
    /// Read or explicitly configure the local browser gateway.
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
    #[command(hide = true)]
    InternalCollect { root: PathBuf, descriptors: String },
    /// Start a confirmed installation (starts the daemon if needed).
    Start { installation_id: Uuid },
    /// Stop the specified managed owner, then recheck and start the target.
    StopOwnerAndStart {
        installation_id: Uuid,
        #[arg(long)]
        owner_installation_id: Uuid,
        #[arg(long)]
        owner_instance_id: Uuid,
    },
    /// Stop all verified process groups belonging to an installation.
    Stop { installation_id: Uuid },
    /// Stop the old instance before preparing and starting a new one.
    Restart { installation_id: Uuid },
    /// Read live state, or explicitly marked history while offline.
    Status { installation_id: Option<Uuid> },
    /// Read retained run history without starting the daemon.
    History { installation_id: Uuid },
    /// Stream bounded runtime observations; never starts a daemon.
    Watch {
        #[arg(long)]
        installation_id: Option<Uuid>,
        /// JSON cursor from a previous status snapshot or event.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Read retained failure evidence without starting the daemon or an App.
    Failure {
        installation_id: Uuid,
        /// Observe a port-conflict condition for at most 60 seconds, without retrying.
        #[arg(long)]
        watch: bool,
        #[arg(long)]
        failure_id: Option<Uuid>,
        #[arg(long)]
        json: bool,
    },
    /// Pause automatic wake and stop all managed instances.
    StopAll,
    /// Re-enable automatic wake without starting any App.
    ResumeWake,
    /// Read operation facts without starting the daemon.
    Journal {
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Read a bounded observation window from the current instance.
    Logs {
        installation_id: Uuid,
        instance_id: Uuid,
        #[arg(long)]
        follow: bool,
        #[arg(long, requires = "follow")]
        after: Option<u64>,
        #[arg(long, default_value_t = 0)]
        from_ms: u128,
        #[arg(long,default_value_t=u64::MAX as u128)]
        to_ms: u128,
        #[arg(long, default_value_t = 65536)]
        limit: usize,
    },
    /// Run or contact the local daemon explicitly.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Inspect or explicitly configure the shared runtime PATH; never runs a service.
    Environment {
        #[command(subcommand)]
        command: EnvironmentCommand,
    },
    /// Discover projects without changing them. Review JSON before importing.
    Scan {
        path: PathBuf,
        #[arg(long, default_value_t = 3)]
        max_depth: usize,
        /// Store new inferred declarations in AppHost, leaving project directories untouched.
        #[arg(long)]
        external: bool,
        #[arg(long, default_value_t = 10_000)]
        max_directories: usize,
        #[arg(long, default_value_t = 100_000)]
        max_entries: usize,
        #[arg(long, default_value_t = 67_108_864)]
        max_read_bytes: u64,
        /// Additional directory names to skip (repeatable; not glob patterns).
        #[arg(long)]
        exclude: Vec<String>,
        /// Create a draft file; refuses to overwrite any existing file.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Validate apphost.toml or a project directory without running commands.
    Validate {
        path: PathBuf,
        #[arg(long)]
        check_launch: bool,
        #[arg(long)]
        project_root: Option<PathBuf>,
    },
    /// Confirm inferred launch declarations and optional multi-service grouping.
    Review {
        draft: PathBuf,
        /// Also revisit candidates already selected by discovery.
        #[arg(long)]
        all: bool,
        /// Ask for an optional description for selected inferred apps whose description is empty.
        #[arg(long)]
        describe: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Explicitly associate a draft candidate with an existing logical project.
    Associate {
        draft: PathBuf,
        path: PathBuf,
        installation_id: Uuid,
        #[arg(long)]
        output: PathBuf,
    },
    /// Reconnect a missing source while preserving installation identity.
    Relink {
        installation_id: Uuid,
        path: PathBuf,
    },
    /// Set retention policy metadata; does not perform cleanup.
    Retention {
        installation_id: Uuid,
        #[arg(value_enum)]
        policy: apphost::registry::RetentionPolicy,
    },
    /// Register selected entries in a reviewed draft and create missing manifests.
    Import {
        draft: PathBuf,
        /// Confirm the selected launch declarations and creation of missing apphost.toml files.
        #[arg(long, required = true)]
        yes: bool,
    },
    /// Read, review or explicitly save an installation's declaration.
    Manifest {
        #[command(subcommand)]
        command: ManifestCommand,
    },
    /// List registrations and current source/manifest diagnostics.
    List {
        #[arg(long)]
        all: bool,
    },
    /// Stop managing an installation. Project files and manifest are retained.
    Detach {
        #[arg(required = true)]
        installation_ids: Vec<Uuid>,
    },
    /// Set this installation's human-readable label.
    Label {
        installation_id: Uuid,
        label: String,
    },
}

#[derive(Subcommand)]
enum ManifestCommand {
    ReviewSwitch {
        installation_id: Uuid,
        #[arg(value_enum)]
        target: apphost::registry::ManifestSource,
    },
    Switch {
        installation_id: Uuid,
        #[arg(value_enum)]
        target: apphost::registry::ManifestSource,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        base_sha256: String,
        #[arg(long)]
        target_sha256: Option<String>,
        #[arg(long)]
        confirm: String,
    },
    Read {
        installation_id: Uuid,
    },
    Review {
        installation_id: Uuid,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Save {
        installation_id: Uuid,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        base_sha256: String,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        confirm: String,
    },
}
fn manifest_buffer(path: Option<PathBuf>) -> Result<Option<String>> {
    path.map(|p| apphost::registry::editing::read_buffer(&p))
        .transpose()
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Serve local RPCs in the foreground. Does not start any App.
    Serve,
    /// Read-only handshake; never starts a daemon.
    Ping,
    /// Check a confirmed installation via the daemon, without executing it.
    CheckLaunch { installation_id: Uuid },
}

#[derive(Subcommand)]
enum EnvironmentCommand {
    /// Explicitly run an interactive login shell to capture PATH. This executes shell startup scripts.
    Capture {
        #[arg(long)]
        shell: PathBuf,
        #[arg(long, required = true)]
        yes: bool,
    },
    /// Show the saved PATH or the system default without creating files.
    Show,
    /// Save an absolute PATH list for future runtime launches.
    SetPath { path: String },
    /// Resolve a manifest's commands using the shared runtime environment, without executing them.
    Check {
        #[arg(
            required_unless_present = "installation",
            conflicts_with = "installation"
        )]
        path: Option<PathBuf>,
        /// Check registration and confirmation as well as the runtime environment.
        #[arg(long, conflicts_with = "path")]
        installation: Option<Uuid>,
    },
}

#[derive(Subcommand)]
enum GatewayCommand {
    Show,
    SetPort { port: u16 },
}

fn data_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit.or_else(|| env::var_os("APPHOST_DATA_DIR").map(PathBuf::from)) {
        if path.as_os_str().is_empty() {
            return Err(Error::new("INVALID_INPUT", "data-dir must not be empty"));
        }
        return Ok(path);
    }
    if cfg!(target_os = "windows") {
        return env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("AppHost"))
            .ok_or_else(|| Error::new("INVALID_INPUT", "Set --data-dir or LOCALAPPDATA"));
    }
    if !cfg!(target_os = "macos")
        && let Some(path) = env::var_os("XDG_DATA_HOME")
    {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Ok(path.join("apphost"));
        }
    }
    let home =
        env::var_os("HOME").ok_or_else(|| Error::new("INVALID_INPUT", "Set --data-dir or HOME"))?;
    Ok(PathBuf::from(home).join(if cfg!(target_os = "macos") {
        "Library/Application Support/AppHost"
    } else {
        ".local/share/apphost"
    }))
}

fn run(cli: Cli) -> Result<Option<Value>> {
    let action = match &cli.command {
        Command::Import { .. } => Some("import"),
        Command::Label { .. } => Some("label"),
        Command::Detach { .. } => Some("detach"),
        Command::Relink { .. } => Some("relink"),
        Command::Retention { .. } => Some("retention"),
        Command::Associate { .. } => Some("associate"),
        Command::Environment {
            command: EnvironmentCommand::SetPath { .. },
        } => Some("environment-set-path"),
        Command::Environment {
            command: EnvironmentCommand::Capture { .. },
        } => Some("environment-capture"),
        _ => None,
    };
    if let Some(action) = action {
        let installation = match &cli.command {
            Command::Label {
                installation_id, ..
            }
            | Command::Retention {
                installation_id, ..
            }
            | Command::Relink {
                installation_id, ..
            }
            | Command::Associate {
                installation_id, ..
            } => Some(*installation_id),
            Command::Detach { installation_ids } if installation_ids.len() == 1 => {
                Some(installation_ids[0])
            }
            _ => None,
        };
        let actor = apphost::journal::Actor::from_environment()?;
        let data = data_dir(cli.data_dir.clone())?;
        let result = run_command(cli);
        return match result {
            Ok(Some(value)) => {
                apphost::journal::record_result(&data, action, actor, installation, Ok(value))
                    .map(Some)
            }
            Err(error) => {
                apphost::journal::record_result(&data, action, actor, installation, Err(error))
                    .map(Some)
            }
            Ok(None) => Ok(None),
        };
    }
    run_command(cli)
}

fn run_command(cli: Cli) -> Result<Option<Value>> {
    // Read-only project commands don't need an available home or writable registry.
    let result = match cli.command {
        Command::Url { installation_id } => {
            apphost::gateway::url(&data_dir(cli.data_dir)?, installation_id)
        }
        Command::Gateway { command } => {
            let data = data_dir(cli.data_dir)?;
            match command {
                GatewayCommand::Show => Ok(json!(apphost::gateway::config(&data)?)),
                GatewayCommand::SetPort { port } => apphost::gateway::set_port(&data, port),
            }
        }
        Command::Watch {
            installation_id,
            cursor,
        } => {
            use std::io::Write;
            let cursor = cursor
                .map(|c| {
                    serde_json::from_str::<apphost::events::Cursor>(&c)
                        .map_err(|_| Error::new("INVALID_PARAMS", "Invalid event cursor JSON"))
                })
                .transpose()?;
            apphost::daemon::watch(
                &data_dir(cli.data_dir)?,
                json!({"installation_id":installation_id,"cursor":cursor}),
                |event| {
                    let mut out = std::io::stdout().lock();
                    serde_json::to_writer(&mut out, &event)
                        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
                    out.write_all(b"\n")
                        .and_then(|_| out.flush())
                        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
                    Ok(true)
                },
            )?;
            return Ok(None);
        }
        Command::History { installation_id } => {
            apphost::failure::read_history(&data_dir(cli.data_dir)?, installation_id)
                .map(|h| json!(h))
        }
        Command::InternalCollect { root, descriptors } => {
            #[cfg(target_os = "macos")]
            {
                apphost::runtime::collector::run(&root, &descriptors)?;
                return Ok(None);
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (root, descriptors);
                return Err(Error::new(
                    "UNSUPPORTED_PLATFORM",
                    "Collector requires macOS",
                ));
            }
        }
        Command::Start { installation_id } => apphost::daemon::call(
            &data_dir(cli.data_dir)?,
            "start",
            json!({"installation_id":installation_id}),
            true,
        ),
        Command::StopOwnerAndStart {
            installation_id,
            owner_installation_id,
            owner_instance_id,
        } => apphost::daemon::call(
            &data_dir(cli.data_dir)?,
            "stop-owner-and-start",
            json!({"installation_id":installation_id,"owner_installation_id":owner_installation_id,"owner_instance_id":owner_instance_id}),
            true,
        ),
        Command::Stop { installation_id } => apphost::daemon::call(
            &data_dir(cli.data_dir)?,
            "stop",
            json!({"installation_id":installation_id}),
            true,
        ),
        Command::Restart { installation_id } => apphost::daemon::call(
            &data_dir(cli.data_dir)?,
            "restart",
            json!({"installation_id":installation_id}),
            true,
        ),
        Command::Status { installation_id } => {
            apphost::daemon::status(&data_dir(cli.data_dir)?, installation_id)
        }
        Command::Failure {
            installation_id,
            failure_id,
            watch,
            json: _,
        } => {
            let data = data_dir(cli.data_dir)?;
            let evidence = apphost::failure::read(&data, installation_id, failure_id)?;
            if watch {
                let id = evidence["failure"]["failure_id"].as_str().ok_or_else(|| {
                    Error::new("CONDITION_UNAVAILABLE", "No retained failure to observe")
                })?;
                apphost::daemon::watch_condition(
                    &data,
                    json!({"installation_id":installation_id,"failure_id":id}),
                    |event| {
                        use std::io::Write;
                        let mut out = std::io::stdout().lock();
                        serde_json::to_writer(&mut out, &event)
                            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
                        out.write_all(b"\n")
                            .and_then(|_| out.flush())
                            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
                        Ok(true)
                    },
                )?;
                return Ok(None);
            }
            Ok(evidence)
        }
        Command::StopAll => {
            apphost::daemon::call(&data_dir(cli.data_dir)?, "stop-all", json!({}), true)
        }
        Command::ResumeWake => {
            apphost::daemon::call(&data_dir(cli.data_dir)?, "resume-wake", json!({}), true)
        }
        Command::Journal {
            cursor,
            limit,
            json: _,
        } => apphost::journal::read(&data_dir(cli.data_dir)?, cursor.as_deref(), limit),
        Command::Logs {
            installation_id,
            instance_id,
            follow,
            after,
            from_ms,
            to_ms,
            limit,
        } => {
            if follow {
                if from_ms != 0 || to_ms != u64::MAX as u128 {
                    return Err(Error::new(
                        "INVALID_PARAMS",
                        "Use --after rather than a time window with --follow",
                    ));
                }
                apphost::daemon::follow_logs(
                    &data_dir(cli.data_dir)?,
                    json!({"installation_id":installation_id,"instance_id":instance_id,"after":after,"limit":limit}),
                    |event| {
                        use std::io::Write;
                        let mut out = std::io::stdout().lock();
                        serde_json::to_writer(&mut out, &event)
                            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
                        out.write_all(b"\n")
                            .and_then(|_| out.flush())
                            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
                        Ok(true)
                    },
                )?;
                return Ok(None);
            } else {
                apphost::daemon::call(
                    &data_dir(cli.data_dir)?,
                    "logs",
                    json!({"installation_id":installation_id,"instance_id":instance_id,"from_ms":from_ms,"to_ms":to_ms,"limit":limit}),
                    false,
                )
            }
        }
        Command::Daemon { command } => {
            let directory = data_dir(cli.data_dir)?;
            match command {
                DaemonCommand::Serve => {
                    apphost::daemon::serve(&directory)?;
                    return Ok(None);
                }
                DaemonCommand::Ping => apphost::daemon::ping(&directory),
                DaemonCommand::CheckLaunch { installation_id } => {
                    apphost::daemon::check_launch(&directory, installation_id)
                }
            }
        }
        Command::Environment { command } => {
            let directory = data_dir(cli.data_dir)?;
            match command {
                EnvironmentCommand::Capture { shell, yes: _ } => Ok(json!(
                    apphost::environment::capture_path(&directory, &shell)?
                )),
                EnvironmentCommand::Show => Ok(json!(apphost::environment::load(&directory)?)),
                EnvironmentCommand::SetPath { path } => {
                    Ok(json!(apphost::environment::save_path(&directory, path)?))
                }
                EnvironmentCommand::Check {
                    installation: Some(id),
                    ..
                } => {
                    let plan = apphost::runtime::LaunchPlan::prepare(&directory, id)?;
                    Ok(json!(plan.summary()))
                }
                EnvironmentCommand::Check {
                    path,
                    installation: None,
                } => {
                    let path = path.ok_or_else(|| {
                        Error::new("INVALID_INPUT", "Expected a project path or --installation")
                    })?;
                    let manifest = apphost::validate(&path)?;
                    let root = if path.is_dir() {
                        path.as_path()
                    } else {
                        path.parent()
                            .filter(|p| !p.as_os_str().is_empty())
                            .unwrap_or(std::path::Path::new("."))
                    };
                    let baseline = apphost::environment::load(&directory)?;
                    Ok(
                        json!({"services": apphost::environment::prepare(&manifest, root, &baseline)?}),
                    )
                }
            }
        }
        Command::Scan {
            path,
            max_depth,
            external,
            max_directories,
            max_entries,
            max_read_bytes,
            exclude,
            output,
        } => {
            let registry_dir = data_dir(cli.data_dir).ok();
            let mut draft = apphost::discovery::scan_with_registry(
                &path,
                &apphost::discovery::ScanOptions {
                    max_depth,
                    max_directories,
                    max_entries,
                    max_read_bytes,
                    exclude,
                },
                registry_dir.as_deref(),
            )?;
            if external {
                apphost::registry::sources::select_external(
                    registry_dir.as_deref().ok_or_else(|| {
                        Error::new("INVALID_INPUT", "External drafts require a data directory")
                    })?,
                    &mut draft,
                )?;
            }
            if let Some(path) = output {
                apphost::write_draft(&path, &draft)?;
            }
            Ok(json!(draft))
        }
        Command::Validate {
            path,
            check_launch,
            project_root,
        } => {
            let manifest = match &project_root {
                Some(root) => apphost::registry::sources::validate_at(&path, root)?,
                None => apphost::validate(&path)?,
            };
            if check_launch {
                let root = if let Some(root) = &project_root {
                    root.as_path()
                } else if path.is_dir() {
                    path.as_path()
                } else {
                    path.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(std::path::Path::new("."))
                };
                Ok(json!({"warnings":manifest.launch_warnings(root), "manifest":manifest}))
            } else {
                Ok(json!(manifest))
            }
        }
        Command::Review {
            draft,
            output,
            all,
            describe,
        } => {
            let mut draft = apphost::read_draft(&draft)?;
            apphost::registry::annotate_launch_changes(&data_dir(cli.data_dir)?, &mut draft)?;
            if all {
                for candidate in &mut draft.candidates {
                    candidate.selected = false;
                }
            }
            apphost::review::review_with_descriptions(
                &mut draft,
                &mut std::io::stdin().lock(),
                &mut std::io::stderr().lock(),
                describe,
            )?;
            let selected = draft.candidates.iter().filter(|c| c.selected).count();
            if let Some(output) = output {
                apphost::write_draft(&output, &draft)?;
                eprintln!(
                    "已选 {selected}/{} 项；草稿已保存至 {}",
                    draft.candidates.len(),
                    output.display()
                );
            } else {
                eprintln!(
                    "已选 {selected}/{} 项；未保存草稿，可用 --output 指定保存位置",
                    draft.candidates.len()
                );
            }
            return Ok(None);
        }
        Command::Associate {
            draft,
            path,
            installation_id,
            output,
        } => {
            let mut draft = apphost::read_draft(&draft)?;
            apphost::associate(&data_dir(cli.data_dir)?, &mut draft, &path, installation_id)?;
            apphost::write_draft(&output, &draft)?;
            Ok(json!(draft))
        }
        Command::Relink {
            installation_id,
            path,
        } => {
            let data = data_dir(cli.data_dir)?;
            let path = std::fs::canonicalize(&path)
                .map_err(|e| Error::new("SOURCE_MISSING", e.to_string()))?;
            match apphost::daemon::coordinate(
                &data,
                installation_id,
                "relink",
                json!({"installation_id":installation_id,"path":path}),
            )? {
                Some(value) => Ok(value),
                None => Ok(json!(apphost::relink(&data, installation_id, &path)?)),
            }
        }
        Command::Retention {
            installation_id,
            policy,
        } => Ok(json!(apphost::retention(
            &data_dir(cli.data_dir)?,
            installation_id,
            policy
        )?)),
        Command::Import { draft, yes: _ } => Ok(json!(apphost::registry::import_as(
            &data_dir(cli.data_dir)?,
            &apphost::read_draft(&draft)?,
            apphost::journal::Actor::from_environment()?
        )?)),
        Command::Manifest { command } => {
            let data = data_dir(cli.data_dir)?;
            match command {
                ManifestCommand::ReviewSwitch {
                    installation_id,
                    target,
                } => apphost::registry::sources::review_switch(&data, installation_id, target),
                ManifestCommand::Switch {
                    installation_id,
                    target,
                    revision,
                    base_sha256,
                    target_sha256,
                    confirm,
                } => apphost::registry::sources::switch_coordinated(
                    &data,
                    installation_id,
                    apphost::registry::sources::SwitchRequest {
                        revision,
                        base_sha256,
                        target_source: target,
                        target_sha256,
                        confirmed_launch_sha256: confirm,
                    },
                ),
                ManifestCommand::Read { installation_id } => {
                    apphost::registry::editing::read(&data, installation_id)
                }
                ManifestCommand::Review {
                    installation_id,
                    file,
                } => {
                    let text = manifest_buffer(file)?;
                    apphost::registry::editing::review(&data, installation_id, text.as_deref())
                }
                ManifestCommand::Save {
                    installation_id,
                    file,
                    base_sha256,
                    revision,
                    confirm,
                } => {
                    let text = manifest_buffer(file)?;
                    apphost::registry::editing::save_coordinated(
                        &data,
                        installation_id,
                        apphost::registry::editing::SaveRequest {
                            text,
                            base_sha256,
                            revision,
                            confirmed_launch_sha256: confirm,
                        },
                    )
                }
            }
        }
        Command::List { all } => Ok(json!(apphost::list(&data_dir(cli.data_dir)?, all)?)),
        Command::Detach { installation_ids } => {
            let data = data_dir(cli.data_dir)?;
            let coordinated = installation_ids
                .iter()
                .any(|id| data.join("runtime").join(format!("{id}.json")).exists())
                || apphost::daemon::ping(&data).is_ok();
            if coordinated {
                apphost::registry::check_installation_ids(&data, &installation_ids)?;
                let mut results = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for id in installation_ids {
                    if !seen.insert(id) {
                        continue;
                    }
                    results.push(apphost::daemon::call(
                        &data,
                        "detach",
                        json!({"installation_id":id}),
                        true,
                    )?);
                }
                return Ok(Some(if results.len() == 1 {
                    results.pop().unwrap()
                } else {
                    json!(results)
                }));
            }
            let installations = apphost::detach_many(&data, &installation_ids)?;
            let receipts: Vec<_> = installations
                .iter()
                .map(|i| {
                    let mut value = json!(i);
                    value["retained_files"] = json!(i.retained_files_in(&data));
                    value
                })
                .collect();
            // Preserve the original single-ID JSON response shape.
            if receipts.len() == 1 {
                Ok(receipts.into_iter().next().unwrap())
            } else {
                Ok(json!(receipts))
            }
        }
        Command::Label {
            installation_id,
            label,
        } => Ok(json!(apphost::label(
            &data_dir(cli.data_dir)?,
            installation_id,
            &label
        )?)),
    };
    result.map(Some)
}

fn main() {
    match run(Cli::parse()) {
        Ok(Some(value)) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
        Ok(None) => {}
        Err(error) => {
            eprintln!("{}", json!({"error": error}));
            std::process::exit(1);
        }
    }
}
