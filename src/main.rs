use apphost::{Error, Result};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::{env, path::PathBuf};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    version,
    about = "AppHost M0: inspect manifests and register local apps; never runs project commands"
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
enum EnvironmentCommand {
    /// Show the saved PATH or the system default without creating files.
    Show,
    /// Save an absolute PATH list for future runtime launches.
    SetPath { path: String },
    /// Resolve a manifest's commands using the shared runtime environment, without executing them.
    Check { path: PathBuf },
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
    // Read-only project commands don't need an available home or writable registry.
    let result = match cli.command {
        Command::Environment { command } => {
            let directory = data_dir(cli.data_dir)?;
            match command {
                EnvironmentCommand::Show => Ok(json!(apphost::environment::load(&directory)?)),
                EnvironmentCommand::SetPath { path } => {
                    Ok(json!(apphost::environment::save_path(&directory, path)?))
                }
                EnvironmentCommand::Check { path } => {
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
            max_directories,
            max_entries,
            max_read_bytes,
            exclude,
            output,
        } => {
            let registry_dir = data_dir(cli.data_dir).ok();
            let draft = apphost::discovery::scan_with_registry(
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
            if let Some(path) = output {
                apphost::write_draft(&path, &draft)?;
            }
            Ok(json!(draft))
        }
        Command::Validate { path, check_launch } => {
            let manifest = apphost::validate(&path)?;
            if check_launch {
                let root = if path.is_dir() {
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
        } => Ok(json!(apphost::relink(
            &data_dir(cli.data_dir)?,
            installation_id,
            &path
        )?)),
        Command::Retention {
            installation_id,
            policy,
        } => Ok(json!(apphost::retention(
            &data_dir(cli.data_dir)?,
            installation_id,
            policy
        )?)),
        Command::Import { draft, yes: _ } => Ok(json!(apphost::registry::import_with_receipts(
            &data_dir(cli.data_dir)?,
            &apphost::read_draft(&draft)?
        )?)),
        Command::List { all } => Ok(json!(apphost::list(&data_dir(cli.data_dir)?, all)?)),
        Command::Detach { installation_ids } => {
            let installations = apphost::detach_many(&data_dir(cli.data_dir)?, &installation_ids)?;
            let receipts: Vec<_> = installations
                .iter()
                .map(|i| {
                    let mut value = json!(i);
                    value["retained_files"] = json!(i.retained_files());
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
