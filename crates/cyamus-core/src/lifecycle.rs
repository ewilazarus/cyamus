//! `workspace setup` and `workspace teardown`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::assets::{self, AssetError};
use crate::env;
use crate::fingerprint;
use crate::hooks::{Event, HookError, HookRunner};
use crate::manifest::{Manifest, ManifestError};
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
    #[error("failed to create cache directory {path}")]
    CacheDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Applies assets, computes fingerprints and runs `on_setup` hooks.
pub fn setup(
    path: &Path,
    runtime_vars: &[(String, String)],
    dirs: &Dirs,
    reporter: &mut dyn Reporter,
) -> Result<(), LifecycleError> {
    let Some((workspace, manifest)) = prepare(path, dirs, reporter)? else {
        return Ok(());
    };
    assets::apply(&workspace, &manifest)?;
    run_hooks(Event::Setup, &workspace, &manifest, runtime_vars, reporter)
}

/// Computes fingerprints and runs `on_teardown` hooks. Never deletes anything.
pub fn teardown(
    path: &Path,
    runtime_vars: &[(String, String)],
    dirs: &Dirs,
    reporter: &mut dyn Reporter,
) -> Result<(), LifecycleError> {
    let Some((workspace, manifest)) = prepare(path, dirs, reporter)? else {
        return Ok(());
    };
    run_hooks(
        Event::Teardown,
        &workspace,
        &manifest,
        runtime_vars,
        reporter,
    )
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

fn run_hooks(
    event: Event,
    workspace: &Workspace,
    manifest: &Manifest,
    runtime_vars: &[(String, String)],
    reporter: &mut dyn Reporter,
) -> Result<(), LifecycleError> {
    let fingerprints = fingerprint::compute(&workspace.root, &manifest.fingerprints);
    let cache_dir = &workspace.project.cache_dir;
    fs::create_dir_all(cache_dir).map_err(|source| LifecycleError::CacheDir {
        path: cache_dir.clone(),
        source,
    })?;
    let env: BTreeMap<String, String> =
        env::build(workspace, &manifest.vars, runtime_vars, &fingerprints);
    let groups = match event {
        Event::Setup => &manifest.hooks.on_setup,
        Event::Teardown => &manifest.hooks.on_teardown,
    };
    let bin_dir = workspace.project.paths.bin();
    let runner = HookRunner {
        worktree: &workspace.root,
        bin_dir: &bin_dir,
        env: &env,
    };
    runner.run(event, groups, &workspace.branch, reporter)?;
    Ok(())
}
