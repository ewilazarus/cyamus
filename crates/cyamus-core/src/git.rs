//! Thin wrapper around the `git` CLI.
//!
//! Every query runs `git -C <dir> ...` so that git itself decides worktree,
//! common-dir and config precedence semantics.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("failed to run git (is it installed and on PATH?)")]
    Spawn(#[source] std::io::Error),
    #[error("`git {args}` failed: {stderr}")]
    Command { args: String, stderr: String },
}

type Result<T> = std::result::Result<T, GitError>;

/// Runs git commands against a directory.
#[derive(Debug, Clone)]
pub struct Git {
    dir: PathBuf,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn output<I, S>(&self, args: I) -> Result<(Output, String)>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<_> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        let rendered = args
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let output = Command::new("git")
            // Paths cyamus passes are file names, never glob pathspecs.
            .env("GIT_LITERAL_PATHSPECS", "1")
            .arg("-C")
            .arg(&self.dir)
            .args(&args)
            .output()
            .map_err(GitError::Spawn)?;
        Ok((output, rendered))
    }

    /// Runs git and returns trimmed stdout, failing on a non-zero exit.
    fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let (output, rendered) = self.output(args)?;
        if !output.status.success() {
            return Err(GitError::Command {
                args: rendered,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned())
    }

    /// Runs git and returns trimmed stdout, or `None` on a non-zero exit.
    fn try_run<I, S>(&self, args: I) -> Result<Option<String>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let (output, _) = self.output(args)?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_owned(),
        ))
    }

    /// Root of the worktree containing `dir`, or `None` if `dir` is not inside one.
    pub fn toplevel(&self) -> Result<Option<PathBuf>> {
        Ok(self
            .try_run(["rev-parse", "--show-toplevel"])?
            .filter(|s| !s.is_empty())
            .map(PathBuf::from))
    }

    /// Absolute path of the git common dir (shared by all worktrees), or `None`
    /// if `dir` is not inside a repository.
    pub fn common_dir(&self) -> Result<Option<PathBuf>> {
        Ok(self
            .try_run(["rev-parse", "--path-format=absolute", "--git-common-dir"])?
            .filter(|s| !s.is_empty())
            .map(PathBuf::from))
    }

    /// Short name of the checked-out branch, or `None` when HEAD is detached.
    pub fn current_branch(&self) -> Result<Option<String>> {
        self.try_run(["symbolic-ref", "--quiet", "--short", "HEAD"])
    }

    /// Value of a config key, or `None` if unset.
    pub fn config_get(&self, key: &str) -> Result<Option<String>> {
        self.try_run(["config", "--get", key])
    }

    /// Writes a key to the repository config. Inside a linked worktree this
    /// lands in the common dir's config, shared by every worktree.
    pub fn config_set_local(&self, key: &str, value: &str) -> Result<()> {
        self.run(["config", "--local", key, value]).map(drop)
    }

    /// URL of the `origin` remote, or `None` if there is none.
    pub fn origin_url(&self) -> Result<Option<String>> {
        Ok(self
            .try_run(["remote", "get-url", "origin"])?
            .filter(|s| !s.is_empty()))
    }

    /// Tracked files at or below `path` (relative to the worktree root).
    pub fn tracked_files(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let out = self.run([OsStr::new("ls-files"), OsStr::new("--"), path.as_os_str()])?;
        Ok(out
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect())
    }

    /// Marks the given tracked files with the skip-worktree flag.
    pub fn skip_worktree(&self, files: &[PathBuf]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let mut args = vec![
            OsStr::new("update-index").to_owned(),
            OsStr::new("--skip-worktree").to_owned(),
            OsStr::new("--").to_owned(),
        ];
        args.extend(files.iter().map(|f| f.as_os_str().to_owned()));
        self.run(args).map(drop)
    }
}
