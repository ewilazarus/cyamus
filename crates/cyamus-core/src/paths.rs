//! XDG base directory resolution.
//!
//! cyamus always uses XDG locations, including on macOS (`~/.config`, not
//! `~/Library/Application Support`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("HOME is not set; cannot locate XDG directories")]
    NoHome,
}

/// Base directories cyamus reads from and writes to.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub config_home: PathBuf,
    pub cache_home: PathBuf,
    pub state_home: PathBuf,
}

impl Dirs {
    /// Resolves directories from the process environment.
    pub fn from_env() -> Result<Self, PathsError> {
        Self::from_lookup(|key| std::env::var_os(key))
    }

    /// Resolves directories from an arbitrary variable lookup.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Self, PathsError> {
        let home = lookup("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or(PathsError::NoHome)?;
        let xdg = |var: &str, fallback: &str| {
            lookup(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
        };
        Ok(Self {
            config_home: xdg("XDG_CONFIG_HOME", ".config"),
            cache_home: xdg("XDG_CACHE_HOME", ".cache"),
            state_home: xdg("XDG_STATE_HOME", ".local/state"),
        })
    }

    /// `$XDG_CONFIG_HOME/cyamus/projects/<project>/`
    pub fn project_config_dir(&self, project: &str) -> PathBuf {
        self.config_home
            .join("cyamus")
            .join("projects")
            .join(project)
    }

    /// `$XDG_CACHE_HOME/cyamus/projects/<project>/`
    pub fn project_cache_dir(&self, project: &str) -> PathBuf {
        self.cache_home
            .join("cyamus")
            .join("projects")
            .join(project)
    }

    /// `$XDG_STATE_HOME/cyamus/`: workspace registry and daemon runtime files.
    pub fn state_dir(&self) -> PathBuf {
        self.state_home.join("cyamus")
    }

    /// `$XDG_STATE_HOME/cyamus/workspaces/`
    pub fn registry_dir(&self) -> PathBuf {
        self.state_dir().join("workspaces")
    }

    /// `$XDG_STATE_HOME/cyamus/registry.lock`
    pub fn registry_lock(&self) -> PathBuf {
        self.state_dir().join("registry.lock")
    }

    /// `$XDG_STATE_HOME/cyamus/daemon.pid`
    pub fn daemon_pid(&self) -> PathBuf {
        self.state_dir().join("daemon.pid")
    }

    /// `$XDG_STATE_HOME/cyamus/daemon.lock`
    pub fn daemon_lock(&self) -> PathBuf {
        self.state_dir().join("daemon.lock")
    }

    /// `$XDG_STATE_HOME/cyamus/daemon.log`
    pub fn daemon_log(&self) -> PathBuf {
        self.state_dir().join("daemon.log")
    }
}

/// Files and directories inside a project config directory.
#[derive(Debug, Clone)]
pub struct ProjectPaths {
    pub root: PathBuf,
}

impl ProjectPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join("cyamus.toml")
    }

    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }
}

/// Returns `true` when `path` is relative and never climbs above its base
/// directory (lexically; symlinks are not followed).
pub fn is_contained(path: &Path) -> bool {
    use std::path::Component;
    if path.as_os_str().is_empty() {
        return false;
    }
    let mut depth: usize = 0;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return false,
            },
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    depth > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn dirs(vars: &[(&str, &str)]) -> Dirs {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        Dirs::from_lookup(|k| map.get(k).cloned()).unwrap()
    }

    #[test]
    fn falls_back_to_home_when_unset() {
        let d = dirs(&[("HOME", "/home/u")]);
        assert_eq!(d.config_home, PathBuf::from("/home/u/.config"));
        assert_eq!(d.cache_home, PathBuf::from("/home/u/.cache"));
        assert_eq!(d.state_home, PathBuf::from("/home/u/.local/state"));
    }

    #[test]
    fn state_paths() {
        let d = dirs(&[("HOME", "/home/u"), ("XDG_STATE_HOME", "/tmp/state")]);
        assert_eq!(
            d.registry_dir(),
            PathBuf::from("/tmp/state/cyamus/workspaces")
        );
        assert_eq!(
            d.registry_lock(),
            PathBuf::from("/tmp/state/cyamus/registry.lock")
        );
        assert_eq!(
            d.daemon_pid(),
            PathBuf::from("/tmp/state/cyamus/daemon.pid")
        );
        assert_eq!(
            d.daemon_lock(),
            PathBuf::from("/tmp/state/cyamus/daemon.lock")
        );
        assert_eq!(
            d.daemon_log(),
            PathBuf::from("/tmp/state/cyamus/daemon.log")
        );
    }

    #[test]
    fn uses_absolute_xdg_values() {
        let d = dirs(&[
            ("HOME", "/home/u"),
            ("XDG_CONFIG_HOME", "/tmp/cfg"),
            ("XDG_CACHE_HOME", "/tmp/cache"),
        ]);
        assert_eq!(
            d.project_config_dir("myproj"),
            PathBuf::from("/tmp/cfg/cyamus/projects/myproj")
        );
        assert_eq!(
            d.project_cache_dir("myproj"),
            PathBuf::from("/tmp/cache/cyamus/projects/myproj")
        );
    }

    #[test]
    fn ignores_relative_xdg_values() {
        let d = dirs(&[
            ("HOME", "/home/u"),
            ("XDG_CONFIG_HOME", "rel/cfg"),
            ("XDG_CACHE_HOME", ""),
            ("XDG_STATE_HOME", "state"),
        ]);
        assert_eq!(d.config_home, PathBuf::from("/home/u/.config"));
        assert_eq!(d.cache_home, PathBuf::from("/home/u/.cache"));
        assert_eq!(d.state_home, PathBuf::from("/home/u/.local/state"));
    }

    #[test]
    fn requires_home() {
        assert!(Dirs::from_lookup(|_| None).is_err());
    }

    #[test]
    fn containment() {
        assert!(is_contained(Path::new("a/b")));
        assert!(is_contained(Path::new("./a")));
        assert!(is_contained(Path::new("a/../b")));
        assert!(!is_contained(Path::new("../a")));
        assert!(!is_contained(Path::new("a/../../b")));
        assert!(!is_contained(Path::new("/etc/passwd")));
        assert!(!is_contained(Path::new("")));
        assert!(!is_contained(Path::new(".")));
        assert!(!is_contained(Path::new("a/..")));
    }
}
