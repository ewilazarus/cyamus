//! Applying `[[link]]` and `[[copy]]` entries into a worktree.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::exclude;
use crate::git::GitError;
use crate::manifest::Manifest;
use crate::workspace::Workspace;

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("[[{kind}]] source {source_path:?} does not exist in {}", .assets.display())]
    MissingSource {
        kind: &'static str,
        source_path: String,
        assets: PathBuf,
    },
    #[error(
        "[[link]] target {target:?} already exists and is not a link to {}; remove it or set `override = true` if it is tracked",
        .source_path.display()
    )]
    LinkConflict {
        target: String,
        source_path: PathBuf,
    },
    #[error("[[{kind}]] target {target:?} is not tracked by git; remove `override = true`")]
    OverrideUntracked { kind: &'static str, target: String },
    #[error("[[{kind}]] target {target:?} is tracked by git; set `override = true` to replace it")]
    TargetTracked { kind: &'static str, target: String },
    #[error("failed to apply [[{kind}]] target {target:?}")]
    Io {
        kind: &'static str,
        target: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to update the git exclude file")]
    Exclude(#[source] io::Error),
    #[error(transparent)]
    Git(#[from] GitError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Link,
    Copy { overwrite: bool },
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Link => "link",
            Kind::Copy { .. } => "copy",
        }
    }
}

struct Entry<'a> {
    kind: Kind,
    source: &'a str,
    target: &'a str,
    override_tracked: bool,
}

/// Applies all assets of `manifest` to `workspace` and syncs the exclude block.
///
/// Every source and override precondition is checked before anything in the
/// worktree is modified.
pub fn apply(workspace: &Workspace, manifest: &Manifest) -> Result<(), AssetError> {
    let assets_dir = workspace.project.paths.assets();
    let git = workspace.git();
    let entries: Vec<Entry> = manifest
        .copies
        .iter()
        .map(|c| Entry {
            kind: Kind::Copy {
                overwrite: c.overwrite,
            },
            source: &c.source,
            target: &c.target,
            override_tracked: c.override_tracked,
        })
        .chain(manifest.links.iter().map(|l| Entry {
            kind: Kind::Link,
            source: &l.source,
            target: &l.target,
            override_tracked: l.override_tracked,
        }))
        .collect();

    // Preconditions: no worktree changes until all of these hold.
    let mut tracked = Vec::with_capacity(entries.len());
    for entry in &entries {
        let source = assets_dir.join(entry.source);
        if fs::symlink_metadata(&source).is_err() {
            return Err(AssetError::MissingSource {
                kind: entry.kind.name(),
                source_path: entry.source.to_owned(),
                assets: assets_dir.clone(),
            });
        }
        let files = git.tracked_files(Path::new(entry.target))?;
        match (entry.override_tracked, files.is_empty()) {
            (true, true) => {
                return Err(AssetError::OverrideUntracked {
                    kind: entry.kind.name(),
                    target: entry.target.to_owned(),
                });
            }
            (false, false) => {
                return Err(AssetError::TargetTracked {
                    kind: entry.kind.name(),
                    target: entry.target.to_owned(),
                });
            }
            _ => {}
        }
        tracked.push(files);
    }

    let mut excluded = Vec::new();
    for (entry, tracked_files) in entries.iter().zip(&tracked) {
        let source = assets_dir.join(entry.source);
        let target = workspace.root.join(entry.target);
        let io_err = |source| AssetError::Io {
            kind: entry.kind.name(),
            target: entry.target.to_owned(),
            source,
        };
        if entry.override_tracked {
            git.skip_worktree(tracked_files)?;
        } else {
            excluded.push(entry.target.to_owned());
        }
        match entry.kind {
            Kind::Link => link(&source, &target, entry, io_err)?,
            Kind::Copy { overwrite } => {
                let exists = fs::symlink_metadata(&target).is_ok();
                if exists && !overwrite {
                    continue;
                }
                if exists {
                    remove(&target).map_err(io_err)?;
                }
                copy(&source, &target).map_err(io_err)?;
            }
        }
    }

    exclude::sync(&workspace.project.common_dir, &excluded).map_err(AssetError::Exclude)
}

fn link(
    source: &Path,
    target: &Path,
    entry: &Entry,
    io_err: impl Fn(io::Error) -> AssetError,
) -> Result<(), AssetError> {
    if let Ok(meta) = fs::symlink_metadata(target) {
        let points_to_source =
            meta.file_type().is_symlink() && fs::read_link(target).is_ok_and(|t| t == source);
        if points_to_source {
            return Ok(());
        }
        if !entry.override_tracked {
            return Err(AssetError::LinkConflict {
                target: entry.target.to_owned(),
                source_path: source.to_owned(),
            });
        }
        remove(target).map_err(&io_err)?;
    }
    create_parent(target).map_err(&io_err)?;
    std::os::unix::fs::symlink(source, target).map_err(io_err)
}

fn copy(source: &Path, target: &Path) -> io::Result<()> {
    create_parent(target)?;
    if fs::metadata(source)?.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &target.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(source, target).map(drop)
    }
}

fn remove(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn create_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) => fs::create_dir_all(parent),
        None => Ok(()),
    }
}
