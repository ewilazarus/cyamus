//! `workspace setup` and `workspace teardown`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cyamus_registry::{Record, Registration, Registry, RegistryError, label_from_slug};

use crate::assets::{self, AssetError};
use crate::daemon::DaemonControl;
use crate::env::{self, Routing};
use crate::fingerprint;
use crate::hooks::{Event, HookError, HookRunner};
use crate::manifest::{Manifest, ManifestError};
use crate::naming::slugify;
use crate::paths::Dirs;
use crate::report::Reporter;
use crate::workspace::{Workspace, WorkspaceError};

#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error(transparent)]
    Hook(#[from] HookError),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error("failed to create cache directory {path}")]
    CacheDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Applies assets, computes fingerprints, registers the workspace, makes sure
/// the routing daemon runs, and runs `on_setup` hooks.
pub fn setup(
    path: &Path,
    runtime_vars: &[(String, String)],
    dirs: &Dirs,
    daemon: &mut dyn DaemonControl,
    reporter: &mut dyn Reporter,
) -> Result<(), LifecycleError> {
    let Some((workspace, manifest)) = prepare(path, dirs, reporter)? else {
        return Ok(());
    };
    assets::apply(&workspace, &manifest)?;
    let label = branch_label(&workspace);
    let fingerprints = prepare_hooks(&workspace, &manifest)?;
    register(&workspace, label.as_deref(), dirs, reporter)?;
    if let Err(message) = daemon.ensure_running() {
        reporter.warning(&format!(
            "routing daemon is not running: {message} (see {})",
            dirs.daemon_log().display()
        ));
    }
    let env = hook_env(
        &workspace,
        &manifest,
        runtime_vars,
        &fingerprints,
        label.as_deref(),
        &*daemon,
    );
    run_hooks(Event::Setup, &workspace, &manifest, &env, reporter)
}

/// Computes fingerprints, runs `on_teardown` hooks, then unregisters the
/// workspace (even when a hook failed). Never deletes anything in the worktree.
pub fn teardown(
    path: &Path,
    runtime_vars: &[(String, String)],
    dirs: &Dirs,
    daemon: &dyn DaemonControl,
    reporter: &mut dyn Reporter,
) -> Result<(), LifecycleError> {
    let Some((workspace, manifest)) = prepare(path, dirs, reporter)? else {
        return Ok(());
    };
    let label = branch_label(&workspace);
    let fingerprints = prepare_hooks(&workspace, &manifest)?;
    let env = hook_env(
        &workspace,
        &manifest,
        runtime_vars,
        &fingerprints,
        label.as_deref(),
        daemon,
    );
    let hooks = run_hooks(Event::Teardown, &workspace, &manifest, &env, reporter);
    if let Some(label) = &label {
        registry(dirs).unregister(&workspace.project.name, label, &workspace.root)?;
    }
    hooks
}

fn branch_label(workspace: &Workspace) -> Option<String> {
    label_from_slug(&slugify(&workspace.branch, '-'))
}

fn registry(dirs: &Dirs) -> Registry {
    Registry::new(dirs.registry_dir(), dirs.registry_lock())
}

fn register(
    workspace: &Workspace,
    label: Option<&str>,
    dirs: &Dirs,
    reporter: &mut dyn Reporter,
) -> Result<(), LifecycleError> {
    let Some(label) = label else {
        reporter.warning(&format!(
            "branch {:?} has no letters or digits to use in a hostname; this workspace gets no routes",
            workspace.branch
        ));
        return Ok(());
    };
    let record = Record {
        project: workspace.project.name.clone(),
        branch: workspace.branch.clone(),
        label: label.to_owned(),
        path: workspace.root.clone(),
    };
    if let Registration::Conflict { existing } = registry(dirs).register(&record)? {
        reporter.warning(&format!(
            "hostname label {label:?} of project {:?} already belongs to {}; {} gets no routes",
            workspace.project.name,
            existing.display(),
            workspace.root.display()
        ));
    }
    Ok(())
}

/// Resolves the workspace and loads its manifest, or returns `None` (after a
/// notice) when the project has no config directory.
fn prepare(
    path: &Path,
    dirs: &Dirs,
    reporter: &mut dyn Reporter,
) -> Result<Option<(Workspace, Manifest)>, LifecycleError> {
    let workspace = Workspace::resolve(path, dirs, reporter)?;
    let project = &workspace.project;
    if !project.is_configured() {
        reporter.notice(&format!(
            "project {:?} is not configured (no {}); nothing to do. Run `cyamus edit` to create it.",
            project.name,
            project.paths.root.display()
        ));
        return Ok(None);
    }
    let manifest = Manifest::load(&project.paths.manifest())?;
    Ok(Some((workspace, manifest)))
}

/// Computes fingerprints and ensures the cache directory exists.
fn prepare_hooks(
    workspace: &Workspace,
    manifest: &Manifest,
) -> Result<Vec<(String, String)>, LifecycleError> {
    let fingerprints = fingerprint::compute(&workspace.root, &manifest.fingerprints);
    let cache_dir = &workspace.project.cache_dir;
    fs::create_dir_all(cache_dir).map_err(|source| LifecycleError::CacheDir {
        path: cache_dir.clone(),
        source,
    })?;
    Ok(fingerprints)
}

/// Builds the hook environment. Runs after the daemon step, because the URL
/// suffix depends on whether the running daemon is reachable on port 80.
fn hook_env(
    workspace: &Workspace,
    manifest: &Manifest,
    runtime_vars: &[(String, String)],
    fingerprints: &[(String, String)],
    label: Option<&str>,
    daemon: &dyn DaemonControl,
) -> BTreeMap<String, String> {
    let url_suffix = daemon.url_suffix();
    let routing = Routing {
        label,
        proxy_port: daemon.port(),
        url_suffix: &url_suffix,
    };
    env::build(
        workspace,
        &manifest.vars,
        runtime_vars,
        fingerprints,
        routing,
    )
}

fn run_hooks(
    event: Event,
    workspace: &Workspace,
    manifest: &Manifest,
    env: &BTreeMap<String, String>,
    reporter: &mut dyn Reporter,
) -> Result<(), LifecycleError> {
    let groups = match event {
        Event::Setup => &manifest.hooks.on_setup,
        Event::Teardown => &manifest.hooks.on_teardown,
    };
    let bin_dir = workspace.project.paths.bin();
    let runner = HookRunner {
        worktree: &workspace.root,
        bin_dir: &bin_dir,
        env,
    };
    runner.run(event, groups, &workspace.branch, reporter)?;
    Ok(())
}
