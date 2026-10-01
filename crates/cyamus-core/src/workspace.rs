//! A workspace: one worktree with its project resolved.

use std::path::{Path, PathBuf};

use crate::git::{Git, GitError};
use crate::paths::Dirs;
use crate::project::{Project, ProjectError};
use crate::report::Reporter;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{0} does not exist")]
    MissingPath(PathBuf),
    #[error("{0} is not inside a git worktree")]
    NotAWorktree(PathBuf),
    #[error("{0} has a detached HEAD; cyamus requires a checked-out branch")]
    DetachedHead(PathBuf),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Project(#[from] ProjectError),
}

#[derive(Debug, Clone)]
pub struct Workspace {
    /// Worktree root.
    pub root: PathBuf,
    pub branch: String,
    pub project: Project,
}

impl Workspace {
    /// Resolves the workspace containing `path` (the worktree root or any
    /// directory inside it).
    pub fn resolve(
        path: &Path,
        dirs: &Dirs,
        reporter: &mut dyn Reporter,
    ) -> Result<Self, WorkspaceError> {
        if !path.exists() {
            return Err(WorkspaceError::MissingPath(path.to_owned()));
        }
        let git = Git::new(path);
        let root = git
            .toplevel()?
            .ok_or_else(|| WorkspaceError::NotAWorktree(path.to_owned()))?;
        let branch = git
            .current_branch()?
            .ok_or_else(|| WorkspaceError::DetachedHead(root.clone()))?;
        let project = Project::locate(&root, dirs, reporter)?;
        Ok(Self {
            root,
            branch,
            project,
        })
    }

    pub fn git(&self) -> Git {
        Git::new(&self.root)
    }
}
