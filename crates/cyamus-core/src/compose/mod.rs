//! `cyamus compose` (a worktree's stack) and `cyamus compose-shared` (the
//! project's shared stack, `<config>/compose.yaml`).
//!
//! Services defined in the shared stack are shadowed in worktree stacks: a
//! generated override keeps them from starting and rewrites `depends_on`.
//! Each worktree gets its own network `cyamus-<project>-<label>`, to which the
//! shared containers are attached under their service names, so worktrees
//! reach shared services by name without ever seeing each other.

pub mod args;
pub mod docker;
mod env;
pub mod files;
pub mod model;
pub mod overlay;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use cyamus_registry::label_from_slug;

use crate::daemon::DaemonControl;
use crate::manifest::{Manifest, ManifestError};
use crate::naming::slugify;
use crate::paths::Dirs;
use crate::report::Reporter;
use crate::workspace::{Workspace, WorkspaceError};

use docker::{Docker, PROJECT_LABEL, WORKSPACE_LABEL};

/// Shared stack file inside the project config directory.
pub const SHARED_FILE: &str = "compose.yaml";

#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("branch {0:?} has no letters or digits to name its compose stack after")]
    NoLabel(String),
    #[error("no shared compose file at {0}")]
    NoSharedFile(PathBuf),
    #[error(
        "no compose file found in {0} or its parents (looked for compose.yaml, compose.yml, docker-compose.yml, docker-compose.yaml)"
    )]
    NoComposeFile(PathBuf),
    #[error("the shared stack failed to start; not starting the worktree stack")]
    SharedFailed,
    #[error("{0}")]
    Docker(String),
    #[error("cannot write {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<String> for ComposeError {
    fn from(message: String) -> Self {
        Self::Docker(message)
    }
}

/// What both commands need to know about where they run.
pub struct Context {
    pub workspace: Workspace,
    pub manifest: Option<Manifest>,
    pub label: Option<String>,
}

impl Context {
    pub fn resolve(
        cwd: &Path,
        dirs: &Dirs,
        reporter: &mut dyn Reporter,
    ) -> Result<Self, ComposeError> {
        let workspace = Workspace::resolve(cwd, dirs, reporter)?;
        let manifest_path = workspace.project.paths.manifest();
        let manifest = if manifest_path.is_file() {
            Some(Manifest::load(&manifest_path)?)
        } else {
            None
        };
        let label = label_from_slug(&slugify(&workspace.branch, '-'));
        Ok(Self {
            workspace,
            manifest,
            label,
        })
    }

    fn project(&self) -> &str {
        &self.workspace.project.name
    }

    fn shared_file(&self) -> PathBuf {
        self.workspace.project.paths.root.join(SHARED_FILE)
    }

    fn shared_docker(&self, daemon: &dyn DaemonControl) -> Docker {
        let (set, remove) = env::shared(self, daemon, std::env::vars().map(|(k, _)| k));
        Docker {
            set,
            remove,
            cwd: self.workspace.project.paths.root.clone(),
        }
    }
}

/// Exit code of a finished `docker compose` (128 + signal when killed).
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// `cyamus compose-shared [args…]`.
pub fn shared(
    cwd: &Path,
    args: &[String],
    dirs: &Dirs,
    daemon: &dyn DaemonControl,
    reporter: &mut dyn Reporter,
) -> Result<i32, ComposeError> {
    let ctx = Context::resolve(cwd, dirs, reporter)?;
    let file = ctx.shared_file();
    if !file.is_file() {
        return Err(ComposeError::NoSharedFile(file));
    }
    let docker = ctx.shared_docker(daemon);
    docker::check_version(docker.compose_version()?)?;
    let status = docker
        .command()
        .args(["compose", "-p", ctx.project(), "-f"])
        .arg(&file)
        .args(args)
        .status()
        .map_err(|e| format!("cannot run docker: {e}"))?;
    if let Err(e) = reconcile(&docker, ctx.project()) {
        reporter.warning(&format!(
            "cannot attach shared services to worktree networks: {e}"
        ));
    }
    Ok(exit_code(status))
}

/// Connects every running shared container to every worktree network of
/// the project (recreated containers lose runtime attachments).
fn reconcile(docker: &Docker, project: &str) -> Result<(), String> {
    let containers = docker.project_containers(project)?;
    for network in docker.labelled_networks(PROJECT_LABEL, project)? {
        docker.attach(&network, &containers)?;
    }
    Ok(())
}

/// Subcommands that start dependencies, and so need the shared stack up.
const STARTING: &[&str] = &["up", "run"];

