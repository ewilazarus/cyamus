//! Project identification and config directory management.

use std::fs;
use std::path::{Path, PathBuf};

use crate::git::{Git, GitError};
use crate::naming;
use crate::paths::{Dirs, ProjectPaths};
use crate::report::Reporter;

/// Git config key holding the project name.
pub const PROJECT_KEY: &str = "cyamus.project";

const SCAFFOLD_MANIFEST: &str = "version = 1\n";

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("{0} does not exist")]
    MissingPath(PathBuf),
    #[error("{0} is not inside a git repository")]
    NotARepository(PathBuf),
    #[error("`{PROJECT_KEY}` is set to an invalid value: {0}")]
    InvalidName(String),
    #[error(
        "cannot guess a project name for this repository; set one with `git config {PROJECT_KEY} <name>`"
    )]
    CannotGuess,
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("failed to scaffold {path}")]
    Scaffold {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// A repository resolved to its cyamus project.
#[derive(Debug, Clone)]
pub struct Project {
    pub name: String,
    pub common_dir: PathBuf,
    pub paths: ProjectPaths,
    pub cache_dir: PathBuf,
}

impl Project {
    /// Resolves the project of the repository containing `path`.
    ///
    /// If `cyamus.project` is unset, a name is guessed from the `origin`
    /// remote (or the repository directory), persisted to git config and
    /// reported.
    pub fn locate(
        path: &Path,
        dirs: &Dirs,
        reporter: &mut dyn Reporter,
    ) -> Result<Self, ProjectError> {
        if !path.exists() {
            return Err(ProjectError::MissingPath(path.to_owned()));
        }
        let git = Git::new(path);
        let common_dir = git
            .common_dir()?
            .ok_or_else(|| ProjectError::NotARepository(path.to_owned()))?;

        let name = match git.config_get(PROJECT_KEY)? {
            Some(name) => {
                naming::validate_project_name(&name).map_err(ProjectError::InvalidName)?;
                name
            }
            None => {
                let name = guess_name(git.origin_url()?.as_deref(), &common_dir)
                    .ok_or(ProjectError::CannotGuess)?;
                if let Err(err) = git.config_set_local(PROJECT_KEY, &name) {
                    // A parallel setup (Orca creating several worktrees at
                    // once) may hold git's config lock; accept its result.
                    return match git.config_get(PROJECT_KEY)? {
                        Some(_) => Self::locate(path, dirs, reporter),
                        None => Err(err.into()),
                    };
                }
                reporter.notice(&format!(
                    "guessed project name {name:?} and saved it as `{PROJECT_KEY}` in git config"
                ));
                name
            }
        };

        Ok(Self {
            paths: ProjectPaths::new(dirs.project_config_dir(&name)),
            cache_dir: dirs.project_cache_dir(&name),
            name,
            common_dir,
        })
    }

    /// Whether the project's config directory exists.
    pub fn is_configured(&self) -> bool {
        self.paths.root.is_dir()
    }

    /// Creates the config directory with an empty `assets/`, `bin/` and a
    /// minimal `cyamus.toml`, leaving anything that already exists untouched.
    pub fn scaffold(&self) -> Result<(), ProjectError> {
        let err = |path: &Path| {
            let path = path.to_owned();
            move |source| ProjectError::Scaffold { path, source }
        };
        for dir in [self.paths.assets(), self.paths.bin()] {
            fs::create_dir_all(&dir).map_err(err(&dir))?;
        }
        let manifest = self.paths.manifest();
        if !manifest.exists() {
            fs::write(&manifest, SCAFFOLD_MANIFEST).map_err(err(&manifest))?;
        }
        Ok(())
    }
}

/// Guesses a project name from the origin URL's last path segment, falling
/// back to the repository directory name.
fn guess_name(origin_url: Option<&str>, common_dir: &Path) -> Option<String> {
    let from_origin = origin_url.and_then(|url| {
        url.trim_end_matches('/')
            .rsplit(['/', ':'])
            .find(|s| !s.is_empty())
            .map(strip_git_suffix)
    });
    let from_dir = || {
        let name = common_dir.file_name()?.to_str()?;
        if name == ".git" {
            common_dir
                .parent()?
                .file_name()?
                .to_str()
                .map(str::to_owned)
        } else {
            Some(strip_git_suffix(name))
        }
    };
    from_origin
        .and_then(|n| naming::project_name_from(&n))
        .or_else(|| from_dir().and_then(|n| naming::project_name_from(&n)))
}

fn strip_git_suffix(name: &str) -> String {
    name.strip_suffix(".git").unwrap_or(name).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_from_origin() {
        let dir = Path::new("/code/whatever/.git");
        assert_eq!(
            guess_name(Some("git@github.com:acme/My_Proj.git"), dir).as_deref(),
            Some("my-proj")
        );
        assert_eq!(
            guess_name(Some("https://github.com/acme/widgets.git/"), dir).as_deref(),
            Some("widgets")
        );
        assert_eq!(
            guess_name(Some("/srv/repos/thing"), dir).as_deref(),
            Some("thing")
        );
    }

    #[test]
    fn guesses_from_common_dir() {
        assert_eq!(
            guess_name(None, Path::new("/code/widgets.git")).as_deref(),
            Some("widgets")
        );
        assert_eq!(
            guess_name(None, Path::new("/code/Gadgets/.git")).as_deref(),
            Some("gadgets")
        );
    }
}