/// `cyamus compose [args…]`.
pub fn worktree(
    cwd: &Path,
    args: &[String],
    dirs: &Dirs,
    daemon: &dyn DaemonControl,
    reporter: &mut dyn Reporter,
) -> Result<i32, ComposeError> {
    let ctx = Context::resolve(cwd, dirs, reporter)?;
    let label = ctx
        .label
        .clone()
        .ok_or_else(|| ComposeError::NoLabel(ctx.workspace.branch.clone()))?;
    let project = ctx.project().to_owned();
    let split = args::split(args);

    let inherited = |k: &str| std::env::var_os(k).is_some();
    let mut docker = Docker {
        set: env::worktree(&ctx, daemon, &inherited),
        remove: Vec::new(),
        cwd: cwd.to_owned(),
    };
    docker::check_version(docker.compose_version()?)?;
    let stack = std::env::var("COMPOSE_PROJECT_NAME")
        .ok()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{project}-{label}"));

    // The files compose would use, which cyamus must now name explicitly.
    let files: Vec<OsString> = if split.has_files {
        args::file_values(&split.globals)
            .into_iter()
            .map(OsString::from)
            .collect()
    } else if let Some(value) = std::env::var("COMPOSE_FILE").ok().filter(|v| !v.is_empty()) {
        docker.remove.push("COMPOSE_FILE".to_owned());
        let sep = std::env::var("COMPOSE_PATH_SEPARATOR").ok();
        files::from_compose_file(&value, sep.as_deref())
            .into_iter()
            .map(OsString::from)
            .collect()
    } else {
        files::discover(cwd)
            .ok_or_else(|| ComposeError::NoComposeFile(cwd.to_owned()))?
            .into_iter()
            .map(OsString::from)
            .collect()
    };

    let shared_file = ctx.shared_file();
    let shared_docker = shared_file.is_file().then(|| ctx.shared_docker(daemon));
    let network = shared_docker
        .as_ref()
        .map(|_| format!("cyamus-{project}-{label}"));

    // Shadowing: services of the shared stack replace same-named ones here.
    let worktree_model = model::parse(&docker.config_json(&stack, &files)?)?;
    let mut shared_defaults = Vec::new();
    let mut shadowed = BTreeSet::new();
    if let Some(shared) = &shared_docker {
        let shared_model =
            model::parse(&shared.config_json(&project, &[shared_file.clone().into()])?)?;
        shared_defaults = shared_model.default_services().map(str::to_owned).collect();
        shadowed = worktree_model
            .services
            .keys()
            .filter(|s| shared_model.services.contains_key(*s))
            .cloned()
            .collect();
    }
    let override_file =
        match overlay::worktree_override(&worktree_model, &shadowed, network.as_deref()) {
            Some(text) => Some(write_override(dirs, &project, &label, &text)?),
            None => None,
        };
    if !shadowed.is_empty() {
        let names: Vec<&str> = shadowed.iter().map(String::as_str).collect();
        reporter.notice(&format!("using shared: {}", names.join(", ")));
    }

    let subcommand = split.subcommand.as_deref().unwrap_or_default();
    if let (Some(shared), Some(network)) = (&shared_docker, &network) {
        docker.ensure_network(
            network,
            &[(PROJECT_LABEL, &project), (WORKSPACE_LABEL, &label)],
        )?;
        if STARTING.contains(&subcommand) {
            let running = shared.running_services(&project)?;
            if shared_defaults.iter().any(|s| !running.contains(s)) {
                reporter.notice(&format!("starting the shared stack for {project}"));
                let ok = shared
                    .command()
                    .args(["compose", "-p", &project, "-f"])
                    .arg(&shared_file)
                    .args(["up", "-d"])
                    .status()
                    .map_err(|e| format!("cannot run docker: {e}"))?
                    .success();
                if !ok {
                    return Err(ComposeError::SharedFailed);
                }
            }
        }
        if subcommand != "down" {
            // Before the worktree's containers exist, so `db` resolves at boot.
            reconcile(shared, &project)?;
        }
    }

    let mut cmd = docker.command();
    cmd.arg("compose");
    if !split.has_project {
        cmd.args(["-p", &stack]);
    }
    cmd.args(&split.globals);
    if !split.has_files {
        for f in &files {
            cmd.arg("-f").arg(f);
        }
    }
    if let Some(path) = &override_file {
        cmd.arg("-f").arg(path);
    }
    cmd.args(split.subcommand.iter()).args(&split.rest);
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run docker: {e}"))?;

    if subcommand == "down"
        && status.success()
        && let Some(network) = &network
        && let Err(e) = docker.remove_network(network)
    {
        reporter.warning(&format!("cannot remove network {network}: {e}"));
    }
    Ok(exit_code(status))
}

fn write_override(
    dirs: &Dirs,
    project: &str,
    label: &str,
    text: &str,
) -> Result<PathBuf, ComposeError> {
    let dir = dirs.state_dir().join("compose").join(project);
    let path = dir.join(format!("{label}.yaml"));
    fs::create_dir_all(&dir)
        .and_then(|()| fs::write(&path, text))
        .map_err(|source| ComposeError::Io {
            path: path.clone(),
            source,
        })?;
    Ok(path)
}
